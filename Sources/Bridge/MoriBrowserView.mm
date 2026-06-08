#import "MoriBrowserView.h"

#include <string>
#include <vector>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <functional>
#include <limits>

#include "BrowserClient.h"
#include "include/cef_app.h"
#include "include/cef_browser.h"
#include "include/cef_devtools_message_observer.h"
#include "include/wrapper/cef_helpers.h"
#include "../Shared/MoriSchemes.h"

// ---------------------------------------------------------------------------
// Why this view hosts Chromium as an embedded child view
//
// Mori is the browser UI; Chromium is the page engine underneath it. On macOS
// a CEF browser created with parent_view/SetAsChild uses the Alloy runtime, so
// it embeds cleanly in our NSView hierarchy instead of launching Chrome's own
// top-level window. Chrome's built-in extension runtime is therefore not
// available on this surface; extension behavior must be implemented by Mori
// itself (see BrowserClient's content-script injection path).
// ---------------------------------------------------------------------------

namespace {
NSString* SafeString(const std::string& s) {
  NSString* out = [NSString stringWithUTF8String:s.c_str()];
  return out ?: @"";
}

NSString* JSONLiteral(id object) {
  id safe = object ?: [NSNull null];
  if (![NSJSONSerialization isValidJSONObject:@[ safe ]]) {
    safe = [NSNull null];
  }
  NSData* data = [NSJSONSerialization dataWithJSONObject:@[ safe ]
                                                 options:0
                                                   error:nil];
  NSString* array =
      data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding]
           : @"[null]";
  return [array substringWithRange:NSMakeRange(1, array.length - 2)];
}

uint64_t MoriStableHash64(const std::string& value) {
  uint64_t hash = 1469598103934665603ULL;
  for (unsigned char c : value) {
    hash ^= c;
    hash *= 1099511628211ULL;
  }
  return hash;
}

int MoriExtensionFrameID(CefRefPtr<CefFrame> frame) {
  if (!frame || frame->IsMain()) return 0;
  std::string identifier = frame->GetIdentifier().ToString();
  std::size_t raw = std::hash<std::string>{}(identifier);
  int value = static_cast<int>(raw % std::numeric_limits<int>::max());
  return value > 0 ? value : 1;
}

int MoriExtensionParentFrameID(CefRefPtr<CefFrame> frame) {
  if (!frame || frame->IsMain()) return -1;
  CefRefPtr<CefFrame> parent = frame->GetParent();
  return parent ? MoriExtensionFrameID(parent) : 0;
}

NSString* MoriExtensionDocumentID(CefRefPtr<CefFrame> frame) {
  if (!frame) return @"";
  std::string seed = frame->GetIdentifier().ToString() + "\n" +
                     frame->GetURL().ToString();
  uint64_t hi = MoriStableHash64(seed);
  uint64_t lo = MoriStableHash64("document:" + seed);
  return [NSString stringWithFormat:@"%08llx-%04llx-%04llx-%04llx-%012llx",
                                    (unsigned long long)((hi >> 32) & 0xffffffffULL),
                                    (unsigned long long)((hi >> 16) & 0xffffULL),
                                    (unsigned long long)(hi & 0xffffULL),
                                    (unsigned long long)((lo >> 48) & 0xffffULL),
                                    (unsigned long long)(lo & 0xffffffffffffULL)];
}

NSDictionary* MoriExtensionFrameRecord(CefRefPtr<CefFrame> frame,
                                       NSInteger tabID) {
  NSMutableDictionary* record = [@{
    @"errorOccurred" : @NO,
    @"tabId" : @(tabID),
    @"frameId" : @(MoriExtensionFrameID(frame)),
    @"parentFrameId" : @(MoriExtensionParentFrameID(frame)),
    @"documentId" : MoriExtensionDocumentID(frame),
    @"documentLifecycle" : @"active",
    @"frameType" : (frame && frame->IsMain()) ? @"outermost_frame" : @"sub_frame",
    @"url" : SafeString(frame ? frame->GetURL().ToString() : "")
  } mutableCopy];
  if (frame && !frame->IsMain()) {
    CefRefPtr<CefFrame> parent = frame->GetParent();
    if (parent) {
      record[@"parentDocumentId"] = MoriExtensionDocumentID(parent);
    }
  }
  return record;
}

// One press changes the CEF zoom level by this much. CEF zoom is logarithmic
// (scale = 1.2^level), so 0.5 is roughly a 10% step — close to Chrome's feel.
const double kZoomStep = 0.5;

// Matches Radius.window — the floating web-content card's corner radius.
const CGFloat kCardCornerRadius = 10.0;

// Suppresses every embedded web view at once (e.g. while a full-window SwiftUI
// overlay like the launcher is up). Toggled from the Swift layer.
bool g_web_content_suppressed = false;

NSView* ViewFromCEFHandle(void* handle) {
  if (!handle) return nil;
  id object = (__bridge id)handle;
  if ([object isKindOfClass:[NSView class]]) {
    return (NSView*)object;
  }
  return nil;
}

const char* RuntimeStyleName(cef_runtime_style_t style) {
  switch (style) {
    case CEF_RUNTIME_STYLE_ALLOY:
      return "alloy";
    case CEF_RUNTIME_STYLE_DEFAULT:
      return "default";
    default:
      return "other";
  }
}

void EmitEngineAuditMarker(cef_runtime_style_t runtime_style) {
  const char* style = RuntimeStyleName(runtime_style);
  const char* scheme = mori::kExtensionScheme;
  fprintf(stderr,
          "__MORI_CHROMIUM_ENGINE__ runtime=%s embedding=child-view scheme=%s\n",
          style, scheme);

  const char* audit_path = getenv("MORI_CHROMIUM_ENGINE_AUDIT_PATH");
  if (!audit_path || audit_path[0] == '\0') return;
  FILE* audit = fopen(audit_path, "a");
  if (!audit) return;
  fprintf(audit,
          "__MORI_CHROMIUM_ENGINE__ runtime=%s embedding=child-view scheme=%s\n",
          style, scheme);
  fclose(audit);
}

}  // namespace

extern void MoriRegisterExtensionDownload(NSString* url,
                                            NSString* extensionID,
                                            NSString* requestID,
                                            NSString* filename);

// Private interface (declared up-front so the C++ delegate below can call it).
@interface MoriBrowserView ()
- (void)_attachBrowser:(CefRefPtr<CefBrowser>)browser;
- (void)_detachBrowser;
- (void)_applyTitle:(NSString*)title;
- (void)_applyURL:(NSString*)url;
- (void)_applyLoading:(BOOL)isLoading
            canGoBack:(BOOL)canGoBack
         canGoForward:(BOOL)canGoForward;
- (void)_applyFaviconURLs:(NSArray<NSString*>*)urls;
- (void)_applyFaviconImage:(nullable NSImage*)image;
- (void)_applyNavigationStart:(NSString*)url
                   isRedirect:(BOOL)isRedirect
                  userGesture:(BOOL)userGesture;
- (void)_applyNavigationCommit:(NSString*)url;
- (void)_applyNavigationFinish:(NSString*)url httpStatusCode:(NSInteger)code;
- (void)_applyLoadError:(NSString*)errorText failedURL:(NSString*)failedURL;
- (void)_requestNewTab:(NSString*)url;
- (void)_applyFindOrdinal:(int)ordinal ofMatches:(int)count;
- (void)_syncBrowserFrame;
- (void)_syncBrowserVisibility;
@end

// C++ delegate that forwards CEF callbacks to the owning ObjC view.
// Holds a __weak reference so a destroyed view never causes a dangling call.
class ViewClientDelegate : public BrowserClientDelegate {
 public:
  explicit ViewClientDelegate(MoriBrowserView* view) : view_(view) {}

  void OnAfterCreated(CefRefPtr<CefBrowser> browser) override {
    MoriBrowserView* v = view_;
    if (!v) return;
    [v _attachBrowser:browser];
  }
  void OnBeforeClose(CefRefPtr<CefBrowser> browser) override {
    MoriBrowserView* v = view_;
    if (!v) return;
    [v _detachBrowser];
  }

  void OnTitleChange(const std::string& title) override {
    MoriBrowserView* v = view_;
    if (!v) return;
    [v _applyTitle:SafeString(title)];
  }
  void OnAddressChange(const std::string& url) override {
    MoriBrowserView* v = view_;
    if (!v) return;
    [v _applyURL:SafeString(url)];
  }
  void OnLoadingStateChange(bool isLoading,
                            bool canGoBack,
                            bool canGoForward) override {
    MoriBrowserView* v = view_;
    if (!v) return;
    [v _applyLoading:isLoading canGoBack:canGoBack canGoForward:canGoForward];
  }
  void OnFaviconURLChange(const std::vector<std::string>& icon_urls) override {
    MoriBrowserView* v = view_;
    if (!v) return;
    NSMutableArray<NSString*>* urls =
        [NSMutableArray arrayWithCapacity:icon_urls.size()];
    for (const auto& u : icon_urls) {
      [urls addObject:SafeString(u)];
    }
    [v _applyFaviconURLs:urls];
  }
  void OnFaviconImage(const std::string& /*image_url*/,
                      const unsigned char* png_bytes,
                      size_t len) override {
    MoriBrowserView* v = view_;
    if (!v) return;
    NSImage* image = nil;
    if (png_bytes && len > 0) {
      NSData* data = [NSData dataWithBytes:png_bytes length:len];
      image = [[NSImage alloc] initWithData:data];
    }
    [v _applyFaviconImage:image];
  }
  void OnBeforeBrowse(const std::string& url,
                      bool is_redirect,
                      bool user_gesture) override {
    MoriBrowserView* v = view_;
    if (!v) return;
    [v _applyNavigationStart:SafeString(url)
                  isRedirect:is_redirect
                 userGesture:user_gesture];
  }
  void OnLoadStart(const std::string& url) override {
    MoriBrowserView* v = view_;
    if (!v) return;
    [v _applyNavigationCommit:SafeString(url)];
  }
  void OnLoadEnd(const std::string& url, int http_status_code) override {
    MoriBrowserView* v = view_;
    if (!v) return;
    [v _applyNavigationFinish:SafeString(url) httpStatusCode:http_status_code];
  }
  void OnLoadError(int errorCode,
                   const std::string& errorText,
                   const std::string& failedUrl) override {
    MoriBrowserView* v = view_;
    if (!v) return;
    [v _applyLoadError:SafeString(errorText) failedURL:SafeString(failedUrl)];
  }
  bool OnOpenURLFromTab(const std::string& target_url) override {
    MoriBrowserView* v = view_;
    if (!v) return false;
    [v _requestNewTab:SafeString(target_url)];
    return true;
  }
  void OnFindResult(int count, int activeMatchOrdinal) override {
    MoriBrowserView* v = view_;
    if (!v) return;
    [v _applyFindOrdinal:activeMatchOrdinal ofMatches:count];
  }

 private:
  __weak MoriBrowserView* view_;
};

class ScreenshotObserver : public CefDevToolsMessageObserver {
 public:
  ScreenshotObserver(NSString* extensionID, NSString* requestID)
      : extension_id_([extensionID copy]), request_id_([requestID copy]) {}

  void SetMessageID(int message_id) { message_id_ = message_id; }
  void SetRegistration(CefRefPtr<CefRegistration> registration) {
    registration_ = registration;
  }

  void OnDevToolsMethodResult(CefRefPtr<CefBrowser> browser,
                              int message_id,
                              bool success,
                              const void* result,
                              size_t result_size) override {
    if (message_id_ != 0 && message_id != message_id_) return;

    NSString* error = nil;
    NSString* dataURL = nil;
    NSData* jsonData = result_size > 0
        ? [NSData dataWithBytes:result length:result_size]
        : nil;
    id parsed = jsonData
        ? [NSJSONSerialization JSONObjectWithData:jsonData options:0 error:nil]
        : nil;
    if (success && [parsed isKindOfClass:NSDictionary.class]) {
      NSDictionary* dict = (NSDictionary*)parsed;
      NSString* data = [dict[@"data"] isKindOfClass:NSString.class]
          ? (NSString*)dict[@"data"]
          : nil;
      if (data.length > 0) {
        dataURL = [@"data:image/png;base64," stringByAppendingString:data];
      }
    }
    if (!dataURL) {
      NSDictionary* dict =
          [parsed isKindOfClass:NSDictionary.class] ? (NSDictionary*)parsed : nil;
      NSString* message = [dict[@"message"] isKindOfClass:NSString.class]
          ? (NSString*)dict[@"message"]
          : nil;
      if (message.length > 0) {
        error = message;
      }
      if (error.length == 0) {
        error = success ? @"Chromium returned an empty screenshot."
                        : @"Chromium screenshot capture failed.";
      }
    }

    NSMutableDictionary* response =
        [@{@"requestId" : request_id_ ?: @"",
           @"extensionId" : extension_id_ ?: @""} mutableCopy];
    if (dataURL) {
      response[@"result"] = dataURL;
    } else {
      response[@"error"] = error ?: @"Chromium screenshot capture failed.";
    }
    [MoriBrowserView dispatchExtensionBridgeResponse:response];
    registration_ = nullptr;
  }

 private:
  NSString* extension_id_;
  NSString* request_id_;
  int message_id_ = 0;
  CefRefPtr<CefRegistration> registration_;

  IMPLEMENT_REFCOUNTING(ScreenshotObserver);
};

class JavaScriptEvalObserver : public CefDevToolsMessageObserver {
 public:
  explicit JavaScriptEvalObserver(MoriJavaScriptResultHandler completion)
      : completion_([completion copy]) {}

  void SetMessageID(int message_id) { message_id_ = message_id; }
  void SetRegistration(CefRefPtr<CefRegistration> registration) {
    registration_ = registration;
  }

  void OnDevToolsMethodResult(CefRefPtr<CefBrowser> browser,
                              int message_id,
                              bool success,
                              const void* result,
                              size_t result_size) override {
    if (completed_ || message_id_ == 0 || message_id != message_id_) return;
    completed_ = true;

    NSString* error = nil;
    id value = nil;
    NSData* jsonData = result_size > 0
        ? [NSData dataWithBytes:result length:result_size]
        : nil;
    id parsed = jsonData
        ? [NSJSONSerialization JSONObjectWithData:jsonData options:0 error:nil]
        : nil;
    NSDictionary* dict =
        [parsed isKindOfClass:NSDictionary.class] ? (NSDictionary*)parsed : nil;
    NSDictionary* exception =
        [dict[@"exceptionDetails"] isKindOfClass:NSDictionary.class]
            ? (NSDictionary*)dict[@"exceptionDetails"]
            : nil;
    if (exception) {
      NSDictionary* exceptionObject =
          [exception[@"exception"] isKindOfClass:NSDictionary.class]
              ? (NSDictionary*)exception[@"exception"]
              : nil;
      NSString* description =
          [exceptionObject[@"description"] isKindOfClass:NSString.class]
              ? (NSString*)exceptionObject[@"description"]
              : nil;
      NSString* text = [exception[@"text"] isKindOfClass:NSString.class]
          ? (NSString*)exception[@"text"]
          : nil;
      error = description.length > 0 ? description : (text ?: @"JavaScript failed.");
    } else if (success) {
      NSDictionary* resultObject =
          [dict[@"result"] isKindOfClass:NSDictionary.class]
              ? (NSDictionary*)dict[@"result"]
              : nil;
      value = resultObject[@"value"];
      if (!value || value == [NSNull null]) {
        NSString* description =
            [resultObject[@"description"] isKindOfClass:NSString.class]
                ? (NSString*)resultObject[@"description"]
                : nil;
        value = description ?: [NSNull null];
      }
    } else {
      error = @"Chromium JavaScript evaluation failed.";
    }

    MoriJavaScriptResultHandler completion = completion_;
    if (completion) {
      id callbackValue = value ?: [NSNull null];
      NSString* callbackError = error;
      dispatch_async(dispatch_get_main_queue(), ^{
        completion(callbackValue, callbackError);
      });
    }
    completion_ = nil;
    if (registration_) {
      registration_ = nullptr;
    }
  }

 private:
  int message_id_ = 0;
  bool completed_ = false;
  CefRefPtr<CefRegistration> registration_;
  MoriJavaScriptResultHandler completion_;

  IMPLEMENT_REFCOUNTING(JavaScriptEvalObserver);
};

// Weak registry of every live tracker view, so the class-level suppression
// toggle can re-sync them all.
static NSHashTable<MoriBrowserView*>* g_all_views = nil;

@implementation MoriBrowserView {
  CefRefPtr<CefBrowser> _browser;
  CefRefPtr<BrowserClient> _client;
  ViewClientDelegate* _delegate;  // owned
  NSView* _browserContentView;    // CEF's embedded child view.
  NSString* _pendingURL;
  NSString* _lastFindText;  // distinguishes a new search from find-next.
  NSInteger _extensionTabID;
  BOOL _created;
  BOOL _webWindowVisible;
  BOOL _ignoresGlobalWebContentSuppression;
}

@synthesize currentURL = _currentURL;
@synthesize currentTitle = _currentTitle;
@synthesize isLoading = _isLoading;
@synthesize canGoBack = _canGoBack;
@synthesize canGoForward = _canGoForward;

+ (void)initialize {
  if (self == [MoriBrowserView class]) {
    g_all_views = [NSHashTable weakObjectsHashTable];
  }
}

- (instancetype)initWithURL:(NSString*)url {
  self = [super initWithFrame:NSZeroRect];
  if (self) {
    _pendingURL = [url copy] ?: @"about:blank";
    _currentURL = [_pendingURL copy];
    _currentTitle = @"";
    self.wantsLayer = YES;
    self.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    _webWindowVisible = YES;
    _delegate = new ViewClientDelegate(self);
    _client = new BrowserClient(_delegate);
    [g_all_views addObject:self];
  }
  return self;
}

- (void)dealloc {
  [[NSNotificationCenter defaultCenter] removeObserver:self];
  [self closeBrowser];
  if (_delegate) {
    delete _delegate;
    _delegate = nullptr;
  }
}

- (BOOL)isFlipped {
  return YES;
}

- (NSInteger)extensionTabID {
  return _extensionTabID;
}

- (void)setExtensionTabID:(NSInteger)extensionTabID {
  _extensionTabID = extensionTabID;
  if (_client) {
    _client->SetExtensionTabID(static_cast<int>(extensionTabID));
  }
}

// Create the browser only once installed in a window with a real size.
- (void)viewDidMoveToWindow {
  [super viewDidMoveToWindow];
  NSNotificationCenter* center = [NSNotificationCenter defaultCenter];
  [center removeObserver:self name:NSWindowDidBecomeKeyNotification object:nil];
  [center removeObserver:self name:NSWindowDidResignKeyNotification object:nil];
  if (self.window) {
    [self _createBrowserIfReady];
    // Forward the window's key (focus) state to CEF so document.hasFocus()
    // tracks reality. Without this, a freshly loaded page or one whose window
    // just regained focus reports hasFocus()===false, which breaks extensions
    // that gate UI on focus (e.g. Proton Pass autofill suppresses its in-field
    // icon until the document is focused).
    [center addObserver:self
               selector:@selector(_windowFocusChanged:)
                   name:NSWindowDidBecomeKeyNotification
                 object:self.window];
    [center addObserver:self
               selector:@selector(_windowFocusChanged:)
                   name:NSWindowDidResignKeyNotification
                 object:self.window];
  }
  [self _syncBrowserVisibility];
  [self _syncBrowserFocus];
}

- (void)_windowFocusChanged:(NSNotification*)notification {
  [self _syncBrowserFocus];
}

- (void)setFrameSize:(NSSize)newSize {
  [super setFrameSize:newSize];
  [self _createBrowserIfReady];
  [self _syncBrowserFrame];
}

- (void)_createBrowserIfReady {
  if (_created || _browser) return;
  if (self.window == nil) return;
  NSRect bounds = self.bounds;
  if (bounds.size.width < 1 || bounds.size.height < 1) return;

  _created = YES;

  // Create on the next runloop turn, never nested inside the AppKit layout
  // pass that called us.
  __weak MoriBrowserView* weakSelf = self;
  dispatch_async(dispatch_get_main_queue(), ^{
    MoriBrowserView* strongSelf = weakSelf;
    if (strongSelf) [strongSelf _createBrowserNow];
  });
}

- (void)_createBrowserNow {
  if (_browser || self.window == nil) return;

  CefBrowserSettings settings;
  CefString cef_url;
  cef_url.FromString(std::string([_pendingURL UTF8String]));

  CefWindowInfo window_info;
  NSRect b = self.bounds;
  window_info.SetAsChild((__bridge void*)self,
                         CefRect(0, 0,
                                 static_cast<int>(b.size.width),
                                 static_cast<int>(b.size.height)));

  CefBrowserHost::CreateBrowser(window_info, _client.get(), cef_url, settings,
                                nullptr, nullptr);
}

- (void)_styleBrowserContentView {
  if (!_browserContentView) return;
  _browserContentView.wantsLayer = YES;
  if (_browserContentView.layer) {
    _browserContentView.layer.cornerRadius = kCardCornerRadius;
    _browserContentView.layer.cornerCurve = kCACornerCurveContinuous;
    _browserContentView.layer.masksToBounds = YES;
  }
}

- (void)_syncBrowserFrame {
  if (!_browserContentView) return;
  _browserContentView.frame = self.bounds;
  _browserContentView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
  if (_browser) {
    _browser->GetHost()->WasResized();
  }
}

- (void)_syncBrowserVisibility {
  const BOOL hidden = !_webWindowVisible ||
      (g_web_content_suppressed && !_ignoresGlobalWebContentSuppression);
  if (_browserContentView) {
    _browserContentView.hidden = hidden;
  }
  if (_browser) {
    _browser->GetHost()->WasHidden(hidden);
  }
  [self _syncBrowserFocus];
}

// The active tab's content view should hold CEF focus whenever its window is
// key, so document.hasFocus() is true for the page the user is looking at.
// Tie focus to (visible && window-is-key); hidden tabs and background windows
// release it.
- (void)_syncBrowserFocus {
  if (!_browser) return;
  const BOOL hidden = !_webWindowVisible ||
      (g_web_content_suppressed && !_ignoresGlobalWebContentSuppression);
  const BOOL windowKey = self.window != nil && self.window.isKeyWindow;
  const BOOL shouldFocus = !hidden && windowKey;
  // Promote the page to first responder, not just SetFocus: a freshly-created
  // tab (opened via the launcher, which grabs first-responder for its search
  // field and never hands it back) would otherwise have a focused-at-the-CEF-
  // -level-but-not-key-in-AppKit view, so document.hasFocus() stays false and
  // focus-gated extension UI (Proton Pass autofill) never loads until the user
  // clicks the page. We only steal first responder for the visible tab of a key
  // window, and the launcher is always dismissed before a new tab's browser
  // attaches, so this never yanks focus out from under the open launcher.
  if (shouldFocus && _browserContentView && self.window &&
      self.window.firstResponder != _browserContentView) {
    [self.window makeFirstResponder:_browserContentView];
  }
  _browser->GetHost()->SetFocus(shouldFocus);
}

- (void)setHidden:(BOOL)hidden {
  [super setHidden:hidden];
  _webWindowVisible = !hidden;
  [self _syncBrowserVisibility];
}

- (void)setWebWindowVisible:(BOOL)visible {
  if (_webWindowVisible == visible) return;
  _webWindowVisible = visible;
  [self _syncBrowserVisibility];
}

- (void)setIgnoresGlobalWebContentSuppression:(BOOL)ignores {
  if (_ignoresGlobalWebContentSuppression == ignores) return;
  _ignoresGlobalWebContentSuppression = ignores;
  [self _syncBrowserVisibility];
}

// AppKit calls this whenever our own frame changes (incl. SwiftUI relayout).
- (void)resizeSubviewsWithOldSize:(NSSize)oldSize {
  [super resizeSubviewsWithOldSize:oldSize];
  [self _syncBrowserFrame];
}

- (void)layout {
  [super layout];
  [self _syncBrowserFrame];
}

- (void)_attachBrowser:(CefRefPtr<CefBrowser>)browser {
  _browser = browser;
  _browserContentView = ViewFromCEFHandle(browser->GetHost()->GetWindowHandle());
  EmitEngineAuditMarker(browser->GetHost()->GetRuntimeStyle());
  [self _styleBrowserContentView];
  [self _syncBrowserFrame];
  [self _syncBrowserVisibility];
  [self _syncBrowserFocus];
}

- (void)_detachBrowser {
  _browser = nullptr;
}

#pragma mark - Suppression (class-level)

+ (void)setWebContentSuppressed:(BOOL)suppressed {
  if (g_web_content_suppressed == (bool)suppressed) return;
  g_web_content_suppressed = suppressed;
  for (MoriBrowserView* view in g_all_views) {
    [view _syncBrowserVisibility];
  }
}

+ (void)dispatchExtensionMessage:(id)message
                  forExtensionID:(NSString*)extensionID {
  [self dispatchExtensionMessage:message
                  forExtensionID:extensionID
                       requestID:nil
                       sourceURL:nil];
}

+ (void)dispatchExtensionMessage:(id)message
                  forExtensionID:(NSString*)extensionID
                       requestID:(NSString*)requestID {
  [self dispatchExtensionMessage:message
                  forExtensionID:extensionID
                       requestID:requestID
                       sourceURL:nil];
}

+ (void)dispatchExtensionMessage:(id)message
                  forExtensionID:(NSString*)extensionID
                       requestID:(NSString*)requestID
                       sourceURL:(NSString*)sourceURL {
  [self dispatchExtensionMessage:message
                  forExtensionID:extensionID
                       requestID:requestID
                       sourceURL:sourceURL
                    sourceOrigin:nil];
}

+ (void)dispatchExtensionMessage:(id)message
                  forExtensionID:(NSString*)extensionID
                       requestID:(NSString*)requestID
                       sourceURL:(NSString*)sourceURL
                    sourceOrigin:(NSString*)sourceOrigin {
  [self dispatchExtensionMessage:message
                  forExtensionID:extensionID
                       requestID:requestID
                       sourceURL:sourceURL
                    sourceOrigin:sourceOrigin
                        external:NO];
}

+ (void)dispatchExtensionMessage:(id)message
                  forExtensionID:(NSString*)extensionID
                       requestID:(NSString*)requestID
                       sourceURL:(NSString*)sourceURL
                    sourceOrigin:(NSString*)sourceOrigin
                        external:(BOOL)external {
  [self dispatchExtensionMessage:message
                  forExtensionID:extensionID
                       requestID:requestID
                       sourceURL:sourceURL
                    sourceOrigin:sourceOrigin
                     sourceTabID:-1
                        external:external];
}

+ (void)dispatchExtensionMessage:(id)message
                  forExtensionID:(NSString*)extensionID
                       requestID:(NSString*)requestID
                       sourceURL:(NSString*)sourceURL
                    sourceOrigin:(NSString*)sourceOrigin
                     sourceTabID:(NSInteger)sourceTabID
                        external:(BOOL)external {
  [self dispatchExtensionMessage:message
                  forExtensionID:extensionID
                       requestID:requestID
                       sourceURL:sourceURL
                    sourceOrigin:sourceOrigin
                     sourceTabID:sourceTabID
                    sourceFrameID:-1
                 sourceDocumentID:nil
                        external:external];
}

+ (void)dispatchExtensionMessage:(id)message
                  forExtensionID:(NSString*)extensionID
                       requestID:(NSString*)requestID
                       sourceURL:(NSString*)sourceURL
                    sourceOrigin:(NSString*)sourceOrigin
                     sourceTabID:(NSInteger)sourceTabID
                    sourceFrameID:(NSInteger)sourceFrameID
                 sourceDocumentID:(NSString*)sourceDocumentID
                        external:(BOOL)external {
  if (extensionID.length == 0) return;
  // sourceURL lets the in-page handler skip the sending document, matching
  // Chrome's rule that runtime.sendMessage is never delivered to its sender.
  // The 6th arg (toContentScript) is false: a runtime.sendMessage broadcast
  // reaches extension contexts only, never content scripts (tabs.sendMessage
  // routes through a separate path that opts into content-script delivery).
  // The 7th arg (external) routes to onMessageExternal with an id-less sender.
  // The 8th arg carries the originating tab id so external account handoffs can
  // see a Chrome-like MessageSender.tab. The 9th arg carries the originating
  // frame id; content-script control planes such as Proton Pass reject messages
  // when sender.frameId is missing. The 10th arg carries modern Chrome's
  // MessageSender.documentId.
  NSString* source = [NSString stringWithFormat:
      @"if(window.__moriExtDispatchMessage){"
       "window.__moriExtDispatchMessage(%@,%@,%@,%@,%@,false,%@,%@,%@,%@);}",
      JSONLiteral(extensionID), JSONLiteral(message ?: [NSNull null]),
      JSONLiteral(requestID ?: [NSNull null]),
      JSONLiteral(sourceURL ?: [NSNull null]),
      JSONLiteral(sourceOrigin ?: [NSNull null]),
      external ? @"true" : @"false",
      sourceTabID >= 0 ? [NSString stringWithFormat:@"%ld", (long)sourceTabID]
                       : @"null",
      sourceFrameID >= 0 ? [NSString stringWithFormat:@"%ld", (long)sourceFrameID]
                         : @"null",
      JSONLiteral(sourceDocumentID ?: [NSNull null])];
  for (MoriBrowserView* view in g_all_views) {
    [view executeExtensionJavaScript:source allFrames:YES];
  }
}

+ (void)dispatchExtensionBridgeResponse:(NSDictionary*)response {
  if (![response isKindOfClass:NSDictionary.class]) return;
  NSString* source = [NSString stringWithFormat:
      @"if(window.__moriExtResolve){window.__moriExtResolve(%@);}",
      JSONLiteral(response)];
  for (MoriBrowserView* view in g_all_views) {
    [view executeExtensionJavaScript:source allFrames:YES];
  }
}

+ (void)dispatchExtensionEvent:(NSString*)eventName
                          args:(NSArray*)args
                forExtensionID:(NSString*)extensionID {
  if (eventName.length == 0) return;
  NSString* source = [NSString stringWithFormat:
      @"if(window.__moriExtDispatchEvent){"
       "window.__moriExtDispatchEvent(%@,%@,%@);}",
      JSONLiteral(eventName), JSONLiteral(args ?: @[]),
      JSONLiteral(extensionID ?: [NSNull null])];
  for (MoriBrowserView* view in g_all_views) {
    [view executeExtensionJavaScript:source allFrames:YES];
  }
}

+ (void)broadcastExtensionJavaScript:(NSString*)source
                      forExtensionID:(NSString*)extensionID {
  if (source.length == 0) return;
  NSString* guarded = source;
  if (extensionID.length > 0) {
    guarded = [NSString
        stringWithFormat:@"if(window.__moriExtensionID===%@){%@}",
                         JSONLiteral(extensionID), source];
  }
  for (MoriBrowserView* view in g_all_views) {
    [view executeExtensionJavaScript:guarded allFrames:YES];
  }
}

#pragma mark - Public API

- (void)loadURL:(NSString*)url {
  if (url.length == 0) return;
  _pendingURL = [url copy];
  if (_browser) {
    CefString cef_url;
    cef_url.FromString(std::string([url UTF8String]));
    _browser->GetMainFrame()->LoadURL(cef_url);
  }
}

- (void)goBack {
  if (_browser) _browser->GoBack();
}
- (void)goForward {
  if (_browser) _browser->GoForward();
}
- (void)reload {
  if (_browser) _browser->Reload();
}
- (void)reloadIgnoringCache {
  if (_browser) _browser->ReloadIgnoreCache();
}
- (void)stopLoading {
  if (_browser) _browser->StopLoad();
}

- (void)zoomIn {
  if (_browser) {
    CefRefPtr<CefBrowserHost> host = _browser->GetHost();
    host->SetZoomLevel(host->GetZoomLevel() + kZoomStep);
  }
}
- (void)zoomOut {
  if (_browser) {
    CefRefPtr<CefBrowserHost> host = _browser->GetHost();
    host->SetZoomLevel(host->GetZoomLevel() - kZoomStep);
  }
}
- (void)resetZoom {
  if (_browser) _browser->GetHost()->SetZoomLevel(0.0);
}
- (void)setZoomFactor:(double)factor {
  if (!_browser) return;
  double safe_factor = factor;
  if (safe_factor < 0.25) safe_factor = 0.25;
  if (safe_factor > 5.0) safe_factor = 5.0;
  _browser->GetHost()->SetZoomLevel(log(safe_factor) / log(1.2));
}

- (void)findText:(NSString*)text forward:(BOOL)forward {
  if (!_browser) return;
  if (text.length == 0) {
    [self stopFinding:YES];
    return;
  }
  // findNext == YES when continuing the same query; NO starts a fresh search.
  BOOL findNext = (_lastFindText && [_lastFindText isEqualToString:text]);
  _lastFindText = [text copy];
  CefString needle;
  needle.FromString(std::string([text UTF8String]));
  _browser->GetHost()->Find(needle, forward, /*matchCase=*/false, findNext);
}

- (void)stopFinding:(BOOL)clearSelection {
  _lastFindText = nil;
  if (_browser) _browser->GetHost()->StopFinding(clearSelection);
}

- (void)showDevTools {
  if (!_browser) return;
  CefWindowInfo window_info;  // Default → DevTools open in their own window.
  CefBrowserSettings settings;
  _browser->GetHost()->ShowDevTools(window_info, nullptr, settings, CefPoint());
}

- (void)closeDevTools {
  if (_browser) _browser->GetHost()->CloseDevTools();
}

- (void)toggleDevTools {
  if (!_browser) return;
  if (_browser->GetHost()->HasDevTools()) {
    [self closeDevTools];
  } else {
    [self showDevTools];
  }
}

- (void)printPage {
  if (_browser) _browser->GetHost()->Print();
}

- (BOOL)startDownload:(NSString*)url
          extensionID:(NSString*)extensionID
            requestID:(NSString*)requestID
             filename:(NSString*)filename {
  if (url.length == 0 || !_browser) return NO;
  MoriRegisterExtensionDownload(url, extensionID ?: @"", requestID ?: @"",
                                  filename ?: @"");
  _browser->GetHost()->StartDownload(CefString(url.UTF8String));
  return YES;
}

- (void)executeExtensionJavaScript:(NSString*)source allFrames:(BOOL)allFrames {
  if (!_browser || source.length == 0) return;
  auto runInFrame = ^(CefRefPtr<CefFrame> frame) {
    if (!frame) return;
    NSString* documentID = MoriExtensionDocumentID(frame);
    NSString* frameSource = [NSString stringWithFormat:
        @"try{Object.defineProperty(globalThis,'__moriNativeFrameId',{"
         "configurable:true,value:%d});}catch(e){try{globalThis.__moriNativeFrameId=%d;}catch(_e){}}\n"
         "try{Object.defineProperty(globalThis,'__moriNativeDocumentId',{"
         "configurable:true,value:%@});}catch(e){try{globalThis.__moriNativeDocumentId=%@;}catch(_e){}}\n%@",
        MoriExtensionFrameID(frame), MoriExtensionFrameID(frame),
        JSONLiteral(documentID), JSONLiteral(documentID), source];
    CefString code(frameSource.UTF8String);
    frame->ExecuteJavaScript(code, frame->GetURL(), 0);
  };
  if (!allFrames) {
    CefRefPtr<CefFrame> frame = _browser->GetMainFrame();
    runInFrame(frame);
    return;
  }

  std::vector<CefString> ids;
  _browser->GetFrameIdentifiers(ids);
  for (const auto& id : ids) {
    CefRefPtr<CefFrame> frame = _browser->GetFrameByIdentifier(id);
    runInFrame(frame);
  }
}

- (NSArray<NSDictionary*>*)extensionFrameRecordsWithTabID:(NSInteger)tabID {
  if (!_browser) return @[];

  NSMutableArray<NSDictionary*>* records = [NSMutableArray array];
  std::vector<CefString> ids;
  _browser->GetFrameIdentifiers(ids);
  for (const auto& id : ids) {
    CefRefPtr<CefFrame> frame = _browser->GetFrameByIdentifier(id);
    if (frame && frame->IsValid()) {
      [records addObject:MoriExtensionFrameRecord(frame, tabID)];
    }
  }
  if (records.count == 0) {
    CefRefPtr<CefFrame> main = _browser->GetMainFrame();
    if (main && main->IsValid()) {
      [records addObject:MoriExtensionFrameRecord(main, tabID)];
    }
  }
  return records;
}

- (BOOL)evaluateJavaScript:(NSString*)source
                completion:(MoriJavaScriptResultHandler)completion {
  if (!_browser || source.length == 0 || !completion) return NO;

  CefRefPtr<JavaScriptEvalObserver> observer(
      new JavaScriptEvalObserver(completion));
  CefRefPtr<CefRegistration> registration =
      _browser->GetHost()->AddDevToolsMessageObserver(observer);
  if (!registration) {
    return NO;
  }
  observer->SetRegistration(registration);

  CefRefPtr<CefDictionaryValue> params = CefDictionaryValue::Create();
  params->SetString("expression", CefString(source.UTF8String));
  params->SetBool("awaitPromise", true);
  params->SetBool("returnByValue", true);
  params->SetBool("userGesture", true);
  int messageID = _browser->GetHost()->ExecuteDevToolsMethod(
      0, CefString("Runtime.evaluate"), params);
  if (messageID == 0) {
    return NO;
  }
  observer->SetMessageID(messageID);
  return YES;
}

- (BOOL)captureVisiblePNGDataURLForExtensionID:(NSString*)extensionID
                                     requestID:(NSString*)requestID {
  if (!_browser || extensionID.length == 0 || requestID.length == 0) {
    return NO;
  }

  CefRefPtr<ScreenshotObserver> observer(
      new ScreenshotObserver(extensionID, requestID));
  CefRefPtr<CefRegistration> registration =
      _browser->GetHost()->AddDevToolsMessageObserver(observer);
  if (!registration) {
    return NO;
  }
  observer->SetRegistration(registration);

  CefRefPtr<CefDictionaryValue> params = CefDictionaryValue::Create();
  params->SetString("format", "png");
  params->SetBool("fromSurface", true);
  int messageID = _browser->GetHost()->ExecuteDevToolsMethod(
      0, CefString("Page.captureScreenshot"), params);
  if (messageID == 0) {
    return NO;
  }
  observer->SetMessageID(messageID);
  return YES;
}

- (void)focusBrowser {
  if (_browserContentView.window) {
    [_browserContentView.window makeFirstResponder:_browserContentView];
  }
  if (_browser) {
    _browser->GetHost()->SetFocus(true);
  }
}

#pragma mark - Media / Picture-in-Picture

- (int)browserIdentifier {
  return _browser ? _browser->GetIdentifier() : 0;
}

- (void)sendMediaCommand:(NSString*)action value:(double)value {
  if (!_browser || action.length == 0) return;
  std::string js = "if(window.__moriMedia){window.__moriMedia('" +
                   std::string([action UTF8String]) + "'," +
                   std::to_string(value) + ");}";
  // The active media may live in any frame (e.g. an embedded player), so run
  // the command in every frame; only the one with the element responds.
  std::vector<CefString> ids;
  _browser->GetFrameIdentifiers(ids);
  for (const auto& id : ids) {
    CefRefPtr<CefFrame> frame = _browser->GetFrameByIdentifier(id);
    if (frame) {
      frame->ExecuteJavaScript(js, frame->GetURL(), 0);
    }
  }
}

- (void)setPageHidden:(BOOL)hidden {
  if (_browser) {
    _browser->GetHost()->WasHidden(hidden);
  }
}

- (void)applyAutoPiP:(BOOL)enabled {
  MoriSetAutoPiPEnabled(enabled);
  if (_browser) {
    std::string js =
        std::string("window.__moriAutoPiP=") + (enabled ? "true" : "false") + ";";
    _browser->GetMainFrame()->ExecuteJavaScript(js, "", 0);
  }
}

+ (void)setAutoPiPEnabled:(BOOL)enabled {
  MoriSetAutoPiPEnabled(enabled);
}

+ (void)setAdBlockerEnabled:(BOOL)enabled {
  MoriSetAdBlockerEnabled(enabled);
}

+ (BOOL)cancelDownloadWithID:(uint32_t)downloadID {
  return MoriCancelDownload(downloadID);
}

- (void)closeBrowser {
  [NSNotificationCenter.defaultCenter removeObserver:self];
  if (_client) {
    _client->DetachDelegate();
  }
  _browserContentView = nil;
  if (_browser) {
    // Closing the browser tears down its CEF-owned child view too.
    _browser->GetHost()->CloseBrowser(true);
    _browser = nullptr;
  }
}

#pragma mark - State application (from C++ delegate, main thread)

- (void)_applyTitle:(NSString*)title {
  _currentTitle = [title copy];
  id<MoriBrowserViewDelegate> d = self.navDelegate;
  if ([d respondsToSelector:@selector(browserView:didChangeTitle:)]) {
    [d browserView:self didChangeTitle:title];
  }
}

- (void)_applyURL:(NSString*)url {
  _currentURL = [url copy];
  id<MoriBrowserViewDelegate> d = self.navDelegate;
  if ([d respondsToSelector:@selector(browserView:didChangeURL:)]) {
    [d browserView:self didChangeURL:url];
  }
}

- (void)_applyLoading:(BOOL)isLoading
            canGoBack:(BOOL)canGoBack
         canGoForward:(BOOL)canGoForward {
  _isLoading = isLoading;
  _canGoBack = canGoBack;
  _canGoForward = canGoForward;
  id<MoriBrowserViewDelegate> d = self.navDelegate;
  if ([d respondsToSelector:@selector(browserView:
                                 didChangeLoading:canGoBack:canGoForward:)]) {
    [d browserView:self
        didChangeLoading:isLoading
               canGoBack:canGoBack
            canGoForward:canGoForward];
  }
}

- (void)_applyFaviconURLs:(NSArray<NSString*>*)urls {
  id<MoriBrowserViewDelegate> d = self.navDelegate;
  if ([d respondsToSelector:@selector(browserView:didChangeFaviconURLs:)]) {
    [d browserView:self didChangeFaviconURLs:urls];
  }
}

- (void)_applyFaviconImage:(NSImage*)image {
  id<MoriBrowserViewDelegate> d = self.navDelegate;
  if ([d respondsToSelector:@selector(browserView:didLoadFaviconImage:)]) {
    [d browserView:self didLoadFaviconImage:image];
  }
}

- (void)_applyNavigationStart:(NSString*)url
                   isRedirect:(BOOL)isRedirect
                  userGesture:(BOOL)userGesture {
  id<MoriBrowserViewDelegate> d = self.navDelegate;
  if ([d respondsToSelector:@selector(browserView:
                          didStartNavigationToURL:isRedirect:userGesture:)]) {
    [d browserView:self
        didStartNavigationToURL:url
                     isRedirect:isRedirect
                    userGesture:userGesture];
  }
}

- (void)_applyNavigationCommit:(NSString*)url {
  id<MoriBrowserViewDelegate> d = self.navDelegate;
  if ([d respondsToSelector:@selector(browserView:didCommitNavigationToURL:)]) {
    [d browserView:self didCommitNavigationToURL:url];
  }
}

- (void)_applyNavigationFinish:(NSString*)url httpStatusCode:(NSInteger)code {
  id<MoriBrowserViewDelegate> d = self.navDelegate;
  if ([d respondsToSelector:@selector(browserView:
                               didFinishNavigationToURL:httpStatusCode:)]) {
    [d browserView:self didFinishNavigationToURL:url httpStatusCode:code];
  }
}

- (void)_applyLoadError:(NSString*)errorText failedURL:(NSString*)failedURL {
  id<MoriBrowserViewDelegate> d = self.navDelegate;
  if ([d respondsToSelector:@selector(browserView:didFailLoad:failedURL:)]) {
    [d browserView:self didFailLoad:errorText failedURL:failedURL];
  }
}

- (void)_requestNewTab:(NSString*)url {
  id<MoriBrowserViewDelegate> d = self.navDelegate;
  if ([d respondsToSelector:@selector(browserView:requestsNewTabWithURL:)]) {
    [d browserView:self requestsNewTabWithURL:url];
  }
}

- (void)_applyFindOrdinal:(int)ordinal ofMatches:(int)count {
  id<MoriBrowserViewDelegate> d = self.navDelegate;
  if ([d respondsToSelector:@selector(browserView:
                                 didUpdateFindMatchOrdinal:ofMatches:)]) {
    [d browserView:self didUpdateFindMatchOrdinal:ordinal ofMatches:count];
  }
}

@end
