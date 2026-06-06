#include "BrowserClient.h"

#import <Cocoa/Cocoa.h>

#include <atomic>
#include <cstring>
#include <map>

#include "MediaAgentScript.h"
#include "PasskeyAgentScript.h"
#include "include/cef_cookie.h"
#include "include/cef_download_item.h"
#include "include/cef_request_context.h"
#include "include/cef_values.h"
#include "include/internal/cef_time.h"
#include "include/wrapper/cef_stream_resource_handler.h"
#include "include/wrapper/cef_helpers.h"

#include "../Shared/MoriSchemes.h"

// Swift bridge (generated from the @objc MoriPasskeys interface).
#import "Mori-Swift.h"
#import "../Bridge/MoriBrowserView.h"

// Broadcast name used to drive the SwiftUI Downloads panel. userInfo carries
// the keys read by `DownloadStore` (id, url, filename, path, bytes, percent,
// state flags).
NSString* const kMoriDownloadUpdated = @"MoriDownloadUpdated";

// Broadcast name driving the SwiftUI media player. userInfo: {browserId, json}.
NSString* const kMoriMediaUpdated = @"MoriMediaUpdated";

// Auto-PiP preference, shared across browsers, set from the Swift settings UI.
static std::atomic<bool> g_mori_auto_pip{false};
void MoriSetAutoPiPEnabled(bool enabled) { g_mori_auto_pip.store(enabled); }
bool MoriAutoPiPEnabled() { return g_mori_auto_pip.load(); }

// Built-in ad blocker preference. Default on; Swift settings may override it
// after UserDefaults loads.
static std::atomic<bool> g_mori_ad_blocker{true};
void MoriSetAdBlockerEnabled(bool enabled) {
  g_mori_ad_blocker.store(enabled);
}
bool MoriAdBlockerEnabled() { return g_mori_ad_blocker.load(); }

static const char* kMoriWebNavigationAgent = R"JS(
(function(){
  if(window.__moriWebNavigationHooked)return;
  window.__moriWebNavigationHooked=true;
  function emit(eventName,url){
    try{
      console.info("__MORI_WEBNAV__"+JSON.stringify({
        event:eventName,
        url:String(url||location.href)
      }));
    }catch(e){}
  }
  var pushState=history.pushState;
  var replaceState=history.replaceState;
  history.pushState=function(){
    var result=pushState.apply(this,arguments);
    emit("webNavigation.onHistoryStateUpdated",location.href);
    return result;
  };
  history.replaceState=function(){
    var result=replaceState.apply(this,arguments);
    emit("webNavigation.onHistoryStateUpdated",location.href);
    return result;
  };
  addEventListener("hashchange",function(){
    emit("webNavigation.onReferenceFragmentUpdated",location.href);
  },true);
})();
)JS";

NSMutableDictionary<NSString*, NSMutableArray<NSDictionary*>*>*
ExtensionDownloadPendingByURL() {
  static NSMutableDictionary<NSString*, NSMutableArray<NSDictionary*>*>* pending =
      [NSMutableDictionary dictionary];
  return pending;
}

NSMutableDictionary<NSNumber*, NSString*>* ExtensionCrxDownloadTargets() {
  static NSMutableDictionary<NSNumber*, NSString*>* targets =
      [NSMutableDictionary dictionary];
  return targets;
}

NSString* RewriteChromeExtensionURLForMori(NSString* raw) {
  if (raw.length == 0) return nil;
  NSURLComponents* components = [NSURLComponents componentsWithString:raw];
  NSString* scheme = components.scheme.lowercaseString ?: @"";
  NSString* legacyExtensionScheme =
      [@"chrome" stringByAppendingString:@"-extension"];
  if (![scheme isEqualToString:legacyExtensionScheme]) return nil;
  components.scheme = @(mori::kExtensionScheme);
  return components.string ?: nil;
}

void MoriRegisterExtensionDownload(NSString* url,
                                     NSString* extensionID,
                                     NSString* requestID,
                                     NSString* filename) {
  if (url.length == 0 || requestID.length == 0) return;
  NSMutableDictionary* pending = ExtensionDownloadPendingByURL();
  @synchronized(pending) {
    NSMutableArray* queue = pending[url];
    if (!queue) {
      queue = [NSMutableArray array];
      pending[url] = queue;
    }
    NSMutableDictionary* request =
        [@{@"extensionId" : extensionID ?: @"", @"requestId" : requestID ?: @""}
            mutableCopy];
    if (filename.length > 0) {
      request[@"filename"] = filename;
    }
    [queue addObject:request];
  }
}

NSDictionary* TakeExtensionDownloadRequest(NSString* url) {
  if (url.length == 0) return nil;
  NSMutableDictionary* pending = ExtensionDownloadPendingByURL();
  @synchronized(pending) {
    NSMutableArray* queue = pending[url];
    NSDictionary* request = queue.firstObject;
    if (request) {
      [queue removeObjectAtIndex:0];
      if (queue.count == 0) [pending removeObjectForKey:url];
    }
    return request;
  }
}

void ResolveExtensionDownloadRequest(NSDictionary* request,
                                     NSNumber* downloadID,
                                     NSString* error = nil) {
  NSString* requestID = [request[@"requestId"] isKindOfClass:NSString.class]
      ? request[@"requestId"]
      : @"";
  NSString* extensionID = [request[@"extensionId"] isKindOfClass:NSString.class]
      ? request[@"extensionId"]
      : @"";
  if (requestID.length == 0 || extensionID.length == 0) return;
  NSMutableDictionary* response =
      [@{@"requestId" : requestID, @"extensionId" : extensionID} mutableCopy];
  if (error.length > 0) {
    response[@"error"] = error;
  } else {
    response[@"result"] = downloadID ?: [NSNull null];
  }
  [MoriBrowserView dispatchExtensionBridgeResponse:response];
}

BrowserClient::BrowserClient(BrowserClientDelegate* delegate)
    : delegate_(delegate) {}

void BrowserClient::DetachDelegate() {
  delegate_ = nullptr;
}

void BrowserClient::SetExtensionTabID(int tab_id) {
  extension_tab_id_.store(tab_id);
}

// --- CefLifeSpanHandler -----------------------------------------------------

bool BrowserClient::OnBeforePopup(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    int popup_id,
    const CefString& target_url,
    const CefString& target_frame_name,
    CefLifeSpanHandler::WindowOpenDisposition target_disposition,
    bool user_gesture,
    const CefPopupFeatures& popupFeatures,
    CefWindowInfo& windowInfo,
    CefRefPtr<CefClient>& client,
    CefBrowserSettings& settings,
    CefRefPtr<CefDictionaryValue>& extra_info,
    bool* no_javascript_access) {
  CEF_REQUIRE_UI_THREAD();
  // Open popups/target=_blank navigations as Mori-managed tabs instead of
  // letting CEF spawn native child windows we do not own.
  if (delegate_ && !target_url.empty()) {
    NSString* raw = @(target_url.ToString().c_str());
    NSString* rewritten = RewriteChromeExtensionURLForMori(raw);
    NSString* target = rewritten ?: raw;
    delegate_->OnOpenURLFromTab(std::string(target.UTF8String ?: ""));
  }
  return true;  // Cancel the popup.
}

void BrowserClient::OnAfterCreated(CefRefPtr<CefBrowser> browser) {
  CEF_REQUIRE_UI_THREAD();
  if (delegate_) {
    delegate_->OnAfterCreated(browser);
  }
}

void BrowserClient::OnBeforeClose(CefRefPtr<CefBrowser> browser) {
  CEF_REQUIRE_UI_THREAD();
  if (delegate_) {
    delegate_->OnBeforeClose(browser);
  }
}

// --- CefRequestHandler ------------------------------------------------------

bool BrowserClient::OnBeforeBrowse(CefRefPtr<CefBrowser> browser,
                                   CefRefPtr<CefFrame> frame,
                                   CefRefPtr<CefRequest> request,
                                   bool user_gesture,
                                   bool is_redirect) {
  CEF_REQUIRE_UI_THREAD();
  if (frame && frame->IsMain() && request) {
    NSString* rewritten = RewriteChromeExtensionURLForMori(
        @(request->GetURL().ToString().c_str()));
    if (rewritten.length > 0) {
      frame->LoadURL(CefString(rewritten.UTF8String));
      return true;
    }
  }
  if (delegate_ && frame && frame->IsMain() && request) {
    delegate_->OnBeforeBrowse(request->GetURL().ToString(), is_redirect,
                              user_gesture);
  }
  return false;
}

bool BrowserClient::OnOpenURLFromTab(CefRefPtr<CefBrowser> browser,
                                     CefRefPtr<CefFrame> frame,
                                     const CefString& target_url,
                                     WindowOpenDisposition target_disposition,
                                     bool user_gesture) {
  CEF_REQUIRE_UI_THREAD();
  if (!delegate_ || target_url.empty()) return false;
  NSString* rewritten =
      RewriteChromeExtensionURLForMori(@(target_url.ToString().c_str()));
  if (rewritten.length == 0) return false;
  return delegate_->OnOpenURLFromTab(std::string(rewritten.UTF8String ?: ""));
}

// --- CefLoadHandler ---------------------------------------------------------

void BrowserClient::OnLoadingStateChange(CefRefPtr<CefBrowser> browser,
                                         bool isLoading,
                                         bool canGoBack,
                                         bool canGoForward) {
  CEF_REQUIRE_UI_THREAD();
  if (delegate_) {
    delegate_->OnLoadingStateChange(isLoading, canGoBack, canGoForward);
  }
}

void BrowserClient::OnLoadError(CefRefPtr<CefBrowser> browser,
                                CefRefPtr<CefFrame> frame,
                                ErrorCode errorCode,
                                const CefString& errorText,
                                const CefString& failedUrl) {
  CEF_REQUIRE_UI_THREAD();
  // ERR_ABORTED (-3) is normal during fast re-navigation; ignore it.
  if (errorCode == ERR_ABORTED) {
    return;
  }
  if (delegate_) {
    delegate_->OnLoadError(errorCode, errorText.ToString(),
                           failedUrl.ToString());
  }
}

// --- CefDisplayHandler ------------------------------------------------------

void BrowserClient::OnTitleChange(CefRefPtr<CefBrowser> browser,
                                  const CefString& title) {
  CEF_REQUIRE_UI_THREAD();
  if (delegate_) {
    delegate_->OnTitleChange(title.ToString());
  }
}

void BrowserClient::OnAddressChange(CefRefPtr<CefBrowser> browser,
                                    CefRefPtr<CefFrame> frame,
                                    const CefString& url) {
  CEF_REQUIRE_UI_THREAD();
  if (delegate_ && frame->IsMain()) {
    delegate_->OnAddressChange(url.ToString());
  }
}

namespace {

// Bridges CefBrowserHost::DownloadImage back into the owning BrowserClient.
// Held alive by CEF for the duration of the download; keeps a ref to the client
// so the result can be delivered even if the page has since changed.
class FaviconDownloadCallback : public CefDownloadImageCallback {
 public:
  explicit FaviconDownloadCallback(CefRefPtr<BrowserClient> client)
      : client_(client) {}

  void OnDownloadImageFinished(const CefString& image_url,
                               int /*http_status_code*/,
                               CefRefPtr<CefImage> image) override {
    client_->DeliverFaviconImage(image_url, image);
  }

 private:
  CefRefPtr<BrowserClient> client_;
  IMPLEMENT_REFCOUNTING(FaviconDownloadCallback);
};

}  // namespace

void BrowserClient::OnFaviconURLChange(
    CefRefPtr<CefBrowser> browser,
    const std::vector<CefString>& icon_urls) {
  CEF_REQUIRE_UI_THREAD();
  if (!delegate_) return;

  std::vector<std::string> urls;
  urls.reserve(icon_urls.size());
  for (const auto& u : icon_urls) {
    urls.push_back(u.ToString());
  }
  delegate_->OnFaviconURLChange(urls);

  // Let Chromium download and decode the icon itself: this covers ICO, SVG,
  // data-URI and PNG favicons uniformly (a plain URLSession/AsyncImage load on
  // the Swift side cannot decode SVG or data-URIs, which is why those sites fell
  // back to the monogram). The first declared icon is the page's primary one;
  // `max_image_size` 64 keeps it crisp on Retina at our 15–20pt render sizes.
  if (!urls.empty() && browser && browser->GetHost()) {
    browser->GetHost()->DownloadImage(
        CefString(urls.front()), /*is_favicon=*/true, /*max_image_size=*/64,
        /*bypass_cache=*/false, new FaviconDownloadCallback(this));
  }
}

void BrowserClient::DeliverFaviconImage(const CefString& image_url,
                                        CefRefPtr<CefImage> image) {
  CEF_REQUIRE_UI_THREAD();
  if (!delegate_) return;

  if (!image || image->IsEmpty()) {
    delegate_->OnFaviconImage(image_url.ToString(), nullptr, 0);
    return;
  }

  int width = 0, height = 0;
  CefRefPtr<CefBinaryValue> png =
      image->GetAsPNG(1.0f, /*with_transparency=*/true, width, height);
  if (!png || png->GetSize() == 0) {
    delegate_->OnFaviconImage(image_url.ToString(), nullptr, 0);
    return;
  }

  std::vector<unsigned char> buffer(png->GetSize());
  png->GetData(buffer.data(), buffer.size(), 0);
  delegate_->OnFaviconImage(image_url.ToString(), buffer.data(), buffer.size());
}

// --- CefDownloadHandler -----------------------------------------------------

namespace {

// Pick a non-colliding path in ~/Downloads for `suggested`, appending " (n)"
// before the extension if a file already exists (Safari/Chrome behavior).
void DispatchExtensionEventOnMain(NSString* eventName,
                                  NSArray* args,
                                  NSString* extensionID);

std::map<uint32_t, CefRefPtr<CefDownloadItemCallback>>& DownloadCallbacks() {
  static std::map<uint32_t, CefRefPtr<CefDownloadItemCallback>> callbacks;
  return callbacks;
}

NSDictionary* CancelDownload(uint32_t download_id) {
  if (!MoriCancelDownload(download_id)) {
    return @{@"error" : @"No active download with that id."};
  }
  return @{@"result" : [NSNull null]};
}

NSMutableDictionary* NotificationStore() {
  static NSMutableDictionary* store = [NSMutableDictionary dictionary];
  return store;
}

NSMutableDictionary* NotificationsForExtension(NSString* extensionId) {
  NSString* key = extensionId.length > 0 ? extensionId : @"";
  NSMutableDictionary* all = NotificationStore();
  NSMutableDictionary* notifications = all[key];
  if (![notifications isKindOfClass:NSMutableDictionary.class]) {
    notifications = [NSMutableDictionary dictionary];
    all[key] = notifications;
  }
  return notifications;
}

NSString* GeneratedNotificationID() {
  return [NSString stringWithFormat:@"mori-notification-%@",
                                    NSUUID.UUID.UUIDString.lowercaseString];
}

NSDictionary* HandleNotifications(NSString* method,
                                  NSDictionary* args,
                                  NSString* extensionId) {
  NSMutableDictionary* notifications = NotificationsForExtension(extensionId);

  if ([method isEqualToString:@"notifications.create"]) {
    NSString* notificationID = [args[@"id"] isKindOfClass:NSString.class]
        ? args[@"id"]
        : @"";
    if (notificationID.length == 0) notificationID = GeneratedNotificationID();
    NSDictionary* options = [args[@"options"] isKindOfClass:NSDictionary.class]
        ? args[@"options"]
        : @{};
    notifications[notificationID] = [options mutableCopy];
    return @{@"result" : notificationID};
  }

  if ([method isEqualToString:@"notifications.update"]) {
    NSString* notificationID = [args[@"id"] isKindOfClass:NSString.class]
        ? args[@"id"]
        : @"";
    if (notificationID.length == 0) {
      return @{@"error" : @"Missing notification id."};
    }
    NSMutableDictionary* existing = notifications[notificationID];
    if (![existing isKindOfClass:NSMutableDictionary.class]) {
      return @{@"result" : @NO};
    }
    NSDictionary* options = [args[@"options"] isKindOfClass:NSDictionary.class]
        ? args[@"options"]
        : @{};
    [existing addEntriesFromDictionary:options];
    return @{@"result" : @YES};
  }

  if ([method isEqualToString:@"notifications.clear"]) {
    NSString* notificationID = [args[@"id"] isKindOfClass:NSString.class]
        ? args[@"id"]
        : @"";
    BOOL existed = notificationID.length > 0 && notifications[notificationID] != nil;
    if (existed) {
      [notifications removeObjectForKey:notificationID];
      DispatchExtensionEventOnMain(@"notifications.onClosed",
                                   @[ notificationID, @NO ], extensionId);
    }
    return @{@"result" : @(existed)};
  }

  if ([method isEqualToString:@"notifications.getAll"]) {
    return @{@"result" : [notifications copy]};
  }

  if ([method isEqualToString:@"notifications.getPermissionLevel"]) {
    return @{@"result" : @"granted"};
  }

  return @{@"error" : [NSString stringWithFormat:@"Unsupported notifications method: %@", method]};
}

NSString* UniqueDownloadPath(NSString* suggested) {
  NSArray<NSString*>* dirs = NSSearchPathForDirectoriesInDomains(
      NSDownloadsDirectory, NSUserDomainMask, YES);
  NSString* dir = dirs.firstObject ?: NSHomeDirectory();
  NSString* name = suggested.length ? suggested : @"download";
  NSString* base = name.stringByDeletingPathExtension;
  NSString* ext = name.pathExtension;

  NSFileManager* fm = NSFileManager.defaultManager;
  NSString* candidate = [dir stringByAppendingPathComponent:name];
  NSUInteger n = 1;
  while ([fm fileExistsAtPath:candidate]) {
    NSString* stem = ext.length ? [NSString stringWithFormat:@"%@ (%lu).%@",
                                                             base, (unsigned long)n, ext]
                                : [NSString stringWithFormat:@"%@ (%lu)", base,
                                                             (unsigned long)n];
    candidate = [dir stringByAppendingPathComponent:stem];
    n++;
  }
  return candidate;
}

// A Chrome extension package (.crx) the user downloaded. We install these into
// Mori rather than dropping them in ~/Downloads, where double-clicking would
// hand the extension off to whatever app owns the .crx type (usually Chrome).
bool IsCrxName(NSString* name) {
  return [name.pathExtension.lowercaseString isEqualToString:@"crx"];
}

// A package often arrives without a .crx filename — Google's update service 302s
// to an opaque blob — so also recognise it by the originating URL.
bool IsCrxURL(NSString* url) {
  if (url.length == 0) return false;
  if ([url.lowercaseString containsString:@"/service/update2/crx"]) return true;
  NSString* path = [NSURL URLWithString:url].path ?: @"";
  return [path.pathExtension.lowercaseString isEqualToString:@"crx"];
}

// The dedicated MIME type Chrome's web store / update service serves packages as.
bool IsCrxMimeType(NSString* mime) {
  return [mime.lowercaseString isEqualToString:@"application/x-chrome-extension"];
}

// Last-resort sniff once bytes are on disk: a CRX container starts with "Cr24".
bool FileHasCrxMagic(NSString* path) {
  NSFileHandle* fh = [NSFileHandle fileHandleForReadingAtPath:path];
  if (!fh) return false;
  NSData* head = [fh readDataOfLength:4];
  [fh closeFile];
  if (head.length < 4) return false;
  const uint8_t* b = (const uint8_t*)head.bytes;
  return b[0] == 'C' && b[1] == 'r' && b[2] == '2' && b[3] == '4';
}

// Recognise an extension package before its bytes land (name / URL / MIME), so
// it's installed into Mori instead of slipping into ~/Downloads where opening
// it would hand the .crx off to whatever app owns the type (usually Chrome).
bool IsCrxDownload(CefRefPtr<CefDownloadItem> item, NSString* suggested) {
  if (IsCrxName(suggested)) return true;
  if (!item) return false;
  if (IsCrxURL(@(item->GetURL().ToString().c_str()))) return true;
  return IsCrxMimeType(@(item->GetMimeType().ToString().c_str()));
}

// A throwaway location for a .crx mid-download; it's deleted once installed.
NSString* TempCrxPath(NSString* suggested) {
  NSString* dir = [NSTemporaryDirectory()
      stringByAppendingPathComponent:@"MoriExtensionDownloads"];
  [[NSFileManager defaultManager] createDirectoryAtPath:dir
                            withIntermediateDirectories:YES
                                             attributes:nil
                                                  error:nil];
  NSString* name = suggested.length ? suggested : @"extension.crx";
  // Keep a .crx extension even when the server suggested none, so the temp file
  // is unambiguous and the installer's id heuristics behave.
  if (![name.pathExtension.lowercaseString isEqualToString:@"crx"]) {
    name = [name stringByAppendingPathExtension:@"crx"];
  }
  return [dir stringByAppendingPathComponent:name];
}

void BroadcastDownload(CefRefPtr<CefDownloadItem> item, NSString* fullPath) {
  NSDictionary* info = @{
    @"id" : @(item->GetId()),
    @"url" : @(item->GetURL().ToString().c_str()),
    @"filename" : @(item->GetSuggestedFileName().ToString().c_str()),
    @"path" : fullPath ?: @"",
    @"received" : @(item->GetReceivedBytes()),
    @"total" : @(item->GetTotalBytes()),
    @"percent" : @(item->GetPercentComplete()),
    @"speed" : @(item->GetCurrentSpeed()),
    @"complete" : @(item->IsComplete()),
    @"canceled" : @(item->IsCanceled()),
    @"inProgress" : @(item->IsInProgress()),
  };
  // Already on the CEF UI thread == AppKit main thread.
  [NSNotificationCenter.defaultCenter postNotificationName:kMoriDownloadUpdated
                                                    object:nil
                                                  userInfo:info];
}

}  // namespace

bool MoriCancelDownload(uint32_t download_id) {
  auto& callbacks = DownloadCallbacks();
  auto it = callbacks.find(download_id);
  if (it == callbacks.end()) {
    return false;
  }
  it->second->Cancel();
  callbacks.erase(it);
  return true;
}

bool BrowserClient::OnBeforeDownload(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefDownloadItem> download_item,
    const CefString& suggested_name,
    CefRefPtr<CefBeforeDownloadCallback> callback) {
  CEF_REQUIRE_UI_THREAD();
  NSString* url = @(download_item->GetURL().ToString().c_str());
  NSDictionary* extensionRequest = TakeExtensionDownloadRequest(url);
  NSString* suggested = @(suggested_name.ToString().c_str());
  NSString* extensionFilename =
      [extensionRequest[@"filename"] isKindOfClass:NSString.class]
          ? extensionRequest[@"filename"]
          : nil;
  if (extensionFilename.length > 0) {
    suggested = extensionFilename.lastPathComponent;
  }

  // Chrome extension packages are routed to a temp file and installed on
  // completion (see OnDownloadUpdated) instead of landing in ~/Downloads.
  if (IsCrxDownload(download_item, suggested)) {
    NSString* target = TempCrxPath(suggested);
    NSLog(@"Mori CRX download begin id=%u url=%@ target=%@",
          download_item->GetId(), url, target);
    callback->Continue(CefString(target.UTF8String), /*show_dialog=*/false);
    NSMutableDictionary* targets = ExtensionCrxDownloadTargets();
    @synchronized(targets) {
      targets[@(download_item->GetId())] = target;
    }
    ResolveExtensionDownloadRequest(extensionRequest, @(download_item->GetId()));
    return true;
  }

  NSString* target = UniqueDownloadPath(suggested);
  // false → don't pop the OS save panel; we choose the path ourselves.
  callback->Continue(CefString(target.UTF8String), /*show_dialog=*/false);
  ResolveExtensionDownloadRequest(extensionRequest, @(download_item->GetId()));
  BroadcastDownload(download_item, target);
  return true;
}

void BrowserClient::OnDownloadUpdated(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefDownloadItem> download_item,
    CefRefPtr<CefDownloadItemCallback> callback) {
  CEF_REQUIRE_UI_THREAD();
  NSString* full = @(download_item->GetFullPath().ToString().c_str());
  uint32_t download_id = download_item->GetId();

  if (download_item->IsInProgress() && !download_item->IsCanceled()) {
    DownloadCallbacks()[download_id] = callback;
  } else {
    DownloadCallbacks().erase(download_id);
  }

  // A finished .crx is installed into Mori, not surfaced as a normal
  // download. Track handled ids so the (terminal) complete update installs once.
  NSMutableDictionary* crxTargets = ExtensionCrxDownloadTargets();
  NSString* crxTarget = nil;
  @synchronized(crxTargets) {
    crxTarget = crxTargets[@(download_id)];
    if ((download_item->IsComplete() || download_item->IsCanceled()) && crxTarget) {
      [crxTargets removeObjectForKey:@(download_id)];
    }
  }
  if (download_item->IsComplete() &&
      (crxTarget.length > 0 || IsCrxName(full) || FileHasCrxMagic(full))) {
    static NSMutableSet<NSNumber*>* handled = [NSMutableSet set];
    NSNumber* itemId = @(download_id);
    if ([handled containsObject:itemId]) return;
    [handled addObject:itemId];
    NSString* installPath = crxTarget.length > 0 ? crxTarget : full;
    NSLog(@"Mori CRX download complete id=%u full=%@ installPath=%@",
          download_id, full, installPath);
    [MoriExtensionBridge installCRXAtPath:installPath
                                fallbackURL:@(download_item->GetURL().ToString().c_str())];
    return;
  }

  BroadcastDownload(download_item, full);

  if (download_item->IsComplete()) {
    // Let the Dock bounce so a finished download is noticeable.
    [NSApp requestUserAttention:NSInformationalRequest];
  }
}

// --- CefJSDialogHandler -----------------------------------------------------

bool BrowserClient::OnJSDialog(CefRefPtr<CefBrowser> browser,
                               const CefString& origin_url,
                               JSDialogType dialog_type,
                               const CefString& message_text,
                               const CefString& default_prompt_text,
                               CefRefPtr<CefJSDialogCallback> callback,
                               bool& suppress_message) {
  CEF_REQUIRE_UI_THREAD();

  NSAlert* alert = [[NSAlert alloc] init];
  alert.messageText = @(message_text.ToString().c_str());
  alert.alertStyle = NSAlertStyleInformational;

  NSTextField* input = nil;
  switch (dialog_type) {
    case JSDIALOGTYPE_ALERT:
      [alert addButtonWithTitle:@"OK"];
      break;
    case JSDIALOGTYPE_CONFIRM:
      [alert addButtonWithTitle:@"OK"];
      [alert addButtonWithTitle:@"Cancel"];
      break;
    case JSDIALOGTYPE_PROMPT:
      [alert addButtonWithTitle:@"OK"];
      [alert addButtonWithTitle:@"Cancel"];
      input = [[NSTextField alloc] initWithFrame:NSMakeRect(0, 0, 280, 24)];
      input.stringValue = @(default_prompt_text.ToString().c_str());
      alert.accessoryView = input;
      break;
  }

  NSModalResponse response = [alert runModal];
  bool ok = (response == NSAlertFirstButtonReturn);
  CefString result;
  if (dialog_type == JSDIALOGTYPE_PROMPT && ok && input) {
    result = CefString(input.stringValue.UTF8String);
  }
  callback->Continue(ok, result);
  return true;  // We handled the dialog.
}

bool BrowserClient::OnBeforeUnloadDialog(
    CefRefPtr<CefBrowser> browser,
    const CefString& message_text,
    bool is_reload,
    CefRefPtr<CefJSDialogCallback> callback) {
  CEF_REQUIRE_UI_THREAD();
  NSAlert* alert = [[NSAlert alloc] init];
  alert.messageText = is_reload ? @"Reload this page?" : @"Leave this page?";
  alert.informativeText =
      @"Changes you made may not be saved.";
  [alert addButtonWithTitle:is_reload ? @"Reload" : @"Leave"];
  [alert addButtonWithTitle:@"Stay"];
  bool leave = ([alert runModal] == NSAlertFirstButtonReturn);
  callback->Continue(leave, CefString());
  return true;
}

// --- CefFindHandler ---------------------------------------------------------

void BrowserClient::OnFindResult(CefRefPtr<CefBrowser> browser,
                                 int identifier,
                                 int count,
                                 const CefRect& selectionRect,
                                 int activeMatchOrdinal,
                                 bool finalUpdate) {
  CEF_REQUIRE_UI_THREAD();
  if (delegate_) {
    delegate_->OnFindResult(count, activeMatchOrdinal);
  }
}

// --- CefKeyboardHandler -----------------------------------------------------

namespace {

NSEventModifierFlags MoriModifierMaskForCEF(uint32_t modifiers) {
  NSEventModifierFlags mask = 0;
  if (modifiers & EVENTFLAG_COMMAND_DOWN) mask |= NSEventModifierFlagCommand;
  if (modifiers & EVENTFLAG_CONTROL_DOWN) mask |= NSEventModifierFlagControl;
  if (modifiers & EVENTFLAG_SHIFT_DOWN) mask |= NSEventModifierFlagShift;
  if (modifiers & EVENTFLAG_ALT_DOWN) mask |= NSEventModifierFlagOption;
  return mask;
}

NSString* MoriStringFromCEFCharacter(char16_t character) {
  if (character == 0) return @"";
  unichar value = static_cast<unichar>(character);
  return [[NSString alloc] initWithCharacters:&value length:1];
}

NSString* MoriCharactersIgnoringModifiers(const CefKeyEvent& event) {
  if (event.unmodified_character != 0) {
    return MoriStringFromCEFCharacter(event.unmodified_character);
  }
  if (event.character != 0) {
    return MoriStringFromCEFCharacter(event.character);
  }
  return @"";
}

bool MoriIsShortcutKeyDown(const CefKeyEvent& event) {
  return event.type == KEYEVENT_RAWKEYDOWN || event.type == KEYEVENT_KEYDOWN;
}

bool MoriHandleCEFShortcutEvent(const CefKeyEvent& event,
                                CefEventHandle os_event) {
  if (!MoriIsShortcutKeyDown(event)) {
    return false;
  }

  if (os_event) {
    NSEvent* ns_event = (__bridge NSEvent*)os_event;
    if (ns_event.type == NSEventTypeKeyDown) {
      if ([MoriRoot handleShortcutEvent:ns_event]) {
        return true;
      }
    }
  }

  return [MoriRoot
      handleShortcutWithKeyCode:static_cast<uint16_t>(event.windows_key_code)
     charactersIgnoringModifiers:MoriCharactersIgnoringModifiers(event)
                    modifierMask:static_cast<NSUInteger>(
                                     MoriModifierMaskForCEF(event.modifiers))
                        isRepeat:(event.modifiers & EVENTFLAG_IS_REPEAT) != 0]
                ? true
                : false;
}

}  // namespace

bool BrowserClient::OnPreKeyEvent(CefRefPtr<CefBrowser> browser,
                                  const CefKeyEvent& event,
                                  CefEventHandle os_event,
                                  bool* is_keyboard_shortcut) {
  CEF_REQUIRE_UI_THREAD();
  // Only the initial key-down: key-up and the synthesized post-IME key event
  // would double-fire toggles. CEF can report either RAWKEYDOWN or KEYDOWN for
  // shortcut-shaped input; route both through the same registry.
  bool handled = MoriHandleCEFShortcutEvent(event, os_event);

  if (handled && is_keyboard_shortcut) {
    *is_keyboard_shortcut = true;
  }
  return handled;
}

bool BrowserClient::OnKeyEvent(CefRefPtr<CefBrowser> browser,
                               const CefKeyEvent& event,
                               CefEventHandle os_event) {
  CEF_REQUIRE_UI_THREAD();
  // Fallback for any shortcut that was not delivered through OnPreKeyEvent.
  // The registry dedupes the same physical press, so this is safe when CEF
  // calls both keyboard hooks for the same input.
  return MoriHandleCEFShortcutEvent(event, os_event);
}

// --- Script injection + console channel -------------------------------------

namespace {

NSString* const kMoriExtensionsCatalogKey = @"mori.extensions";
NSString* const kMoriExtensionCatalogEnvironmentKey =
    @"MORI_EXTENSION_CATALOG_JSON";

NSString* ExtensionStorageDefaultsKey(NSString* extensionId);
NSString* DNRDynamicRulesDefaultsKey(NSString* extensionId);
NSString* DNREnabledRulesetsDefaultsKey(NSString* extensionId);
NSString* ContextMenusDefaultsKey(NSString* extensionId);
NSString* PermissionsDefaultsKey(NSString* extensionId);

// Escape an arbitrary string into a JavaScript string literal (incl. quotes)
// by round-tripping through NSJSONSerialization, so it can be embedded in an
// ExecuteJavaScript call argument safely.
NSString* JSStringLiteral(NSString* s) {
  NSData* data = [NSJSONSerialization dataWithJSONObject:@[ s ?: @"" ]
                                                 options:0
                                                   error:nil];
  NSString* arr = data
      ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding]
      : @"[\"\"]";
  // Strip the surrounding [ ] to leave just the quoted, escaped string.
  return [arr substringWithRange:NSMakeRange(1, arr.length - 2)];
}

NSString* JSONStringLiteral(id object) {
  id safe = object ?: [NSNull null];
  if (![NSJSONSerialization isValidJSONObject:safe]) {
    safe = @{};
  }
  NSData* data = [NSJSONSerialization dataWithJSONObject:safe
                                                 options:0
                                                   error:nil];
  return data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding]
              : @"{}";
}

NSString* MoriHostBrowserVersion() {
  NSString* version = [NSBundle.mainBundle
      objectForInfoDictionaryKey:@"CFBundleShortVersionString"];
  return version.length ? version : @"0.1.0";
}

void BroadcastExtensionPortConnect(NSString* extensionID,
                                   NSString* portID,
                                   NSString* name,
                                   NSDictionary* sender,
                                   NSString* sourceURL,
                                   BOOL external) {
  if (extensionID.length == 0 || portID.length == 0) return;
  NSString* source = [NSString stringWithFormat:
      @"if(window.__moriExtDispatchConnect){"
       "window.__moriExtDispatchConnect(%@,%@,%@,%@,%@,%@);}",
      JSStringLiteral(extensionID), JSStringLiteral(portID),
      JSStringLiteral(name ?: @""), JSONStringLiteral(sender ?: @{}),
      JSStringLiteral(sourceURL ?: @""), external ? @"true" : @"false"];
  [MoriBrowserView broadcastExtensionJavaScript:source
                                  forExtensionID:extensionID];
}

void BroadcastExtensionPortMessage(NSString* extensionID,
                                   NSString* portID,
                                   id message,
                                   NSString* sourceURL) {
  if (extensionID.length == 0 || portID.length == 0) return;
  NSString* source = [NSString stringWithFormat:
      @"if(window.__moriExtDispatchPortMessage){"
       "window.__moriExtDispatchPortMessage(%@,%@,%@,%@);}",
      JSStringLiteral(extensionID), JSStringLiteral(portID),
      JSONStringLiteral(message ?: [NSNull null]),
      JSStringLiteral(sourceURL ?: @"")];
  [MoriBrowserView broadcastExtensionJavaScript:source
                                  forExtensionID:extensionID];
}

void BroadcastExtensionPortDisconnect(NSString* extensionID,
                                      NSString* portID,
                                      NSString* sourceURL) {
  if (extensionID.length == 0 || portID.length == 0) return;
  NSString* source = [NSString stringWithFormat:
      @"if(window.__moriExtDispatchPortDisconnect){"
       "window.__moriExtDispatchPortDisconnect(%@,%@,%@);}",
      JSStringLiteral(extensionID), JSStringLiteral(portID),
      JSStringLiteral(sourceURL ?: @"")];
  [MoriBrowserView broadcastExtensionJavaScript:source
                                  forExtensionID:extensionID];
}

void BroadcastExtensionNativePortMessage(NSString* extensionID,
                                         NSString* portID,
                                         id message) {
  if (extensionID.length == 0 || portID.length == 0) return;
  NSString* source = [NSString stringWithFormat:
      @"if(window.__moriExtDispatchNativePortMessage){"
       "window.__moriExtDispatchNativePortMessage(%@,%@,%@);}",
      JSStringLiteral(extensionID), JSStringLiteral(portID),
      JSONStringLiteral(message ?: [NSNull null])];
  [MoriBrowserView broadcastExtensionJavaScript:source
                                  forExtensionID:extensionID];
}

void BroadcastExtensionNativePortDisconnect(NSString* extensionID,
                                            NSString* portID) {
  if (extensionID.length == 0 || portID.length == 0) return;
  NSString* source = [NSString stringWithFormat:
      @"if(window.__moriExtDispatchNativePortDisconnect){"
       "window.__moriExtDispatchNativePortDisconnect(%@,%@);}",
      JSStringLiteral(extensionID), JSStringLiteral(portID)];
  [MoriBrowserView broadcastExtensionJavaScript:source
                                  forExtensionID:extensionID];
}

NSArray<NSDictionary*>* EnabledExtensionRecords() {
  NSData* data = nil;
  NSString* environmentCatalog =
      NSProcessInfo.processInfo.environment[kMoriExtensionCatalogEnvironmentKey];
  if (environmentCatalog.length > 0) {
    data = [environmentCatalog dataUsingEncoding:NSUTF8StringEncoding];
  } else {
    data = [[NSUserDefaults standardUserDefaults]
        dataForKey:kMoriExtensionsCatalogKey];
  }
  if (!data) return @[];
  id json = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
  if (![json isKindOfClass:[NSArray class]]) return @[];

  NSMutableArray<NSDictionary*>* out = [NSMutableArray array];
  for (id item in (NSArray*)json) {
    if (![item isKindOfClass:[NSDictionary class]]) continue;
    NSDictionary* dict = (NSDictionary*)item;
    if (![dict[@"enabled"] boolValue]) continue;
    NSString* path = [dict[@"path"] isKindOfClass:[NSString class]]
        ? dict[@"path"]
        : nil;
    if (path.length == 0) continue;
    [out addObject:dict];
  }
  return out;
}

NSDictionary* EnabledExtensionRecordForID(NSString* extensionID) {
  if (extensionID.length == 0) return nil;
  for (NSDictionary* ext in EnabledExtensionRecords()) {
    NSString* identifier = [ext[@"id"] isKindOfClass:[NSString class]]
        ? ext[@"id"]
        : nil;
    if ([identifier caseInsensitiveCompare:extensionID] == NSOrderedSame) {
      return ext;
    }
  }
  return nil;
}

NSDictionary* ManifestForExtension(NSDictionary* ext) {
  NSString* path = [ext[@"path"] isKindOfClass:[NSString class]]
      ? ext[@"path"]
      : nil;
  if (path.length == 0) return nil;
  NSString* manifestPath = [path stringByAppendingPathComponent:@"manifest.json"];
  NSData* data = [NSData dataWithContentsOfFile:manifestPath];
  if (!data) return nil;
  id json = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
  return [json isKindOfClass:[NSDictionary class]] ? (NSDictionary*)json : nil;
}

BOOL NativeMessagingHostNameIsSafe(NSString* hostName) {
  if (hostName.length == 0 || hostName.length > 255) return NO;
  NSCharacterSet* allowed = [NSCharacterSet
      characterSetWithCharactersInString:
          @"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-"];
  return [hostName rangeOfCharacterFromSet:allowed.invertedSet].location ==
         NSNotFound;
}

NSArray<NSString*>* NativeMessagingHostSearchDirectories() {
  NSMutableArray<NSString*>* dirs = [NSMutableArray array];
  NSString* override =
      NSProcessInfo.processInfo.environment[@"MORI_NATIVE_MESSAGING_HOSTS_DIR"];
  if (override.length > 0) {
    for (NSString* dir in [override componentsSeparatedByString:@":"]) {
      if (dir.length > 0) [dirs addObject:dir.stringByExpandingTildeInPath];
    }
  }
  NSString* home = NSHomeDirectory();
  NSArray<NSString*>* defaults = @[
    [home stringByAppendingPathComponent:
              @"Library/Application Support/Mori/NativeMessagingHosts"],
    [home stringByAppendingPathComponent:
              @"Library/Application Support/Google/Chrome/NativeMessagingHosts"],
    [home stringByAppendingPathComponent:
              @"Library/Application Support/Chromium/NativeMessagingHosts"],
    @"/Library/Google/Chrome/NativeMessagingHosts",
    @"/Library/Application Support/Chromium/NativeMessagingHosts"
  ];
  [dirs addObjectsFromArray:defaults];
  return dirs;
}

NSDictionary* NativeMessagingHostManifest(NSString* hostName) {
  if (!NativeMessagingHostNameIsSafe(hostName)) {
    return @{@"error" : @"Invalid native messaging host name."};
  }
  NSString* fileName = [hostName stringByAppendingPathExtension:@"json"];
  for (NSString* dir in NativeMessagingHostSearchDirectories()) {
    NSString* path = [dir stringByAppendingPathComponent:fileName];
    NSData* data = [NSData dataWithContentsOfFile:path];
    if (!data) continue;
    id json = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    if (![json isKindOfClass:[NSDictionary class]]) {
      return @{@"error" : @"Native messaging host manifest is invalid JSON."};
    }
    NSMutableDictionary* manifest = [(NSDictionary*)json mutableCopy];
    manifest[@"__manifestPath"] = path;
    return manifest;
  }
  return @{@"error" : @"Native messaging host was not found."};
}

NSArray<NSString*>* NativeMessagingOriginCandidates(NSString* extensionID) {
  NSString* rawID = extensionID ?: @"";
  NSString* lowerID = rawID.lowercaseString ?: rawID;
  NSString* chromeExtensionScheme =
      [@"chrome" stringByAppendingString:@"-extension"];
  NSMutableOrderedSet<NSString*>* origins = [NSMutableOrderedSet orderedSet];
  for (NSString* scheme in @[
         @(mori::kExtensionScheme),
         chromeExtensionScheme
       ]) {
    if (rawID.length > 0) {
      [origins addObject:[NSString stringWithFormat:@"%@://%@/", scheme, rawID]];
    }
    if (lowerID.length > 0 && ![lowerID isEqualToString:rawID]) {
      [origins addObject:[NSString stringWithFormat:@"%@://%@/", scheme, lowerID]];
    }
  }
  return origins.array;
}

NSString* NativeMessagingHostAllowedOrigin(NSDictionary* manifest,
                                           NSString* extensionID) {
  NSArray* origins = [manifest[@"allowed_origins"] isKindOfClass:[NSArray class]]
      ? manifest[@"allowed_origins"]
      : @[];
  NSArray<NSString*>* candidates = NativeMessagingOriginCandidates(extensionID);
  for (id origin in origins) {
    if (![origin isKindOfClass:[NSString class]]) continue;
    for (NSString* candidate in candidates) {
      if ([origin isEqualToString:candidate]) {
        return candidate;
      }
    }
  }
  return nil;
}

NSDictionary* NativeMessagingValidatedHost(NSString* extensionID,
                                           NSString* hostName) {
  NSDictionary* manifest = NativeMessagingHostManifest(hostName);
  NSString* manifestError = [manifest[@"error"] isKindOfClass:[NSString class]]
      ? manifest[@"error"]
      : nil;
  if (manifestError.length > 0) return @{@"error" : manifestError};

  NSString* allowedOrigin = NativeMessagingHostAllowedOrigin(manifest, extensionID);
  if (allowedOrigin.length == 0) {
    return @{@"error" : @"Native messaging host does not allow this extension."};
  }
  NSString* hostPath = [manifest[@"path"] isKindOfClass:[NSString class]]
      ? manifest[@"path"]
      : @"";
  if (hostPath.length == 0 || !hostPath.isAbsolutePath) {
    return @{@"error" : @"Native messaging host path must be absolute."};
  }
  if (![NSFileManager.defaultManager isExecutableFileAtPath:hostPath]) {
    return @{@"error" : @"Native messaging host is not executable."};
  }
  return @{@"path" : hostPath, @"origin" : allowedOrigin};
}

BOOL NativeMessagingFrame(id message, NSMutableData** framed, NSString** error) {
  if (![NSJSONSerialization isValidJSONObject:message]) {
    if (error) *error = @"Native messaging payload must be JSON serializable.";
    return NO;
  }
  NSError* jsonError = nil;
  NSData* payload = [NSJSONSerialization dataWithJSONObject:message
                                                    options:0
                                                      error:&jsonError];
  if (!payload || jsonError) {
    if (error) *error = @"Native messaging payload could not be encoded.";
    return NO;
  }
  if (payload.length > 64 * 1024 * 1024) {
    if (error) *error = @"Native messaging payload exceeds 64MB.";
    return NO;
  }
  uint32_t payloadLength = CFSwapInt32HostToLittle((uint32_t)payload.length);
  NSMutableData* out =
      [NSMutableData dataWithBytes:&payloadLength length:sizeof(payloadLength)];
  [out appendData:payload];
  if (framed) *framed = out;
  return YES;
}

id NativeMessagingReadMessage(NSFileHandle* output,
                              NSFileHandle* errorOutput,
                              NSTask* task,
                              NSString** error) {
  NSData* lengthData = [output readDataOfLength:4];
  if (lengthData.length != 4) {
    if (task) [task waitUntilExit];
    NSData* errorData = [errorOutput readDataToEndOfFile];
    NSString* stderrText =
        [[NSString alloc] initWithData:errorData encoding:NSUTF8StringEncoding];
    if (error) {
      *error = stderrText.length ? stderrText : @"Native messaging host did not respond.";
    }
    return nil;
  }

  uint32_t rawResponseLength = 0;
  std::memcpy(&rawResponseLength, lengthData.bytes, sizeof(rawResponseLength));
  uint32_t responseLength = CFSwapInt32LittleToHost(rawResponseLength);
  if (responseLength > 1024 * 1024) {
    if (task) [task terminate];
    if (error) *error = @"Native messaging response exceeds 1MB.";
    return nil;
  }
  NSData* responseData = [output readDataOfLength:responseLength];
  if (responseData.length != responseLength) {
    if (error) *error = @"Native messaging host returned a truncated response.";
    return nil;
  }
  id responseObject =
      [NSJSONSerialization JSONObjectWithData:responseData options:0 error:nil];
  if (!responseObject) {
    if (error) *error = @"Native messaging host returned invalid JSON.";
    return nil;
  }
  return responseObject;
}

NSMutableDictionary<NSString*, NSMutableDictionary*>* NativeMessagingPorts() {
  static NSMutableDictionary<NSString*, NSMutableDictionary*>* ports =
      [NSMutableDictionary dictionary];
  return ports;
}

NSDictionary* StartNativeMessagingPort(NSString* extensionID,
                                       NSDictionary* args) {
  NSString* hostName = [args[@"hostName"] isKindOfClass:[NSString class]]
      ? args[@"hostName"]
      : @"";
  NSString* portID = [args[@"portId"] isKindOfClass:[NSString class]]
      ? args[@"portId"]
      : @"";
  if (portID.length == 0) return @{@"error" : @"Missing native messaging port id."};

  NSDictionary* host = NativeMessagingValidatedHost(extensionID, hostName);
  NSString* hostError = [host[@"error"] isKindOfClass:[NSString class]]
      ? host[@"error"]
      : nil;
  if (hostError.length > 0) return @{@"error" : hostError};
  NSString* hostPath = host[@"path"];
  NSString* hostOrigin = [host[@"origin"] isKindOfClass:[NSString class]]
      ? host[@"origin"]
      : [NSString stringWithFormat:@"%s://%@/",
                                   mori::kExtensionScheme,
                                   extensionID ?: @""];

  NSPipe* stdinPipe = [NSPipe pipe];
  NSPipe* stdoutPipe = [NSPipe pipe];
  NSPipe* stderrPipe = [NSPipe pipe];
  NSTask* task = [[NSTask alloc] init];
  task.executableURL = [NSURL fileURLWithPath:hostPath];
  task.currentDirectoryURL =
      [NSURL fileURLWithPath:hostPath.stringByDeletingLastPathComponent];
  task.arguments = @[ hostOrigin ];
  task.standardInput = stdinPipe;
  task.standardOutput = stdoutPipe;
  task.standardError = stderrPipe;

  NSError* launchError = nil;
  if (![task launchAndReturnError:&launchError]) {
    return @{@"error" : launchError.localizedDescription ?: @"Native messaging host failed to launch."};
  }

  NSMutableDictionary* ports = NativeMessagingPorts();
  @synchronized(ports) {
    ports[portID] = [@{
      @"task" : task,
      @"stdin" : stdinPipe.fileHandleForWriting,
      @"stdout" : stdoutPipe.fileHandleForReading,
      @"stderr" : stderrPipe.fileHandleForReading,
      @"extensionId" : extensionID ?: @""
    } mutableCopy];
  }
  __block NSString* capturedExtensionID = [extensionID copy];
  __block NSString* capturedPortID = [portID copy];
  task.terminationHandler = ^(NSTask* finishedTask) {
    NSMutableDictionary* livePorts = NativeMessagingPorts();
    @synchronized(livePorts) {
      [livePorts removeObjectForKey:capturedPortID];
    }
    dispatch_async(dispatch_get_main_queue(), ^{
      BroadcastExtensionNativePortDisconnect(capturedExtensionID, capturedPortID);
    });
  };
  return @{@"result" : @{}};
}

NSDictionary* NativeMessagingPortPostMessage(NSString* extensionID,
                                             NSDictionary* args) {
  NSString* portID = [args[@"portId"] isKindOfClass:[NSString class]]
      ? args[@"portId"]
      : @"";
  id message = args[@"message"] ?: @{};
  NSMutableDictionary* ports = NativeMessagingPorts();
  NSDictionary* port = nil;
  @synchronized(ports) {
    port = [ports[portID] copy];
  }
  if (!port) return @{@"error" : @"Native messaging port is not connected."};
  NSString* owner = [port[@"extensionId"] isKindOfClass:[NSString class]]
      ? port[@"extensionId"]
      : @"";
  if (![owner isEqualToString:extensionID]) {
    return @{@"error" : @"Native messaging port belongs to another extension."};
  }

  NSMutableData* framed = nil;
  NSString* frameError = nil;
  if (!NativeMessagingFrame(message, &framed, &frameError)) {
    return @{@"error" : frameError ?: @"Native messaging payload failed."};
  }
  @try {
    [(NSFileHandle*)port[@"stdin"] writeData:framed];
  } @catch (NSException* exception) {
    return @{@"error" : @"Native messaging host closed stdin."};
  }
  NSString* readError = nil;
  id responseObject = NativeMessagingReadMessage(
      port[@"stdout"], port[@"stderr"], nil, &readError);
  if (!responseObject) {
    return @{@"error" : readError ?: @"Native messaging host did not respond."};
  }
  BroadcastExtensionNativePortMessage(extensionID, portID, responseObject);
  return @{@"result" : @{}};
}

NSDictionary* DisconnectNativeMessagingPort(NSString* extensionID,
                                            NSDictionary* args) {
  NSString* portID = [args[@"portId"] isKindOfClass:[NSString class]]
      ? args[@"portId"]
      : @"";
  NSMutableDictionary* ports = NativeMessagingPorts();
  NSDictionary* port = nil;
  @synchronized(ports) {
    port = ports[portID];
    [ports removeObjectForKey:portID];
  }
  if (!port) return @{@"result" : @{}};
  NSString* owner = [port[@"extensionId"] isKindOfClass:[NSString class]]
      ? port[@"extensionId"]
      : @"";
  if (![owner isEqualToString:extensionID]) {
    return @{@"error" : @"Native messaging port belongs to another extension."};
  }
  @try {
    [(NSFileHandle*)port[@"stdin"] closeFile];
  } @catch (NSException* exception) {
  }
  NSTask* task = port[@"task"];
  if (task.running) [task terminate];
  BroadcastExtensionNativePortDisconnect(extensionID, portID);
  return @{@"result" : @{}};
}

NSDictionary* HandleNativeMessagingSend(NSString* extensionID,
                                        NSDictionary* args) {
  NSString* hostName = [args[@"hostName"] isKindOfClass:[NSString class]]
      ? args[@"hostName"]
      : @"";
  id message = args[@"message"] ?: @{};
  NSDictionary* host = NativeMessagingValidatedHost(extensionID, hostName);
  NSString* hostError = [host[@"error"] isKindOfClass:[NSString class]]
      ? host[@"error"]
      : nil;
  if (hostError.length > 0) return @{@"error" : hostError};
  NSString* hostPath = host[@"path"];
  NSString* hostOrigin = [host[@"origin"] isKindOfClass:[NSString class]]
      ? host[@"origin"]
      : [NSString stringWithFormat:@"%s://%@/",
                                   mori::kExtensionScheme,
                                   extensionID ?: @""];

  NSMutableData* framed = nil;
  NSString* frameError = nil;
  if (!NativeMessagingFrame(message, &framed, &frameError)) {
    return @{@"error" : frameError ?: @"Native messaging payload failed."};
  }

  NSPipe* stdinPipe = [NSPipe pipe];
  NSPipe* stdoutPipe = [NSPipe pipe];
  NSPipe* stderrPipe = [NSPipe pipe];
  NSTask* task = [[NSTask alloc] init];
  task.executableURL = [NSURL fileURLWithPath:hostPath];
  task.currentDirectoryURL =
      [NSURL fileURLWithPath:hostPath.stringByDeletingLastPathComponent];
  task.arguments = @[ hostOrigin ];
  task.standardInput = stdinPipe;
  task.standardOutput = stdoutPipe;
  task.standardError = stderrPipe;

  NSError* launchError = nil;
  if (![task launchAndReturnError:&launchError]) {
    return @{@"error" : launchError.localizedDescription ?: @"Native messaging host failed to launch."};
  }

  @try {
    [stdinPipe.fileHandleForWriting writeData:framed];
    [stdinPipe.fileHandleForWriting closeFile];
  } @catch (NSException* exception) {
    [task terminate];
    return @{@"error" : @"Native messaging host closed stdin."};
  }

  NSString* readError = nil;
  id responseObject = NativeMessagingReadMessage(stdoutPipe.fileHandleForReading,
                                                stderrPipe.fileHandleForReading,
                                                task,
                                                &readError);
  [task waitUntilExit];
  if (!responseObject) {
    return @{@"error" : readError ?: @"Native messaging host did not respond."};
  }
  return @{@"result" : responseObject};
}

NSString* ExtensionRootPath(NSDictionary* ext) {
  NSString* path = [ext[@"path"] isKindOfClass:[NSString class]]
      ? ext[@"path"]
      : nil;
  if (path.length == 0) return nil;
  return path.stringByStandardizingPath.stringByResolvingSymlinksInPath;
}

NSString* ExtensionFileText(NSDictionary* ext, NSString* relativePath) {
  if (relativePath.length == 0) return nil;
  NSString* root = ExtensionRootPath(ext);
  if (root.length == 0) return nil;
  NSString* full = [[root stringByAppendingPathComponent:relativePath]
      stringByStandardizingPath].stringByResolvingSymlinksInPath;
  if (![full hasPrefix:[root stringByAppendingString:@"/"]]) return nil;
  return [NSString stringWithContentsOfFile:full
                                   encoding:NSUTF8StringEncoding
                                      error:nil];
}

void AddLocaleCandidate(NSMutableArray<NSString*>* candidates, NSString* locale) {
  if (locale.length == 0) return;
  NSString* normalized = [locale stringByReplacingOccurrencesOfString:@"-"
                                                           withString:@"_"];
  if ([normalized rangeOfString:@"/"].location != NSNotFound ||
      [normalized rangeOfString:@".."].location != NSNotFound) {
    return;
  }
  for (NSString* existing in candidates) {
    if ([existing caseInsensitiveCompare:normalized] == NSOrderedSame) return;
  }
  [candidates addObject:normalized];
}

NSArray<NSString*>* LocaleCandidates(NSDictionary* manifest) {
  NSMutableArray<NSString*>* candidates = [NSMutableArray array];
  for (NSString* language in [NSLocale preferredLanguages]) {
    AddLocaleCandidate(candidates, language);
    NSArray<NSString*>* parts =
        [[language stringByReplacingOccurrencesOfString:@"-"
                                             withString:@"_"]
            componentsSeparatedByString:@"_"];
    if (parts.count > 0) AddLocaleCandidate(candidates, parts.firstObject);
  }
  AddLocaleCandidate(candidates,
                     [manifest[@"default_locale"] isKindOfClass:[NSString class]]
                         ? manifest[@"default_locale"]
                         : nil);
  AddLocaleCandidate(candidates, @"en");
  return candidates;
}

NSDictionary* ExtensionMessagesForLocale(NSDictionary* ext, NSString* locale) {
  NSString* root = ExtensionRootPath(ext);
  if (root.length == 0 || locale.length == 0) return nil;
  NSString* path = [[[root stringByAppendingPathComponent:@"_locales"]
      stringByAppendingPathComponent:locale] stringByAppendingPathComponent:@"messages.json"];
  NSData* data = [NSData dataWithContentsOfFile:path];
  if (!data) return nil;
  id json = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
  if (![json isKindOfClass:[NSDictionary class]]) return nil;

  NSMutableDictionary* messages = [NSMutableDictionary dictionary];
  for (id key in (NSDictionary*)json) {
    if (![key isKindOfClass:[NSString class]]) continue;
    id value = ((NSDictionary*)json)[key];
    if (![value isKindOfClass:[NSDictionary class]]) continue;
    NSString* message = [value[@"message"] isKindOfClass:[NSString class]]
        ? value[@"message"]
        : nil;
    if (message.length == 0) continue;
    NSMutableDictionary* entry = [@{ @"message" : message } mutableCopy];
    if ([value[@"placeholders"] isKindOfClass:[NSDictionary class]]) {
      entry[@"placeholders"] = value[@"placeholders"];
    }
    messages[((NSString*)key).lowercaseString] = entry;
  }
  return messages.count > 0 ? messages : nil;
}

NSDictionary* ExtensionI18nBundle(NSDictionary* ext, NSDictionary* manifest) {
  for (NSString* locale in LocaleCandidates(manifest ?: @{})) {
    NSDictionary* messages = ExtensionMessagesForLocale(ext, locale);
    if (messages) {
      return @{ @"locale" : locale, @"messages" : messages };
    }
  }
  return @{ @"locale" : @"en", @"messages" : @{} };
}

id LocalizedManifestValue(id value, NSDictionary* messages) {
  if ([value isKindOfClass:[NSString class]]) {
    NSString* raw = (NSString*)value;
    if ([raw hasPrefix:@"__MSG_"] && [raw hasSuffix:@"__"]) {
      NSString* key =
          [[raw substringWithRange:NSMakeRange(6, raw.length - 8)] lowercaseString];
      NSDictionary* entry = [messages[key] isKindOfClass:[NSDictionary class]]
          ? messages[key]
          : nil;
      NSString* message = [entry[@"message"] isKindOfClass:[NSString class]]
          ? entry[@"message"]
          : nil;
      return message ?: raw;
    }
    return raw;
  }
  if ([value isKindOfClass:[NSArray class]]) {
    NSMutableArray* out = [NSMutableArray array];
    for (id item in (NSArray*)value) {
      [out addObject:LocalizedManifestValue(item, messages) ?: [NSNull null]];
    }
    return out;
  }
  if ([value isKindOfClass:[NSDictionary class]]) {
    NSMutableDictionary* out = [NSMutableDictionary dictionary];
    for (id key in (NSDictionary*)value) {
      id localized = LocalizedManifestValue(((NSDictionary*)value)[key], messages);
      if (localized) out[key] = localized;
    }
    return out;
  }
  return value;
}

BOOL WildcardMatch(NSString* pattern, NSString* value) {
  if ([pattern isEqualToString:@"*"]) return YES;
  NSString* quoted = [NSRegularExpression escapedPatternForString:pattern];
  NSString* regex = [@"^" stringByAppendingString:
      [[quoted stringByReplacingOccurrencesOfString:@"\\*"
                                         withString:@".*"]
          stringByAppendingString:@"$"]];
  return [value rangeOfString:regex options:NSRegularExpressionSearch].location !=
         NSNotFound;
}

BOOL MatchExtensionPattern(NSString* pattern, NSURL* url) {
  if ([pattern isEqualToString:@"<all_urls>"]) {
    return url.scheme.length > 0 &&
           [@[@"http", @"https", @"file"] containsObject:url.scheme.lowercaseString];
  }

  NSRange schemeSep = [pattern rangeOfString:@"://"];
  if (schemeSep.location == NSNotFound) return NO;
  NSString* schemePattern = [pattern substringToIndex:schemeSep.location];
  NSString* rest = [pattern substringFromIndex:NSMaxRange(schemeSep)];
  NSRange slash = [rest rangeOfString:@"/"];
  NSString* hostPattern =
      slash.location == NSNotFound ? rest : [rest substringToIndex:slash.location];
  NSString* pathPattern =
      slash.location == NSNotFound ? @"/*" : [rest substringFromIndex:slash.location];

  NSString* scheme = url.scheme.lowercaseString ?: @"";
  NSString* host = url.host.lowercaseString ?: @"";
  NSString* path = url.path.length ? url.path : @"/";

  if (![schemePattern isEqualToString:@"*"] &&
      ![schemePattern.lowercaseString isEqualToString:scheme]) {
    return NO;
  }
  if ([hostPattern hasPrefix:@"*."]) {
    NSString* suffix = [hostPattern substringFromIndex:1].lowercaseString;
    if (![host hasSuffix:suffix] &&
        ![host isEqualToString:[hostPattern substringFromIndex:2].lowercaseString]) {
      return NO;
    }
  } else if (!WildcardMatch(hostPattern.lowercaseString, host)) {
    return NO;
  }
  return WildcardMatch(pathPattern, path);
}

BOOL ScriptMatchesURL(NSDictionary* script, NSURL* url) {
  NSArray* matches = [script[@"matches"] isKindOfClass:[NSArray class]]
      ? script[@"matches"]
      : nil;
  if (matches.count == 0) return NO;
  BOOL included = NO;
  for (id item in matches) {
    if ([item isKindOfClass:[NSString class]] &&
        MatchExtensionPattern((NSString*)item, url)) {
      included = YES;
      break;
    }
  }
  if (!included) return NO;

  NSArray* excludes = [script[@"exclude_matches"] isKindOfClass:[NSArray class]]
      ? script[@"exclude_matches"]
      : ([script[@"excludeMatches"] isKindOfClass:[NSArray class]]
             ? script[@"excludeMatches"]
             : nil);
  for (id item in excludes) {
    if ([item isKindOfClass:[NSString class]] &&
        MatchExtensionPattern((NSString*)item, url)) {
      return NO;
    }
  }
  return YES;
}

BOOL ExtensionHostPermissionsAllow(NSDictionary* manifest, NSURL* url) {
  if (!manifest || !url) return NO;
  NSString* scheme = url.scheme.lowercaseString ?: @"";
  if (![@[@"http", @"https", @"file"] containsObject:scheme]) return NO;

  NSMutableArray* patterns = [NSMutableArray array];
  NSArray* hostPermissions =
      [manifest[@"host_permissions"] isKindOfClass:NSArray.class]
          ? manifest[@"host_permissions"]
          : @[];
  NSArray* optionalHostPermissions =
      [manifest[@"optional_host_permissions"] isKindOfClass:NSArray.class]
          ? manifest[@"optional_host_permissions"]
          : @[];
  NSArray* permissions = [manifest[@"permissions"] isKindOfClass:NSArray.class]
      ? manifest[@"permissions"]
      : @[];
  [patterns addObjectsFromArray:hostPermissions];
  [patterns addObjectsFromArray:optionalHostPermissions];
  [patterns addObjectsFromArray:permissions];

  for (id item in patterns) {
    if (![item isKindOfClass:NSString.class]) continue;
    NSString* pattern = (NSString*)item;
    if ([pattern isEqualToString:@"<all_urls>"] ||
        [pattern rangeOfString:@"://"].location != NSNotFound) {
      if (MatchExtensionPattern(pattern, url)) return YES;
    }
  }
  return NO;
}

// True when `sourceURL` (the origin of a runtime.sendMessage / runtime.connect
// sender) is a web page allowed to talk to extension `extensionID` through its
// manifest `externally_connectable.matches`. Such messages are external (the
// page is not the extension), so they must be delivered to onMessageExternal /
// onConnectExternal with a sender that carries url/origin but NO id — Chrome
// extensions (e.g. Proton Pass) tell internal from external by comparing
// sender.id against runtime.id, and the account.proton.me sign-in "fork" is
// only honored when it arrives externally.
BOOL ExtensionAllowsExternalConnect(NSString* extensionID, NSString* sourceURL) {
  if (extensionID.length == 0 || sourceURL.length == 0) return NO;
  NSURL* url = [NSURL URLWithString:sourceURL];
  if (!url) return NO;
  // Only real web origins connect externally; extension pages are internal.
  NSString* scheme = url.scheme.lowercaseString;
  if (![scheme isEqualToString:@"https"] && ![scheme isEqualToString:@"http"]) {
    return NO;
  }
  NSDictionary* ext = EnabledExtensionRecordForID(extensionID);
  if (!ext) return NO;
  NSDictionary* manifest = ManifestForExtension(ext);
  NSDictionary* ec = [manifest[@"externally_connectable"] isKindOfClass:[NSDictionary class]]
      ? manifest[@"externally_connectable"]
      : nil;
  NSArray* matches = [ec[@"matches"] isKindOfClass:[NSArray class]]
      ? ec[@"matches"]
      : nil;
  for (id item in matches) {
    if ([item isKindOfClass:[NSString class]] &&
        MatchExtensionPattern((NSString*)item, url)) {
      return YES;
    }
  }
  return NO;
}

BOOL ExtensionMessageHasType(id message, NSString* type) {
  if (![message isKindOfClass:[NSDictionary class]] || type.length == 0) {
    return NO;
  }
  id rawType = ((NSDictionary*)message)[@"type"];
  return [rawType isKindOfClass:[NSString class]] &&
         [(NSString*)rawType isEqualToString:type];
}

BOOL ExtensionMessageIsExternallyConnectableAccountType(id message) {
  return ExtensionMessageHasType(message, @"auth-ext") ||
         ExtensionMessageHasType(message, @"fork") ||
         ExtensionMessageHasType(message, @"pass-onboarding") ||
         ExtensionMessageHasType(message, @"pass-installed");
}

NSDictionary* ExtensionSenderTab(int tabID, NSString* sourceURL) {
  if (tabID < 0) return nil;
  NSMutableDictionary* tab = [@{
    @"id" : @(tabID),
    @"windowId" : @1,
    @"index" : @(-1),
    @"active" : @YES,
    @"highlighted" : @YES,
    @"selected" : @YES,
    @"pinned" : @NO,
    @"incognito" : @NO,
    @"status" : @"complete"
  } mutableCopy];
  if (sourceURL.length > 0) {
    tab[@"url"] = sourceURL;
  }
  return tab;
}

NSString* DNRResourceType(CefRefPtr<CefRequest> request) {
  if (!request) return @"other";
  switch (request->GetResourceType()) {
    case RT_MAIN_FRAME:
      return @"main_frame";
    case RT_SUB_FRAME:
      return @"sub_frame";
    case RT_STYLESHEET:
      return @"stylesheet";
    case RT_SCRIPT:
      return @"script";
    case RT_IMAGE:
      return @"image";
    case RT_FONT_RESOURCE:
      return @"font";
    case RT_XHR:
      return @"xmlhttprequest";
    case RT_MEDIA:
      return @"media";
    case RT_PING:
      return @"ping";
    default:
      return @"other";
  }
}

void DispatchExtensionEventOnMain(NSString* eventName,
                                  NSArray* args,
                                  NSString* extensionID = nil) {
  NSString* name = [eventName copy];
  NSArray* eventArgs = [args copy] ?: @[];
  NSString* targetExtensionID = [extensionID copy];
  dispatch_async(dispatch_get_main_queue(), ^{
    [MoriBrowserView dispatchExtensionEvent:name
                                         args:eventArgs
                               forExtensionID:targetExtensionID];
  });
}

void DispatchWebNavigationConsoleEvent(NSString* message, int tabID) {
  NSData* data = [message dataUsingEncoding:NSUTF8StringEncoding];
  id parsed = data ? [NSJSONSerialization JSONObjectWithData:data
                                                     options:0
                                                       error:nil]
                   : nil;
  if (![parsed isKindOfClass:[NSDictionary class]]) return;
  NSDictionary* payload = (NSDictionary*)parsed;
  NSString* eventName = [payload[@"event"] isKindOfClass:NSString.class]
      ? payload[@"event"]
      : @"";
  if (![eventName isEqualToString:@"webNavigation.onHistoryStateUpdated"] &&
      ![eventName isEqualToString:@"webNavigation.onReferenceFragmentUpdated"]) {
    return;
  }
  NSString* url = [payload[@"url"] isKindOfClass:NSString.class]
      ? payload[@"url"]
      : @"";
  if (url.length == 0) return;
  NSDictionary* details = @{
    @"tabId" : @(tabID),
    @"url" : url,
    @"frameId" : @0,
    @"parentFrameId" : @-1,
    @"timeStamp" : @([[NSDate date] timeIntervalSince1970] * 1000.0)
  };
  DispatchExtensionEventOnMain(eventName, @[ details ]);
}

NSDictionary* WebRequestDetails(CefRefPtr<CefFrame> frame,
                                CefRefPtr<CefRequest> request,
                                int tabID) {
  if (!request) return @{};
  NSMutableDictionary* details = [NSMutableDictionary dictionary];
  details[@"requestId"] =
      [NSString stringWithFormat:@"%llu",
                                 static_cast<unsigned long long>(
                                     request->GetIdentifier())];
  details[@"url"] = @(request->GetURL().ToString().c_str());
  details[@"method"] = @(request->GetMethod().ToString().c_str());
  details[@"type"] = DNRResourceType(request);
  details[@"tabId"] = @(tabID);
  details[@"timeStamp"] = @([[NSDate date] timeIntervalSince1970] * 1000.0);
  if (frame && frame->IsValid()) {
    details[@"frameId"] = frame->IsMain() ? @0 : @1;
    details[@"parentFrameId"] = frame->IsMain() ? @-1 : @0;
    std::string frameURL = frame->GetURL().ToString();
    if (!frameURL.empty()) {
      details[@"documentUrl"] = @(frameURL.c_str());
    }
  } else {
    details[@"frameId"] = @-1;
    details[@"parentFrameId"] = @-1;
  }
  return details;
}

NSArray<NSDictionary*>* HeaderArrayFromMap(
    const std::multimap<CefString, CefString>& headerMap) {
  NSMutableArray<NSDictionary*>* headers = [NSMutableArray array];
  for (const auto& entry : headerMap) {
    NSString* name = @(entry.first.ToString().c_str());
    NSString* value = @(entry.second.ToString().c_str());
    if (name.length == 0) continue;
    [headers addObject:@{
      @"name" : name,
      @"value" : value ?: @""
    }];
  }
  return headers;
}

NSArray<NSDictionary*>* RequestHeaders(CefRefPtr<CefRequest> request) {
  CefRequest::HeaderMap headerMap;
  if (request) request->GetHeaderMap(headerMap);
  return HeaderArrayFromMap(headerMap);
}

NSArray<NSDictionary*>* ResponseHeaders(CefRefPtr<CefResponse> response) {
  CefResponse::HeaderMap headerMap;
  if (response) response->GetHeaderMap(headerMap);
  return HeaderArrayFromMap(headerMap);
}

NSDictionary* WebAuthRequestDetails(const CefString& originURL,
                                    bool isProxy,
                                    const CefString& host,
                                    int port,
                                    const CefString& realm,
                                    const CefString& scheme,
                                    int tabID) {
  NSString* url = @(originURL.ToString().c_str());
  NSString* challengeHost = @(host.ToString().c_str());
  NSString* challengeRealm = @(realm.ToString().c_str());
  NSString* challengeScheme = @(scheme.ToString().c_str());
  NSTimeInterval timestamp = [[NSDate date] timeIntervalSince1970] * 1000.0;

  NSMutableDictionary* details = [NSMutableDictionary dictionary];
  details[@"requestId"] =
      [NSString stringWithFormat:@"auth:%@:%@:%d:%0.f",
                                 url ?: @"",
                                 challengeHost ?: @"",
                                 port,
                                 timestamp];
  details[@"url"] = url.length > 0 ? url : @"";
  details[@"method"] = @"GET";
  details[@"type"] = @"main_frame";
  details[@"tabId"] = @(tabID);
  details[@"timeStamp"] = @(timestamp);
  details[@"frameId"] = @-1;
  details[@"parentFrameId"] = @-1;
  details[@"statusCode"] = isProxy ? @407 : @401;
  details[@"isProxy"] = @(isProxy);
  details[@"challenger"] = @{
    @"host" : challengeHost.length > 0 ? challengeHost : @"",
    @"port" : @(port)
  };
  if (challengeRealm.length > 0) details[@"realm"] = challengeRealm;
  if (challengeScheme.length > 0) details[@"scheme"] = challengeScheme;
  return details;
}

void DispatchWebRequestEvent(NSString* eventName,
                             NSDictionary* details,
                             NSString* error = nil,
                             NSNumber* statusCode = nil) {
  NSMutableDictionary* payload = [details mutableCopy] ?: [NSMutableDictionary dictionary];
  if (error.length > 0) {
    payload[@"error"] = error;
  }
  if (statusCode) {
    payload[@"statusCode"] = statusCode;
  }
  DispatchExtensionEventOnMain(eventName, @[ payload ]);
}

BOOL DNRArrayContainsString(NSArray* values, NSString* value) {
  if (value.length == 0) return NO;
  for (id item in values) {
    if ([item isKindOfClass:NSString.class] &&
        [((NSString*)item) caseInsensitiveCompare:value] == NSOrderedSame) {
      return YES;
    }
  }
  return NO;
}

BOOL DNRDomainMatches(NSString* pattern, NSString* host) {
  if (pattern.length == 0 || host.length == 0) return NO;
  NSString* p = pattern.lowercaseString;
  NSString* h = host.lowercaseString;
  return [h isEqualToString:p] || [h hasSuffix:[@"." stringByAppendingString:p]];
}

BOOL DNRDomainListMatches(NSArray* domains, NSString* host) {
  if (domains.count == 0) return NO;
  for (id item in domains) {
    if ([item isKindOfClass:NSString.class] &&
        DNRDomainMatches((NSString*)item, host)) {
      return YES;
    }
  }
  return NO;
}

NSString* MoriAdBlockDomainFromLine(NSString* rawLine) {
  NSString* line = [rawLine
      stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
  if (line.length == 0 || [line hasPrefix:@"#"] || [line hasPrefix:@"!"]) {
    return nil;
  }

  NSRange inlineComment = [line rangeOfString:@" #"];
  if (inlineComment.location != NSNotFound) {
    line = [[line substringToIndex:inlineComment.location]
        stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
  }

  if ([line hasPrefix:@"||"]) {
    line = [line substringFromIndex:2];
    NSRange separator = [line rangeOfString:@"^"];
    if (separator.location != NSNotFound) {
      line = [line substringToIndex:separator.location];
    }
  } else {
    NSArray<NSString*>* tokens =
        [line componentsSeparatedByCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
    NSMutableArray<NSString*>* nonempty = [NSMutableArray array];
    for (NSString* token in tokens) {
      if (token.length > 0) [nonempty addObject:token];
    }
    if (nonempty.count >= 2) {
      line = nonempty[1];
    }
  }

  line = [line lowercaseString];
  if ([line hasPrefix:@"http://"] || [line hasPrefix:@"https://"]) {
    NSURLComponents* components = [NSURLComponents componentsWithString:line];
    line = components.host ?: @"";
  }
  while ([line hasPrefix:@"."]) {
    line = [line substringFromIndex:1];
  }
  while ([line hasSuffix:@"."]) {
    line = [line substringToIndex:line.length - 1];
  }

  NSCharacterSet* allowed = [NSCharacterSet
      characterSetWithCharactersInString:
          @"abcdefghijklmnopqrstuvwxyz0123456789.-"];
  if (line.length == 0 ||
      [line rangeOfCharacterFromSet:allowed.invertedSet].location != NSNotFound ||
      [line rangeOfString:@"."].location == NSNotFound) {
    return nil;
  }
  return line;
}

NSSet<NSString*>* MoriAdBlockDomains() {
  static NSSet<NSString*>* domains = nil;
  static dispatch_once_t onceToken;
  dispatch_once(&onceToken, ^{
    NSMutableSet<NSString*>* parsed = [NSMutableSet set];
    NSURL* url = [NSBundle.mainBundle URLForResource:@"blocklistproject-ads"
                                       withExtension:@"txt"];
    NSString* body = url ? [NSString stringWithContentsOfURL:url
                                                    encoding:NSUTF8StringEncoding
                                                       error:nil]
                         : nil;
    [body enumerateLinesUsingBlock:^(NSString* line, BOOL* stop) {
      (void)stop;
      NSString* domain = MoriAdBlockDomainFromLine(line);
      if (domain.length > 0) [parsed addObject:domain];
    }];
    domains = [parsed copy];
  });
  return domains ?: [NSSet set];
}

BOOL MoriAdBlockHostMatches(NSString* host) {
  NSString* candidate = [host.lowercaseString
      stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
  while ([candidate hasSuffix:@"."]) {
    candidate = [candidate substringToIndex:candidate.length - 1];
  }
  NSSet<NSString*>* domains = MoriAdBlockDomains();
  while (candidate.length > 0) {
    if ([domains containsObject:candidate]) return YES;
    NSRange dot = [candidate rangeOfString:@"."];
    if (dot.location == NSNotFound) break;
    candidate = [candidate substringFromIndex:dot.location + 1];
  }
  return NO;
}

BOOL MoriAdBlockerShouldBlockRequest(CefRefPtr<CefRequest> request) {
  if (!MoriAdBlockerEnabled() || !request) return NO;
  NSString* urlString = @(request->GetURL().ToString().c_str());
  NSURLComponents* components = [NSURLComponents componentsWithString:urlString];
  NSString* scheme = components.scheme.lowercaseString ?: @"";
  if (![scheme isEqualToString:@"http"] &&
      ![scheme isEqualToString:@"https"] &&
      ![scheme isEqualToString:@"ws"] &&
      ![scheme isEqualToString:@"wss"]) {
    return NO;
  }
  NSString* host = components.host ?: @"";
  return MoriAdBlockHostMatches(host);
}

NSString* DNRRegexFromURLFilter(NSString* filter) {
  if (filter.length == 0) return @".*";

  BOOL startAnchored = [filter hasPrefix:@"|"] && ![filter hasPrefix:@"||"];
  BOOL domainAnchored = [filter hasPrefix:@"||"];
  BOOL endAnchored = [filter hasSuffix:@"|"] && filter.length > (domainAnchored ? 2 : 1);
  NSString* body = filter;
  if (domainAnchored) {
    body = [body substringFromIndex:2];
  } else if (startAnchored) {
    body = [body substringFromIndex:1];
  }
  if (endAnchored) {
    body = [body substringToIndex:body.length - 1];
  }

  NSMutableString* regex = [NSMutableString string];
  if (domainAnchored) {
    [regex appendString:@"^[a-z][a-z0-9+.-]*://([^/?#]+\\.)?"];
  } else if (startAnchored) {
    [regex appendString:@"^"];
  } else {
    [regex appendString:@".*"];
  }

  for (NSUInteger i = 0; i < body.length; i++) {
    unichar c = [body characterAtIndex:i];
    if (c == '*') {
      [regex appendString:@".*"];
    } else if (c == '^') {
      [regex appendString:@"([^A-Za-z0-9_.%-]|$)"];
    } else {
      NSString* one = [NSString stringWithCharacters:&c length:1];
      [regex appendString:[NSRegularExpression escapedPatternForString:one]];
    }
  }

  [regex appendString:endAnchored ? @"$" : @".*"];
  return regex;
}

BOOL DNRURLFilterMatches(NSString* filter, NSString* url, BOOL caseSensitive) {
  if (filter.length == 0) return YES;
  NSString* regex = DNRRegexFromURLFilter(filter);
  NSRegularExpressionOptions options =
      caseSensitive ? 0 : NSRegularExpressionCaseInsensitive;
  NSRegularExpression* re =
      [NSRegularExpression regularExpressionWithPattern:regex
                                                options:options
                                                  error:nil];
  if (!re) return NO;
  NSRange range = NSMakeRange(0, url.length);
  return [re firstMatchInString:url options:0 range:range] != nil;
}

BOOL DNRRegexMatches(NSString* pattern, NSString* url, BOOL caseSensitive) {
  if (pattern.length == 0) return YES;
  NSRegularExpressionOptions options =
      caseSensitive ? 0 : NSRegularExpressionCaseInsensitive;
  NSRegularExpression* re =
      [NSRegularExpression regularExpressionWithPattern:pattern
                                                options:options
                                                  error:nil];
  if (!re) return NO;
  return [re firstMatchInString:url
                        options:0
                          range:NSMakeRange(0, url.length)] != nil;
}

BOOL DNRRuleMatches(NSDictionary* rule,
                    NSURL* url,
                    NSString* urlString,
                    NSString* resourceType) {
  NSDictionary* condition = [rule[@"condition"] isKindOfClass:NSDictionary.class]
      ? rule[@"condition"]
      : nil;
  if (!condition) return NO;

  NSArray* resourceTypes = [condition[@"resourceTypes"] isKindOfClass:NSArray.class]
      ? condition[@"resourceTypes"]
      : nil;
  if (resourceTypes.count > 0 &&
      !DNRArrayContainsString(resourceTypes, resourceType)) {
    return NO;
  }
  NSArray* excludedResourceTypes =
      [condition[@"excludedResourceTypes"] isKindOfClass:NSArray.class]
          ? condition[@"excludedResourceTypes"]
          : nil;
  if (DNRArrayContainsString(excludedResourceTypes, resourceType)) {
    return NO;
  }

  NSString* host = url.host ?: @"";
  NSArray* requestDomains =
      [condition[@"requestDomains"] isKindOfClass:NSArray.class]
          ? condition[@"requestDomains"]
          : ([condition[@"domains"] isKindOfClass:NSArray.class]
                 ? condition[@"domains"]
                 : nil);
  if (requestDomains.count > 0 && !DNRDomainListMatches(requestDomains, host)) {
    return NO;
  }
  NSArray* excludedRequestDomains =
      [condition[@"excludedRequestDomains"] isKindOfClass:NSArray.class]
          ? condition[@"excludedRequestDomains"]
          : ([condition[@"excludedDomains"] isKindOfClass:NSArray.class]
                 ? condition[@"excludedDomains"]
                 : nil);
  if (DNRDomainListMatches(excludedRequestDomains, host)) {
    return NO;
  }

  BOOL caseSensitive = [condition[@"isUrlFilterCaseSensitive"] boolValue];
  NSString* regexFilter =
      [condition[@"regexFilter"] isKindOfClass:NSString.class]
          ? condition[@"regexFilter"]
          : nil;
  if (regexFilter.length > 0) {
    return DNRRegexMatches(regexFilter, urlString, caseSensitive);
  }
  NSString* urlFilter =
      [condition[@"urlFilter"] isKindOfClass:NSString.class]
          ? condition[@"urlFilter"]
          : nil;
  return urlFilter.length == 0 ||
         DNRURLFilterMatches(urlFilter, urlString, caseSensitive);
}

NSMutableDictionary<NSString*, NSArray*>* DNRSessionRulesByExtension() {
  static NSMutableDictionary<NSString*, NSArray*>* rules = nil;
  static dispatch_once_t onceToken;
  dispatch_once(&onceToken, ^{
    rules = [NSMutableDictionary dictionary];
  });
  return rules;
}

BOOL DNRRuleHasID(NSDictionary* rule, NSNumber* ruleID) {
  id raw = rule[@"id"];
  return ruleID && [raw respondsToSelector:@selector(integerValue)] &&
         [raw integerValue] == ruleID.integerValue;
}

NSArray<NSNumber*>* DNRRuleIDsFromArray(NSArray* rawIDs) {
  NSMutableArray<NSNumber*>* ids = [NSMutableArray array];
  for (id raw in rawIDs) {
    if ([raw respondsToSelector:@selector(integerValue)]) {
      [ids addObject:@([raw integerValue])];
    }
  }
  return ids;
}

NSArray<NSDictionary*>* DNRValidRules(NSArray* rawRules) {
  NSMutableArray<NSDictionary*>* rules = [NSMutableArray array];
  for (id raw in rawRules) {
    if ([raw isKindOfClass:NSDictionary.class] &&
        [NSJSONSerialization isValidJSONObject:raw]) {
      [rules addObject:raw];
    }
  }
  return rules;
}

NSArray<NSDictionary*>* DNRFilterRulesByIDs(NSArray<NSDictionary*>* rules,
                                            NSArray<NSNumber*>* ids) {
  if (ids.count == 0) return rules ?: @[];
  NSMutableArray<NSDictionary*>* out = [NSMutableArray array];
  for (NSDictionary* rule in rules) {
    for (NSNumber* ruleID in ids) {
      if (DNRRuleHasID(rule, ruleID)) {
        [out addObject:rule];
        break;
      }
    }
  }
  return out;
}

NSArray<NSDictionary*>* DNRStoredDynamicRules(NSString* extensionID) {
  NSArray* stored = [[NSUserDefaults standardUserDefaults]
      arrayForKey:DNRDynamicRulesDefaultsKey(extensionID)];
  return DNRValidRules(stored);
}

NSArray<NSDictionary*>* DNRStoredSessionRules(NSString* extensionID) {
  NSMutableDictionary<NSString*, NSArray*>* sessions = DNRSessionRulesByExtension();
  @synchronized(sessions) {
    return DNRValidRules(sessions[extensionID] ?: @[]);
  }
}

NSArray<NSDictionary*>* DNRRulesAfterUpdate(NSArray<NSDictionary*>* existing,
                                            NSArray<NSNumber*>* removeIDs,
                                            NSArray<NSDictionary*>* addRules) {
  NSMutableArray<NSDictionary*>* next = [NSMutableArray array];
  for (NSDictionary* rule in existing) {
    BOOL shouldRemove = NO;
    for (NSNumber* ruleID in removeIDs) {
      if (DNRRuleHasID(rule, ruleID)) {
        shouldRemove = YES;
        break;
      }
    }
    if (!shouldRemove) [next addObject:rule];
  }
  for (NSDictionary* rule in addRules) {
    id rawID = rule[@"id"];
    if (![rawID respondsToSelector:@selector(integerValue)]) continue;
    NSInteger ruleID = [rawID integerValue];
    [next filterUsingPredicate:[NSPredicate predicateWithBlock:^BOOL(
        NSDictionary* existingRule, NSDictionary* bindings) {
      id existingID = existingRule[@"id"];
      return ![existingID respondsToSelector:@selector(integerValue)] ||
             [existingID integerValue] != ruleID;
    }]];
    [next addObject:rule];
  }
  return next;
}

NSArray<NSString*>* DNRDefaultEnabledRulesetIDs(NSDictionary* manifest) {
  NSDictionary* dnr =
      [manifest[@"declarative_net_request"] isKindOfClass:NSDictionary.class]
          ? manifest[@"declarative_net_request"]
          : nil;
  NSArray* resources =
      [dnr[@"rule_resources"] isKindOfClass:NSArray.class]
          ? dnr[@"rule_resources"]
          : nil;
  NSMutableArray<NSString*>* ids = [NSMutableArray array];
  for (id item in resources) {
    if (![item isKindOfClass:NSDictionary.class]) continue;
    NSDictionary* resource = (NSDictionary*)item;
    NSNumber* enabled = [resource[@"enabled"] isKindOfClass:NSNumber.class]
        ? resource[@"enabled"]
        : @YES;
    NSString* rulesetID = [resource[@"id"] isKindOfClass:NSString.class]
        ? resource[@"id"]
        : nil;
    if (enabled.boolValue && rulesetID.length > 0) [ids addObject:rulesetID];
  }
  return ids;
}

NSArray<NSString*>* DNREnabledRulesetIDs(NSString* extensionID,
                                         NSDictionary* manifest) {
  NSArray* stored = [[NSUserDefaults standardUserDefaults]
      arrayForKey:DNREnabledRulesetsDefaultsKey(extensionID)];
  if (stored) {
    NSMutableArray<NSString*>* ids = [NSMutableArray array];
    for (id item in stored) {
      if ([item isKindOfClass:NSString.class]) [ids addObject:item];
    }
    return ids;
  }
  return DNRDefaultEnabledRulesetIDs(manifest);
}

BOOL DNRRulesetIsEnabled(NSString* extensionID,
                         NSDictionary* manifest,
                         NSDictionary* resource) {
  NSString* rulesetID = [resource[@"id"] isKindOfClass:NSString.class]
      ? resource[@"id"]
      : nil;
  if (rulesetID.length == 0) {
    NSNumber* enabled = [resource[@"enabled"] isKindOfClass:NSNumber.class]
        ? resource[@"enabled"]
        : @YES;
    return enabled.boolValue;
  }
  return DNRArrayContainsString(DNREnabledRulesetIDs(extensionID, manifest),
                                rulesetID);
}

NSArray<NSDictionary*>* DNRStaticRulesForExtension(NSDictionary* ext,
                                                   NSDictionary* manifest) {
  NSDictionary* dnr =
      [manifest[@"declarative_net_request"] isKindOfClass:NSDictionary.class]
          ? manifest[@"declarative_net_request"]
          : nil;
  NSArray* resources =
      [dnr[@"rule_resources"] isKindOfClass:NSArray.class]
          ? dnr[@"rule_resources"]
          : nil;
  if (resources.count == 0) return @[];

  NSMutableArray<NSDictionary*>* rules = [NSMutableArray array];
  NSString* extensionID = [ext[@"id"] isKindOfClass:NSString.class] ? ext[@"id"] : @"";
  for (id item in resources) {
    if (![item isKindOfClass:NSDictionary.class]) continue;
    NSDictionary* resource = (NSDictionary*)item;
    if (!DNRRulesetIsEnabled(extensionID, manifest, resource)) continue;
    NSString* path = [resource[@"path"] isKindOfClass:NSString.class]
        ? resource[@"path"]
        : nil;
    NSString* text = ExtensionFileText(ext, path);
    NSData* data = [text dataUsingEncoding:NSUTF8StringEncoding];
    if (!data) continue;
    id json = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    if (![json isKindOfClass:NSArray.class]) continue;
    for (id rule in (NSArray*)json) {
      if ([rule isKindOfClass:NSDictionary.class]) {
        [rules addObject:rule];
      }
    }
  }
  return rules;
}

NSArray<NSDictionary*>* DNRRulesForExtension(NSDictionary* ext,
                                             NSDictionary* manifest) {
  NSMutableArray<NSDictionary*>* rules =
      [DNRStaticRulesForExtension(ext, manifest) mutableCopy];
  NSString* extensionID = [ext[@"id"] isKindOfClass:NSString.class] ? ext[@"id"] : @"";
  [rules addObjectsFromArray:DNRStoredDynamicRules(extensionID)];
  [rules addObjectsFromArray:DNRStoredSessionRules(extensionID)];
  return rules;
}

NSDictionary* HandleDeclarativeNetRequest(NSString* method,
                                          NSDictionary* args,
                                          NSString* extensionID) {
  NSDictionary* ext = EnabledExtensionRecordForID(extensionID);
  NSDictionary* manifest = ManifestForExtension(ext ?: @{});
  if (!ext || !manifest) return @{@"error" : @"Extension is not enabled."};

  NSUserDefaults* defaults = [NSUserDefaults standardUserDefaults];
  NSDictionary* details = [args[@"details"] isKindOfClass:NSDictionary.class]
      ? args[@"details"]
      : @{};
  NSDictionary* filter = [args[@"filter"] isKindOfClass:NSDictionary.class]
      ? args[@"filter"]
      : @{};
  NSArray<NSNumber*>* filterIDs =
      DNRRuleIDsFromArray([filter[@"ruleIds"] isKindOfClass:NSArray.class]
                              ? filter[@"ruleIds"]
                              : nil);

  if ([method isEqualToString:@"declarativeNetRequest.getEnabledRulesets"]) {
    return @{@"result" : DNREnabledRulesetIDs(extensionID, manifest)};
  }
  if ([method isEqualToString:@"declarativeNetRequest.updateEnabledRulesets"]) {
    NSMutableOrderedSet<NSString*>* ids = [NSMutableOrderedSet orderedSetWithArray:
        DNREnabledRulesetIDs(extensionID, manifest)];
    NSArray* disable = [details[@"disableRulesetIds"] isKindOfClass:NSArray.class]
        ? details[@"disableRulesetIds"]
        : @[];
    for (id item in disable) {
      if ([item isKindOfClass:NSString.class]) [ids removeObject:item];
    }
    NSArray* enable = [details[@"enableRulesetIds"] isKindOfClass:NSArray.class]
        ? details[@"enableRulesetIds"]
        : @[];
    for (id item in enable) {
      if ([item isKindOfClass:NSString.class]) [ids addObject:item];
    }
    [defaults setObject:ids.array forKey:DNREnabledRulesetsDefaultsKey(extensionID)];
    return @{@"result" : @{}};
  }
  if ([method isEqualToString:@"declarativeNetRequest.getAvailableStaticRuleCount"]) {
    NSInteger used = (NSInteger)DNRStaticRulesForExtension(ext, manifest).count;
    return @{@"result" : @(MAX(0, 300000 - used))};
  }
  if ([method isEqualToString:@"declarativeNetRequest.getDynamicRules"]) {
    return @{@"result" : DNRFilterRulesByIDs(DNRStoredDynamicRules(extensionID), filterIDs)};
  }
  if ([method isEqualToString:@"declarativeNetRequest.updateDynamicRules"]) {
    NSArray<NSNumber*>* removeIDs =
        DNRRuleIDsFromArray([details[@"removeRuleIds"] isKindOfClass:NSArray.class]
                                ? details[@"removeRuleIds"]
                                : nil);
    NSArray<NSDictionary*>* addRules =
        DNRValidRules([details[@"addRules"] isKindOfClass:NSArray.class]
                          ? details[@"addRules"]
                          : nil);
    NSArray<NSDictionary*>* next =
        DNRRulesAfterUpdate(DNRStoredDynamicRules(extensionID), removeIDs, addRules);
    [defaults setObject:next forKey:DNRDynamicRulesDefaultsKey(extensionID)];
    return @{@"result" : @{}};
  }
  if ([method isEqualToString:@"declarativeNetRequest.getSessionRules"]) {
    return @{@"result" : DNRFilterRulesByIDs(DNRStoredSessionRules(extensionID), filterIDs)};
  }
  if ([method isEqualToString:@"declarativeNetRequest.updateSessionRules"]) {
    NSArray<NSNumber*>* removeIDs =
        DNRRuleIDsFromArray([details[@"removeRuleIds"] isKindOfClass:NSArray.class]
                                ? details[@"removeRuleIds"]
                                : nil);
    NSArray<NSDictionary*>* addRules =
        DNRValidRules([details[@"addRules"] isKindOfClass:NSArray.class]
                          ? details[@"addRules"]
                          : nil);
    NSMutableDictionary<NSString*, NSArray*>* sessions = DNRSessionRulesByExtension();
    @synchronized(sessions) {
      sessions[extensionID] =
          DNRRulesAfterUpdate(DNRStoredSessionRules(extensionID), removeIDs, addRules);
    }
    return @{@"result" : @{}};
  }
  if ([method isEqualToString:@"declarativeNetRequest.isRegexSupported"]) {
    NSString* regex = [args[@"regex"] isKindOfClass:NSString.class]
        ? args[@"regex"]
        : ([details[@"regex"] isKindOfClass:NSString.class] ? details[@"regex"] : @"");
    NSError* error = nil;
    [NSRegularExpression regularExpressionWithPattern:regex options:0 error:&error];
    return error ? @{@"result" : @{@"isSupported" : @NO,
                                   @"reason" : error.localizedDescription ?: @""}}
                 : @{@"result" : @{@"isSupported" : @YES}};
  }
  return @{@"error" : [NSString stringWithFormat:@"Unsupported DNR method: %@", method]};
}

NSString* DNRExtensionResourceURL(NSDictionary* ext, NSString* path) {
  NSString* extensionID = [ext[@"id"] isKindOfClass:NSString.class] ? ext[@"id"] : @"";
  if (extensionID.length == 0 || path.length == 0) return nil;
  NSString* clean = [path hasPrefix:@"/"] ? [path substringFromIndex:1] : path;
  NSString* encoded =
      [clean stringByAddingPercentEncodingWithAllowedCharacters:
                 NSCharacterSet.URLPathAllowedCharacterSet] ?: clean;
  return [NSString stringWithFormat:@"%s://%@/%@",
                                    mori::kExtensionScheme,
                                    extensionID.lowercaseString,
                                    encoded];
}

NSString* DNRRedirectURL(NSDictionary* action,
                         NSDictionary* ext,
                         NSURL* originalURL,
                         NSString* urlString) {
  NSDictionary* redirect = [action[@"redirect"] isKindOfClass:NSDictionary.class]
      ? action[@"redirect"]
      : nil;

  NSString* absoluteURL = [redirect[@"url"] isKindOfClass:NSString.class]
      ? redirect[@"url"]
      : nil;
  if (absoluteURL.length > 0) {
    NSURL* parsed = [NSURL URLWithString:absoluteURL];
    NSString* scheme = parsed.scheme.lowercaseString ?: @"";
    if ([@[@"http", @"https", @"file", @(mori::kExtensionScheme)] containsObject:scheme]) {
      return absoluteURL;
    }
  }

  NSString* extensionPath =
      [redirect[@"extensionPath"] isKindOfClass:NSString.class]
          ? redirect[@"extensionPath"]
          : nil;
  if (extensionPath.length > 0) {
    return DNRExtensionResourceURL(ext, extensionPath);
  }

  NSString* regexSubstitution =
      [redirect[@"regexSubstitution"] isKindOfClass:NSString.class]
          ? redirect[@"regexSubstitution"]
          : nil;
  NSDictionary* condition = [action[@"condition"] isKindOfClass:NSDictionary.class]
      ? action[@"condition"]
      : nil;
  (void)condition;

  if (regexSubstitution.length > 0) {
    // Regex substitution requires the rule condition's regexFilter. The rule is
    // already known to match, so this lightweight path only performs the
    // replacement when the regex compiles locally.
    return nil;
  }

  NSString* type = [action[@"type"] isKindOfClass:NSString.class]
      ? ((NSString*)action[@"type"]).lowercaseString
      : @"";
  if ([type isEqualToString:@"upgradescheme"] &&
      [originalURL.scheme.lowercaseString isEqualToString:@"http"]) {
    NSURLComponents* components =
        [NSURLComponents componentsWithURL:originalURL resolvingAgainstBaseURL:NO];
    components.scheme = @"https";
    return components.URL.absoluteString;
  }

  return nil;
}

NSDictionary* DeclarativeNetRequestDecision(CefRefPtr<CefRequest> request) {
  if (!request) return @{@"type" : @"none"};
  NSString* urlString = @(request->GetURL().ToString().c_str());
  NSURL* url = [NSURL URLWithString:urlString];
  NSString* scheme = url.scheme.lowercaseString ?: @"";
  if (![@[@"http", @"https", @"file"] containsObject:scheme]) {
    return @{@"type" : @"none"};
  }

  NSString* resourceType = DNRResourceType(request);
  NSInteger bestPriority = NSIntegerMin;
  NSString* bestType = @"none";
  NSString* bestRedirectURL = nil;

  for (NSDictionary* ext in EnabledExtensionRecords()) {
    NSDictionary* manifest = ManifestForExtension(ext);
    for (NSDictionary* rule in DNRRulesForExtension(ext, manifest ?: @{})) {
      if (!DNRRuleMatches(rule, url, urlString, resourceType)) continue;
      NSDictionary* action = [rule[@"action"] isKindOfClass:NSDictionary.class]
          ? rule[@"action"]
          : nil;
      NSString* type = [action[@"type"] isKindOfClass:NSString.class]
          ? ((NSString*)action[@"type"]).lowercaseString
          : @"";
      BOOL blocks = [type isEqualToString:@"block"];
      BOOL allows = [type isEqualToString:@"allow"] ||
                    [type isEqualToString:@"allowallrequests"];
      BOOL redirects = [type isEqualToString:@"redirect"] ||
                       [type isEqualToString:@"upgradescheme"];
      NSString* redirectURL = nil;
      if (redirects) {
        redirectURL = DNRRedirectURL(action, ext, url, urlString);
        if (redirectURL.length == 0) continue;
      }
      if (!blocks && !allows && !redirects) continue;
      NSInteger priority = [rule[@"priority"] respondsToSelector:@selector(integerValue)]
          ? [rule[@"priority"] integerValue]
          : 1;
      if (priority > bestPriority ||
          (priority == bestPriority && allows && [bestType isEqualToString:@"block"])) {
        bestPriority = priority;
        bestType = allows ? @"allow" : (blocks ? @"block" : @"redirect");
        bestRedirectURL = redirectURL;
      }
    }
  }

  NSMutableDictionary* decision = [@{@"type" : bestType ?: @"none"} mutableCopy];
  if (bestRedirectURL.length > 0) decision[@"redirectUrl"] = bestRedirectURL;
  return decision;
}

NSArray<NSDictionary*>* DNRModifyHeaderOperations(CefRefPtr<CefRequest> request,
                                                  NSString* headerKey) {
  if (!request || headerKey.length == 0) return @[];
  NSString* urlString = @(request->GetURL().ToString().c_str());
  NSURL* url = [NSURL URLWithString:urlString];
  NSString* scheme = url.scheme.lowercaseString ?: @"";
  if (![@[@"http", @"https", @"file"] containsObject:scheme]) return @[];

  NSString* resourceType = DNRResourceType(request);
  NSMutableArray<NSDictionary*>* operations = [NSMutableArray array];
  NSUInteger sequence = 0;
  for (NSDictionary* ext in EnabledExtensionRecords()) {
    NSDictionary* manifest = ManifestForExtension(ext);
    for (NSDictionary* rule in DNRRulesForExtension(ext, manifest ?: @{})) {
      if (!DNRRuleMatches(rule, url, urlString, resourceType)) continue;
      NSDictionary* action = [rule[@"action"] isKindOfClass:NSDictionary.class]
          ? rule[@"action"]
          : nil;
      NSString* type = [action[@"type"] isKindOfClass:NSString.class]
          ? ((NSString*)action[@"type"]).lowercaseString
          : @"";
      if (![type isEqualToString:@"modifyheaders"]) continue;
      NSArray* rawOperations = [action[headerKey] isKindOfClass:NSArray.class]
          ? action[headerKey]
          : @[];
      NSInteger priority = [rule[@"priority"] respondsToSelector:@selector(integerValue)]
          ? [rule[@"priority"] integerValue]
          : 1;
      for (id item in rawOperations) {
        if (![item isKindOfClass:NSDictionary.class]) continue;
        NSMutableDictionary* op = [(NSDictionary*)item mutableCopy];
        op[@"__priority"] = @(priority);
        op[@"__sequence"] = @(sequence++);
        [operations addObject:op];
      }
    }
  }
  [operations sortUsingComparator:^NSComparisonResult(NSDictionary* a,
                                                      NSDictionary* b) {
    NSInteger ap = [a[@"__priority"] integerValue];
    NSInteger bp = [b[@"__priority"] integerValue];
    if (ap > bp) return NSOrderedAscending;
    if (ap < bp) return NSOrderedDescending;
    NSUInteger as = [a[@"__sequence"] unsignedIntegerValue];
    NSUInteger bs = [b[@"__sequence"] unsignedIntegerValue];
    if (as < bs) return NSOrderedAscending;
    if (as > bs) return NSOrderedDescending;
    return NSOrderedSame;
  }];
  return operations;
}

void HeaderMapRemoveName(CefRequest::HeaderMap& headerMap, NSString* headerName) {
  if (headerName.length == 0) return;
  for (auto it = headerMap.begin(); it != headerMap.end();) {
    NSString* name = @(it->first.ToString().c_str());
    if ([name caseInsensitiveCompare:headerName] == NSOrderedSame) {
      it = headerMap.erase(it);
    } else {
      ++it;
    }
  }
}

void ApplyDNRHeaderOperations(CefRequest::HeaderMap& headerMap,
                              NSArray<NSDictionary*>* operations) {
  for (NSDictionary* operation in operations) {
    NSString* header = [operation[@"header"] isKindOfClass:NSString.class]
        ? operation[@"header"]
        : @"";
    NSString* op = [operation[@"operation"] isKindOfClass:NSString.class]
        ? ((NSString*)operation[@"operation"]).lowercaseString
        : @"";
    if (header.length == 0 || op.length == 0) continue;
    if ([op isEqualToString:@"remove"]) {
      HeaderMapRemoveName(headerMap, header);
      continue;
    }
    NSString* value = [operation[@"value"] isKindOfClass:NSString.class]
        ? operation[@"value"]
        : @"";
    if ([op isEqualToString:@"set"]) {
      HeaderMapRemoveName(headerMap, header);
      headerMap.insert(std::make_pair(CefString(header.UTF8String),
                                      CefString(value.UTF8String)));
    } else if ([op isEqualToString:@"append"]) {
      headerMap.insert(std::make_pair(CefString(header.UTF8String),
                                      CefString(value.UTF8String)));
    }
  }
}

void ApplyDNRRequestHeaderModifications(CefRefPtr<CefRequest> request) {
  NSArray<NSDictionary*>* operations =
      DNRModifyHeaderOperations(request, @"requestHeaders");
  if (operations.count == 0) return;
  CefRequest::HeaderMap headerMap;
  request->GetHeaderMap(headerMap);
  ApplyDNRHeaderOperations(headerMap, operations);
  request->SetHeaderMap(headerMap);
}

NSDictionary* ExtensionRecordForFrame(CefRefPtr<CefFrame> frame) {
  if (!frame) return nil;
  NSString* urlString = @(frame->GetURL().ToString().c_str());
  NSURL* url = [NSURL URLWithString:urlString];
  if (![url.scheme isEqualToString:@(mori::kExtensionScheme)]) {
    return nil;
  }
  return EnabledExtensionRecordForID(url.host ?: @"");
}

NSDictionary* ExtensionRecordForOrigin(NSString* origin) {
  if (origin.length == 0) return nil;
  NSURL* url = [NSURL URLWithString:origin];
  if (![url.scheme isEqualToString:@(mori::kExtensionScheme)]) {
    return nil;
  }
  return EnabledExtensionRecordForID(url.host ?: @"");
}

NSMutableDictionary<NSNumber*, NSString*>* ExtensionRequestInitiators() {
  static NSMutableDictionary<NSNumber*, NSString*>* initiators =
      [NSMutableDictionary dictionary];
  return initiators;
}

NSNumber* ExtensionRequestIdentifier(CefRefPtr<CefRequest> request) {
  if (!request) return nil;
  uint64_t identifier = request->GetIdentifier();
  if (identifier == 0) return nil;
  return @(static_cast<unsigned long long>(identifier));
}

void RememberExtensionRequestInitiator(CefRefPtr<CefRequest> request,
                                       const CefString& requestInitiator) {
  NSNumber* key = ExtensionRequestIdentifier(request);
  if (!key) return;
  NSString* initiator = @(requestInitiator.ToString().c_str());
  if (initiator.length == 0) return;
  NSMutableDictionary* initiators = ExtensionRequestInitiators();
  @synchronized(initiators) {
    initiators[key] = initiator;
  }
}

NSString* RememberedExtensionRequestInitiator(CefRefPtr<CefRequest> request) {
  NSNumber* key = ExtensionRequestIdentifier(request);
  if (!key) return nil;
  NSMutableDictionary* initiators = ExtensionRequestInitiators();
  @synchronized(initiators) {
    return initiators[key];
  }
}

void ForgetExtensionRequestInitiator(CefRefPtr<CefRequest> request) {
  NSNumber* key = ExtensionRequestIdentifier(request);
  if (!key) return;
  NSMutableDictionary* initiators = ExtensionRequestInitiators();
  @synchronized(initiators) {
    [initiators removeObjectForKey:key];
  }
}

NSString* HeaderValue(const std::multimap<CefString, CefString>& headerMap,
                      NSString* headerName) {
  if (headerName.length == 0) return nil;
  for (const auto& entry : headerMap) {
    NSString* name = @(entry.first.ToString().c_str());
    if ([name caseInsensitiveCompare:headerName] == NSOrderedSame) {
      return @(entry.second.ToString().c_str());
    }
  }
  return nil;
}

NSString* RequestHeaderValue(CefRefPtr<CefRequest> request,
                             NSString* headerName) {
  if (!request) return nil;
  CefRequest::HeaderMap headerMap;
  request->GetHeaderMap(headerMap);
  return HeaderValue(headerMap, headerName);
}

NSString* ExtensionCORSOrigin(CefRefPtr<CefFrame> frame,
                              CefRefPtr<CefRequest> request) {
  if (!request) return nil;
  NSString* urlString = @(request->GetURL().ToString().c_str());
  NSURL* url = [NSURL URLWithString:urlString];
  NSString* scheme = url.scheme.lowercaseString ?: @"";
  if (![@[@"http", @"https"] containsObject:scheme]) return nil;
  NSString* origin = RequestHeaderValue(request, @"Origin");
  BOOL extensionOrigin =
      origin.length > 0 &&
      [[origin lowercaseString] hasPrefix:
          [NSString stringWithFormat:@"%s://", mori::kExtensionScheme]];
  NSString* initiator = RememberedExtensionRequestInitiator(request);
  BOOL extensionInitiator =
      initiator.length > 0 &&
      [[initiator lowercaseString] hasPrefix:
          [NSString stringWithFormat:@"%s://", mori::kExtensionScheme]];
  NSDictionary* ext = extensionOrigin ? ExtensionRecordForOrigin(origin) : nil;
  if (!ext && extensionInitiator) {
    ext = ExtensionRecordForOrigin(initiator);
  }
  if (!ext) ext = ExtensionRecordForFrame(frame);
  if (!ext) return nil;
  NSDictionary* manifest = ManifestForExtension(ext);
  if (!ExtensionHostPermissionsAllow(manifest, url)) return nil;

  if (extensionOrigin) {
    return origin;
  }
  if (extensionInitiator) {
    return initiator;
  }
  NSString* extensionID =
      [ext[@"id"] isKindOfClass:NSString.class] ? ext[@"id"] : @"";
  return extensionID.length > 0
      ? [NSString stringWithFormat:@"%s://%@",
                                   mori::kExtensionScheme,
                                   extensionID.lowercaseString]
      : nil;
}

void ResponseHeaderMapSet(CefResponse::HeaderMap& headerMap,
                          const char* name,
                          NSString* value) {
  if (!name || value.length == 0) return;
  for (auto it = headerMap.begin(); it != headerMap.end();) {
    NSString* existing = @(it->first.ToString().c_str());
    if ([existing caseInsensitiveCompare:@(name)] == NSOrderedSame) {
      it = headerMap.erase(it);
    } else {
      ++it;
    }
  }
  headerMap.insert(std::make_pair(CefString(name), CefString(value.UTF8String)));
}

CefRefPtr<CefResourceHandler> ExtensionPreflightResponse(
    CefRefPtr<CefFrame> frame,
    CefRefPtr<CefRequest> request) {
  if (!request) return nullptr;
  NSString* method = @(request->GetMethod().ToString().c_str());
  if ([method caseInsensitiveCompare:@"OPTIONS"] != NSOrderedSame) {
    return nullptr;
  }
  NSString* origin = ExtensionCORSOrigin(frame, request);
  if (origin.length == 0) return nullptr;

  NSString* allowedMethod =
      RequestHeaderValue(request, @"Access-Control-Request-Method");
  if (allowedMethod.length == 0) {
    allowedMethod = @"GET, POST, PUT, PATCH, DELETE, OPTIONS";
  }
  NSString* allowedHeaders =
      RequestHeaderValue(request, @"Access-Control-Request-Headers");
  if (allowedHeaders.length == 0) allowedHeaders = @"*";

  CefResponse::HeaderMap headers;
  headers.insert(std::make_pair(CefString("Access-Control-Allow-Origin"),
                                CefString(origin.UTF8String)));
  headers.insert(std::make_pair(CefString("Access-Control-Allow-Credentials"),
                                CefString("true")));
  headers.insert(std::make_pair(CefString("Access-Control-Allow-Methods"),
                                CefString(allowedMethod.UTF8String)));
  headers.insert(std::make_pair(CefString("Access-Control-Allow-Headers"),
                                CefString(allowedHeaders.UTF8String)));
  headers.insert(std::make_pair(CefString("Access-Control-Max-Age"),
                                CefString("600")));
  headers.insert(std::make_pair(CefString("Cache-Control"),
                                CefString("no-store")));
  headers.insert(std::make_pair(CefString("Content-Length"),
                                CefString("3")));
  auto stream = CefStreamReader::CreateForData(const_cast<char*>("OK\n"), 3);
  return new CefStreamResourceHandler(200, CefString("OK"),
                                      CefString("text/plain"),
                                      headers, stream);
}

void ApplyExtensionCORSHeaders(CefRefPtr<CefFrame> frame,
                               CefRefPtr<CefRequest> request,
                               CefRefPtr<CefResponse> response) {
  if (!response) return;
  NSString* origin = ExtensionCORSOrigin(frame, request);
  if (origin.length == 0) return;

  CefResponse::HeaderMap headers;
  response->GetHeaderMap(headers);
  ResponseHeaderMapSet(headers, "Access-Control-Allow-Origin", origin);
  ResponseHeaderMapSet(headers, "Access-Control-Allow-Credentials", @"true");
  ResponseHeaderMapSet(headers, "Access-Control-Allow-Methods",
                       @"GET, POST, PUT, PATCH, DELETE, OPTIONS");
  NSString* requestedHeaders =
      RequestHeaderValue(request, @"Access-Control-Request-Headers");
  if (requestedHeaders.length > 0) {
    ResponseHeaderMapSet(headers, "Access-Control-Allow-Headers",
                         requestedHeaders);
  }
  response->SetHeaderMap(headers);
}

NSString* ExtensionRuntimeShim(NSDictionary* ext, NSDictionary* manifest) {
  // Canonicalize to lowercase: the page's runtime.id comes from the URL host
  // (mori-extension://<host>/…), which the URL parser lowercases, while the
  // catalog id is an uppercase UUID. Extensions that compare sender.id against
  // runtime.id (e.g. Proton Pass's MessageBroker, to tell internal from
  // external messages) reject every internal message when the two differ only
  // in case. Real Chrome ids are always lowercase, so matching that keeps
  // sender.id === runtime.id and the whole id surface consistent.
  NSString* identifier = ([ext[@"id"] isKindOfClass:[NSString class]]
                              ? (NSString*)ext[@"id"]
                              : @"").lowercaseString;
  NSDictionary* i18n = ExtensionI18nBundle(ext, manifest ?: @{});
  NSDictionary* messages = [i18n[@"messages"] isKindOfClass:[NSDictionary class]]
      ? i18n[@"messages"]
      : @{};
  NSString* uiLanguage = [i18n[@"locale"] isKindOfClass:[NSString class]]
      ? i18n[@"locale"]
      : @"en";
  id localizedManifest = LocalizedManifestValue(manifest ?: @{}, messages);
  NSDictionary* browserInfo = @{
    @"name" : @"Mori",
    @"vendor" : @"Mori",
    @"version" : MoriHostBrowserVersion(),
    @"buildID" : @""
  };
  return [NSString stringWithFormat:
      @"(function(){"
       "var extId=%@;"
       "var manifest=%@;"
       "var i18nMessages=%@;"
       "var uiLanguage=%@;"
       "var hostBrowserInfo=%@;"
       "var __moriNativeConsoleInfo=window.__moriNativeConsoleInfo||"
       "(window.__moriNativeConsoleInfo=(function(){try{return Function.prototype.bind.call(console.info,console);}catch(e){return function(message){try{console.info(message);}catch(_){}};}})());"
		       // Capture private aliases to our chrome/browser objects. Some
		       // extensions (e.g. Proton Pass) replace globalThis.chrome/browser
		       // with an anti-tampering Proxy shortly after load; their own code
		       // survives because webextension-polyfill captured the real API by
		       // reference at import. Our runtime delivers responses and events
		       // back into the page via these objects, so it must hold its own
		       // reference rather than re-reading the (later-sealed) global.
		       "var chrome=globalThis.chrome=globalThis.chrome||{};"
		       "window.__moriExtensionID=extId;"
		       "chrome.runtime=chrome.runtime||{};"
		       "var runtime=chrome.runtime;"
		       "runtime.id=runtime.id||extId;"
			       "try{"
			       "globalThis.browser=globalThis.browser||globalThis.chrome;"
			       "globalThis.browser.runtime=globalThis.browser.runtime||runtime;"
			       "globalThis.browser.runtime.id=globalThis.browser.runtime.id||extId;"
			       "globalThis.browser.name=globalThis.browser.name||hostBrowserInfo.name;"
			       "globalThis.browser.version=globalThis.browser.version||hostBrowserInfo.version;"
			       "}catch(e){}"
			       "var browser=globalThis.browser||chrome;"
       "function __moriEvent(){"
       "var listeners=[];"
       "return {addListener:function(fn){if(typeof fn==='function'&&listeners.indexOf(fn)<0)listeners.push(fn);},"
       "removeListener:function(fn){var i=listeners.indexOf(fn);if(i>=0)listeners.splice(i,1);},"
	       "hasListener:function(fn){return listeners.indexOf(fn)>=0;},"
	       "hasListeners:function(){return listeners.length>0;},"
	       "_listeners:listeners,"
	       "_fire:function(){var args=arguments;listeners.slice().forEach(function(fn){try{fn.apply(null,args);}catch(e){console.error(e);}});}};"
	       "}"
		       "function __moriCallbackLastError(message,cb){"
			      "runtime.lastError={message:String(message||'Extension API error')};"
			      "try{if(typeof cb==='function')cb();}finally{setTimeout(function(){delete runtime.lastError;},0);}"
		      "}"
	       "function __moriSourceInfo(){"
	       "var origin='';"
	       "try{origin=String(location.origin&&location.origin!=='null'?location.origin:(new URL(String(location.href))).origin||'');if(origin==='null')origin='';}catch(e){}"
	       "return {sourceUrl:String(location.href),sourceOrigin:origin,frameId:0};"
	       "}"
       "runtime.onMessage=runtime.onMessage||__moriEvent();"
       "runtime.onMessageExternal=runtime.onMessageExternal||__moriEvent();"
       "runtime.onConnect=runtime.onConnect||__moriEvent();"
       "runtime.onConnectExternal=runtime.onConnectExternal||__moriEvent();"
       "runtime.onInstalled=runtime.onInstalled||__moriEvent();"
       "runtime.onStartup=runtime.onStartup||__moriEvent();"
       "runtime.onSuspend=runtime.onSuspend||__moriEvent();"
	       "runtime.onSuspendCanceled=runtime.onSuspendCanceled||__moriEvent();"
	       "runtime.onUpdateAvailable=runtime.onUpdateAvailable||__moriEvent();"
	       "try{"
	       "browser.runtime=browser.runtime||runtime;"
	       "['onMessage','onMessageExternal','onConnect','onConnectExternal','onInstalled','onStartup','onSuspend','onSuspendCanceled','onUpdateAvailable'].forEach(function(name){"
	       "if(!browser.runtime[name])browser.runtime[name]=runtime[name];"
	       "});"
	       "}catch(e){}"
	       "runtime.getURL=runtime.getURL||function(path){"
       "var clean=String(path||'').replace(/^\\/+/, '');"
       "return 'mori-extension://'+extId+'/'+encodeURI(clean).replace(/#/g,'%%23');"
       "};"
       "runtime.getManifest=runtime.getManifest||function(){"
       "return JSON.parse(JSON.stringify(manifest));"
       "};"
       "chrome.i18n=chrome.i18n||{};"
       "chrome.i18n.getUILanguage=chrome.i18n.getUILanguage||function(){return uiLanguage;};"
       "function __moriI18nExpand(raw,substitutions,placeholders){"
       "var subs=Array.isArray(substitutions)?substitutions:"
       "(substitutions===undefined||substitutions===null?[]:[substitutions]);"
       "var text=String(raw||'').replace(/\\$\\$/g,'\\u0000');"
       "text=text.replace(/\\$([A-Za-z0-9_]+)\\$/g,function(_,name){"
       "var p=placeholders&&placeholders[String(name).toLowerCase()];"
       "var content=p&&typeof p.content==='string'?p.content:'';"
       "return content?__moriI18nExpand(content,subs,{}):'';"
       "});"
       "text=text.replace(/\\$([1-9]\\d*)/g,function(_,index){"
       "var value=subs[Number(index)-1];"
       "return value===undefined||value===null?'':String(value);"
       "});"
       "return text.replace(/\\u0000/g,'$');"
       "}"
       "chrome.i18n.getMessage=chrome.i18n.getMessage||function(name,substitutions){"
       "var entry=i18nMessages[String(name||'').toLowerCase()];"
       "if(!entry||typeof entry.message!=='string')return '';"
       "return __moriI18nExpand(entry.message,substitutions,entry.placeholders||{});"
       "};"
       "chrome.i18n.getAcceptLanguages=chrome.i18n.getAcceptLanguages||function(cb){"
       "var result=[navigator.language||uiLanguage];"
       "if(typeof cb==='function')cb(result);return Promise.resolve(result);"
       "};"
       "chrome.i18n.detectLanguage=chrome.i18n.detectLanguage||function(text,cb){"
       "var result={isReliable:false,languages:[{language:(navigator.language||uiLanguage).split('-')[0],percentage:100}]};"
       "if(typeof cb==='function')cb(result);return Promise.resolve(result);"
       "};"
	       "runtime.sendMessage=runtime.sendMessage||function(){"
       "var target=extId,message=null,options={},cb=null;"
       "if(typeof arguments[0]==='string'&&arguments.length>=2&&typeof arguments[1]!=='function'){"
       "target=arguments[0];message=arguments[1];"
       "if(typeof arguments[2]==='function'){cb=arguments[2];}"
       "else{options=arguments[2]||{};cb=arguments[3];}"
       "}else{"
       "message=arguments[0];"
       "if(typeof arguments[1]==='function'){cb=arguments[1];}"
       "else{options=arguments[1]||{};cb=arguments[2];}"
       "}"
	       "var p=__moriExtCall('runtime.sendMessage',Object.assign({targetExtensionId:target,message:message,options:options||{}},__moriSourceInfo()));"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
	       "runtime.getPlatformInfo=runtime.getPlatformInfo||function(cb){"
       "var result={os:'mac',arch:'arm',nacl_arch:'arm'};"
       "if(typeof cb==='function')cb(result);"
       "return Promise.resolve(result);"
       "};"
	       "runtime.getBrowserInfo=runtime.getBrowserInfo||function(cb){"
	       "var result={name:hostBrowserInfo.name,vendor:hostBrowserInfo.vendor,version:hostBrowserInfo.version,buildID:hostBrowserInfo.buildID};"
	       "if(typeof cb==='function')cb(result);"
	       "return Promise.resolve(result);"
	       "};"
		      "try{"
		      "if(String(location.protocol)==='mori-extension:'){"
		      "Object.defineProperty(Navigator.prototype,'onLine',{configurable:true,get:function(){return true;}});"
		      "try{Object.defineProperty(navigator,'onLine',{configurable:true,get:function(){return true;}});}catch(_e){}"
		      "setTimeout(function(){try{window.dispatchEvent(new Event('online'));}catch(_e){}},0);"
		      "}"
		      "}catch(e){}"
		       "runtime.setUninstallURL=runtime.setUninstallURL||function(url,cb){"
	       "var p=__moriExtCall('runtime.setUninstallURL',{url:String(url||'')});"
	       "if(typeof cb==='function'){p.then(function(){cb();},function(error){runtime.lastError={message:error&&error.message?error.message:String(error)};try{cb();}finally{setTimeout(function(){delete runtime.lastError;},0);}});}"
	       "return p;"
	       "};"
	       "runtime.requestUpdateCheck=runtime.requestUpdateCheck||function(cb){"
	       "var result='no_update';if(typeof cb==='function')cb(result);return Promise.resolve(result);"
	       "};"
	       "runtime.reload=runtime.reload||function(){location.reload();};"
	       "runtime.getBackgroundPage=runtime.getBackgroundPage||function(cb){"
	       "var result=null;if(typeof cb==='function')cb(result);return Promise.resolve(result);"
	       "};"
	       "runtime.sendNativeMessage=runtime.sendNativeMessage||function(hostName,message,cb){"
	       "var p=__moriExtCall('runtime.sendNativeMessage',{hostName:String(hostName||''),message:message===undefined?null:message});"
	       "if(typeof cb==='function'){p.then(cb,function(error){runtime.lastError={message:error&&error.message?error.message:String(error)};try{cb();}finally{setTimeout(function(){delete runtime.lastError;},0);}});}"
	       "return p;"
	       "};"
	       "runtime.connectNative=runtime.connectNative||function(hostName){"
	       "var portId=extId+':native:'+Date.now()+':'+Math.random().toString(36).slice(2);"
	       "var ports=window.__moriExtPorts=window.__moriExtPorts||{};"
	       "var port=__moriMakePort(portId,String(hostName||''),{id:extId,url:String(location.href)});"
	       "port.postMessage=function(message){"
	       "return __moriExtCall('runtime.nativePortMessage',Object.assign({portId:portId,message:message},__moriSourceInfo())).catch(function(error){"
	       "runtime.lastError={message:error&&error.message?error.message:String(error)};"
	       "try{port.onDisconnect._fire(port);}finally{delete runtime.lastError;delete ports[portId];}"
	       "});"
	       "};"
	       "port.disconnect=function(){"
	       "if(!ports[portId])return;"
	       "delete ports[portId];"
	       "__moriExtCall('runtime.nativePortDisconnect',Object.assign({portId:portId},__moriSourceInfo()));"
	       "port.onDisconnect._fire(port);"
	       "};"
	       "__moriExtCall('runtime.connectNative',Object.assign({hostName:String(hostName||''),portId:portId},__moriSourceInfo())).catch(function(error){"
	       "runtime.lastError={message:error&&error.message?error.message:String(error)};"
	       "try{port.onDisconnect._fire(port);}finally{delete runtime.lastError;delete ports[portId];}"
	       "});"
	       "return port;"
	       "};"
	       "runtime.openOptionsPage=runtime.openOptionsPage||function(cb){"
	       "var p=__moriExtCall('runtime.openOptionsPage',{});"
	       "if(typeof cb==='function')p.then(function(){cb();});"
	       "return p;"
	       "};"
	       "chrome.extension=chrome.extension||{};"
	       "chrome.extension.getURL=chrome.extension.getURL||runtime.getURL;"
	       "chrome.extension.isAllowedFileSchemeAccess=chrome.extension.isAllowedFileSchemeAccess||function(cb){"
	       "if(typeof cb==='function')cb(false);return Promise.resolve(false);"
	       "};"
	       "chrome.extension.isAllowedIncognitoAccess=chrome.extension.isAllowedIncognitoAccess||function(cb){"
	       "if(typeof cb==='function')cb(false);return Promise.resolve(false);"
	       "};"
	       "chrome.identity=chrome.identity||{};"
	       "chrome.identity.getRedirectURL=chrome.identity.getRedirectURL||function(path){"
	       "var clean=String(path||'').replace(/^\\/+/, '');"
	       "return 'https://'+extId+'.chromiumapp.org/'+clean;"
	       "};"
		       "chrome.identity.launchWebAuthFlow=chrome.identity.launchWebAuthFlow||function(details,cb){"
		       "var p=__moriExtCall('identity.launchWebAuthFlow',{details:details||{}});"
		       "if(typeof cb==='function'){p.then(function(result){cb(result);},function(error){runtime.lastError={message:error&&error.message?error.message:String(error)};try{cb();}finally{setTimeout(function(){delete runtime.lastError;},0);}});}"
		       "return p;"
		       "};"
       "chrome.bookmarks=chrome.bookmarks||{};"
       "chrome.bookmarks.onCreated=chrome.bookmarks.onCreated||__moriEvent();"
       "chrome.bookmarks.onRemoved=chrome.bookmarks.onRemoved||__moriEvent();"
       "chrome.bookmarks.onChanged=chrome.bookmarks.onChanged||__moriEvent();"
       "chrome.bookmarks.onMoved=chrome.bookmarks.onMoved||__moriEvent();"
       "chrome.bookmarks.getTree=chrome.bookmarks.getTree||function(cb){"
       "var p=__moriExtCall('bookmarks.getTree',{});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.bookmarks.getChildren=chrome.bookmarks.getChildren||function(id,cb){"
       "var p=__moriExtCall('bookmarks.getChildren',{id:id});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.bookmarks.get=chrome.bookmarks.get||function(idOrIdList,cb){"
       "var p=__moriExtCall('bookmarks.get',{id:idOrIdList});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.bookmarks.search=chrome.bookmarks.search||function(query,cb){"
       "var p=__moriExtCall('bookmarks.search',{query:query});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.bookmarks.create=chrome.bookmarks.create||function(bookmark,cb){"
       "var p=__moriExtCall('bookmarks.create',{bookmark:bookmark||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.bookmarks.update=chrome.bookmarks.update||function(id,changes,cb){"
       "var p=__moriExtCall('bookmarks.update',{id:id,changes:changes||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.bookmarks.move=chrome.bookmarks.move||function(id,destination,cb){"
       "var p=__moriExtCall('bookmarks.move',{id:id,destination:destination||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.bookmarks.remove=chrome.bookmarks.remove||function(id,cb){"
       "var p=__moriExtCall('bookmarks.remove',{id:id});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.bookmarks.removeTree=chrome.bookmarks.removeTree||function(id,cb){"
       "var p=__moriExtCall('bookmarks.removeTree',{id:id});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.contextMenus=chrome.contextMenus||{};"
       "chrome.contextMenus.ACTION_MENU_TOP_LEVEL_LIMIT=6;"
       "chrome.contextMenus.onClicked=chrome.contextMenus.onClicked||__moriEvent();"
       "chrome.contextMenus.create=chrome.contextMenus.create||function(createProperties,cb){"
       "var p=__moriExtCall('contextMenus.create',{createProperties:createProperties||{}});"
       "if(typeof cb==='function')p.then(function(id){cb(id);});"
       "return p;"
       "};"
       "chrome.contextMenus.update=chrome.contextMenus.update||function(id,updateProperties,cb){"
       "var p=__moriExtCall('contextMenus.update',{id:id,updateProperties:updateProperties||{}});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.contextMenus.remove=chrome.contextMenus.remove||function(id,cb){"
       "var p=__moriExtCall('contextMenus.remove',{id:id});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.contextMenus.removeAll=chrome.contextMenus.removeAll||function(cb){"
       "var p=__moriExtCall('contextMenus.removeAll',{});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.menus=chrome.menus||chrome.contextMenus;"
       "chrome.storage=chrome.storage||{};"
       "chrome.storage.onChanged=chrome.storage.onChanged||__moriEvent();"
	       "chrome.storage.local=chrome.storage.local||{};"
	       "chrome.storage.sync=chrome.storage.sync||{};"
	       "chrome.storage.session=chrome.storage.session||{};"
	       "chrome.storage.managed=chrome.storage.managed||{};"
	       "chrome.tabs=chrome.tabs||{};"
       "chrome.tabs.onCreated=chrome.tabs.onCreated||__moriEvent();"
       "chrome.tabs.onUpdated=chrome.tabs.onUpdated||__moriEvent();"
       "chrome.tabs.onActivated=chrome.tabs.onActivated||__moriEvent();"
       "chrome.tabs.onHighlighted=chrome.tabs.onHighlighted||__moriEvent();"
       "chrome.tabs.onRemoved=chrome.tabs.onRemoved||__moriEvent();"
       "chrome.tabs.onMoved=chrome.tabs.onMoved||__moriEvent();"
       "chrome.windows=chrome.windows||{};"
       "chrome.windows.WINDOW_ID_NONE=-1;"
       "chrome.windows.WINDOW_ID_CURRENT=-2;"
       "chrome.windows.onCreated=chrome.windows.onCreated||__moriEvent();"
	       "chrome.windows.onRemoved=chrome.windows.onRemoved||__moriEvent();"
	       "chrome.windows.onFocusChanged=chrome.windows.onFocusChanged||__moriEvent();"
	       "chrome.idle=chrome.idle||{};"
	       "chrome.idle.IdleState=chrome.idle.IdleState||{ACTIVE:'active',IDLE:'idle',LOCKED:'locked'};"
	       "chrome.idle.onStateChanged=chrome.idle.onStateChanged||__moriEvent();"
	       "chrome.idle.queryState=chrome.idle.queryState||function(detectionIntervalInSeconds,cb){"
	       "var result='active';if(typeof cb==='function')cb(result);return Promise.resolve(result);"
	       "};"
	       "chrome.idle.setDetectionInterval=chrome.idle.setDetectionInterval||function(){return Promise.resolve();};"
	       "chrome.management=chrome.management||{};"
	       "chrome.management.getSelf=chrome.management.getSelf||function(cb){var p=__moriExtCall('management.getSelf',{});if(typeof cb==='function')p.then(cb);return p;};"
		       "chrome.management.get=chrome.management.get||function(id,cb){var p=__moriExtCall('management.get',{id:id});if(typeof cb==='function')p.then(cb);return p;};"
		       "chrome.management.getAll=chrome.management.getAll||function(cb){var p=__moriExtCall('management.getAll',{});if(typeof cb==='function')p.then(cb);return p;};"
		       "chrome.management.setEnabled=chrome.management.setEnabled||function(id,enabled,cb){var p=__moriExtCall('management.setEnabled',{id:id,enabled:!!enabled});if(typeof cb==='function')p.then(function(){cb();});return p;};"
		       "chrome.management.uninstall=chrome.management.uninstall||function(id,options,cb){if(typeof options==='function'){cb=options;options={};}var p=__moriExtCall('management.uninstall',{id:String(id||''),options:options||{}});if(typeof cb==='function')p.then(function(){cb();});return p;};"
		       "chrome.management.uninstallSelf=chrome.management.uninstallSelf||function(options,cb){if(typeof options==='function'){cb=options;options={};}var p=__moriExtCall('management.uninstallSelf',{options:options||{}});if(typeof cb==='function')p.then(function(){cb();});return p;};"
	       "chrome.notifications=chrome.notifications||{};"
	       "chrome.notifications.onClosed=chrome.notifications.onClosed||__moriEvent();"
	       "chrome.notifications.onClicked=chrome.notifications.onClicked||__moriEvent();"
	       "chrome.notifications.onButtonClicked=chrome.notifications.onButtonClicked||__moriEvent();"
	       "chrome.notifications.create=chrome.notifications.create||function(idOrOptions,options,cb){"
	       "var id='',opts={};"
	       "if(typeof idOrOptions==='string'){id=idOrOptions;opts=options||{};}"
	       "else{opts=idOrOptions||{};cb=options;}"
	       "var p=__moriExtCall('notifications.create',{id:id,options:opts});"
	       "if(typeof cb==='function')p.then(cb);return p;"
	       "};"
	       "chrome.notifications.update=chrome.notifications.update||function(id,options,cb){var p=__moriExtCall('notifications.update',{id:id,options:options||{}});if(typeof cb==='function')p.then(cb);return p;};"
	       "chrome.notifications.clear=chrome.notifications.clear||function(id,cb){var p=__moriExtCall('notifications.clear',{id:id});if(typeof cb==='function')p.then(cb);return p;};"
	       "chrome.notifications.getAll=chrome.notifications.getAll||function(cb){var p=__moriExtCall('notifications.getAll',{});if(typeof cb==='function')p.then(cb);return p;};"
	       "chrome.notifications.getPermissionLevel=chrome.notifications.getPermissionLevel||function(cb){var p=__moriExtCall('notifications.getPermissionLevel',{});if(typeof cb==='function')p.then(cb);return p;};"
	       "chrome.topSites=chrome.topSites||{};"
	       "chrome.topSites.get=chrome.topSites.get||function(cb){var p=__moriExtCall('topSites.get',{});if(typeof cb==='function')p.then(cb);return p;};"
	       "chrome.history=chrome.history||{};"
	       "chrome.history.onVisited=chrome.history.onVisited||__moriEvent();"
	       "chrome.history.onVisitRemoved=chrome.history.onVisitRemoved||__moriEvent();"
	       "chrome.history.search=chrome.history.search||function(query,cb){var p=__moriExtCall('history.search',{query:query||{}});if(typeof cb==='function')p.then(cb);return p;};"
	       "chrome.history.getVisits=chrome.history.getVisits||function(details,cb){var p=__moriExtCall('history.getVisits',{details:details||{}});if(typeof cb==='function')p.then(cb);return p;};"
	       "chrome.history.addUrl=chrome.history.addUrl||function(details,cb){var p=__moriExtCall('history.addUrl',{details:details||{}});if(typeof cb==='function')p.then(function(){cb();});return p;};"
	       "chrome.history.deleteUrl=chrome.history.deleteUrl||function(details,cb){var p=__moriExtCall('history.deleteUrl',{details:details||{}});if(typeof cb==='function')p.then(function(){cb();});return p;};"
	       "chrome.history.deleteRange=chrome.history.deleteRange||function(range,cb){var p=__moriExtCall('history.deleteRange',{range:range||{}});if(typeof cb==='function')p.then(function(){cb();});return p;};"
	       "chrome.history.deleteAll=chrome.history.deleteAll||function(cb){var p=__moriExtCall('history.deleteAll',{});if(typeof cb==='function')p.then(function(){cb();});return p;};"
	       "chrome.browsingData=chrome.browsingData||{};"
	       "chrome.browsingData.settings=chrome.browsingData.settings||function(cb){"
	       "var result={options:{since:0},dataToRemove:{cache:true,cookies:true,downloads:true,formData:true,history:true,localStorage:true,passwords:false,pluginData:false},dataRemovalPermitted:{cache:false,cookies:true,downloads:true,formData:true,history:true,localStorage:true,passwords:false,pluginData:false}};"
	       "if(typeof cb==='function')cb(result);return Promise.resolve(result);"
	       "};"
	       "function __moriBrowsingDataCall(method,options,dataToRemove,cb){"
	       "if(typeof options==='function'){cb=options;options={};dataToRemove={};}"
	       "else if(typeof dataToRemove==='function'){cb=dataToRemove;dataToRemove={};}"
	       "var p=__moriExtCall(method,{options:options||{},dataToRemove:dataToRemove||{}});"
	       "if(typeof cb==='function')p.then(function(){cb();});return p;"
	       "}"
	       "chrome.browsingData.remove=chrome.browsingData.remove||function(options,dataToRemove,cb){return __moriBrowsingDataCall('browsingData.remove',options,dataToRemove,cb);};"
	       "chrome.browsingData.removeCache=chrome.browsingData.removeCache||function(options,cb){return __moriBrowsingDataCall('browsingData.removeCache',options,{cache:true},cb);};"
	       "chrome.browsingData.removeCookies=chrome.browsingData.removeCookies||function(options,cb){return __moriBrowsingDataCall('browsingData.removeCookies',options,{cookies:true},cb);};"
	       "chrome.browsingData.removeDownloads=chrome.browsingData.removeDownloads||function(options,cb){return __moriBrowsingDataCall('browsingData.removeDownloads',options,{downloads:true},cb);};"
	       "chrome.browsingData.removeFormData=chrome.browsingData.removeFormData||function(options,cb){return __moriBrowsingDataCall('browsingData.removeFormData',options,{formData:true},cb);};"
	       "chrome.browsingData.removeHistory=chrome.browsingData.removeHistory||function(options,cb){return __moriBrowsingDataCall('browsingData.removeHistory',options,{history:true},cb);};"
	       "chrome.browsingData.removeLocalStorage=chrome.browsingData.removeLocalStorage||function(options,cb){return __moriBrowsingDataCall('browsingData.removeLocalStorage',options,{localStorage:true},cb);};"
	       "chrome.browsingData.removePasswords=chrome.browsingData.removePasswords||function(options,cb){return __moriBrowsingDataCall('browsingData.removePasswords',options,{passwords:true},cb);};"
	       "chrome.browsingData.removePluginData=chrome.browsingData.removePluginData||function(options,cb){return __moriBrowsingDataCall('browsingData.removePluginData',options,{pluginData:true},cb);};"
	       "chrome.sessions=chrome.sessions||{};"
	       "chrome.sessions.MAX_SESSION_RESULTS=25;"
	       "chrome.sessions.onChanged=chrome.sessions.onChanged||__moriEvent();"
	       "chrome.sessions.getRecentlyClosed=chrome.sessions.getRecentlyClosed||function(filter,cb){if(typeof filter==='function'){cb=filter;filter={};}var p=__moriExtCall('sessions.getRecentlyClosed',{filter:filter||{}});if(typeof cb==='function')p.then(cb);return p;};"
	       "chrome.sessions.getDevices=chrome.sessions.getDevices||function(filter,cb){if(typeof filter==='function'){cb=filter;filter={};}var p=__moriExtCall('sessions.getDevices',{filter:filter||{}});if(typeof cb==='function')p.then(cb);return p;};"
	       "chrome.sessions.restore=chrome.sessions.restore||function(sessionId,cb){if(typeof sessionId==='function'){cb=sessionId;sessionId='';}var p=__moriExtCall('sessions.restore',{sessionId:sessionId||''});if(typeof cb==='function')p.then(cb);return p;};"
	       "chrome.webNavigation=chrome.webNavigation||{};"
       "chrome.webNavigation.onBeforeNavigate=chrome.webNavigation.onBeforeNavigate||__moriEvent();"
	       "chrome.webNavigation.onCommitted=chrome.webNavigation.onCommitted||__moriEvent();"
	       "chrome.webNavigation.onDOMContentLoaded=chrome.webNavigation.onDOMContentLoaded||__moriEvent();"
	       "chrome.webNavigation.onCompleted=chrome.webNavigation.onCompleted||__moriEvent();"
	       "chrome.webNavigation.onHistoryStateUpdated=chrome.webNavigation.onHistoryStateUpdated||__moriEvent();"
	       "chrome.webNavigation.onReferenceFragmentUpdated=chrome.webNavigation.onReferenceFragmentUpdated||__moriEvent();"
	       "chrome.webNavigation.onErrorOccurred=chrome.webNavigation.onErrorOccurred||__moriEvent();"
	       "chrome.webNavigation.getFrame=chrome.webNavigation.getFrame||function(details,cb){"
	       "details=details||{};"
	       "var p=__moriExtCall('tabs.get',{tabId:details.tabId}).then(function(tab){return {"
	       "errorOccurred:false,tabId:Number(details.tabId)||0,frameId:Number(details.frameId)||0,parentFrameId:-1,url:(tab&&tab.url)||''};});"
	       "if(typeof cb==='function')p.then(cb);return p;"
	       "};"
	       "chrome.webNavigation.getAllFrames=chrome.webNavigation.getAllFrames||function(details,cb){"
	       "details=details||{};"
	       "var p=chrome.webNavigation.getFrame({tabId:details.tabId,frameId:0}).then(function(frame){return [frame];});"
	       "if(typeof cb==='function')p.then(cb);return p;"
	       "};"
	       "chrome.webRequest=chrome.webRequest||{};"
	       "chrome.webRequest.onBeforeRequest=chrome.webRequest.onBeforeRequest||__moriEvent();"
	       "chrome.webRequest.onBeforeSendHeaders=chrome.webRequest.onBeforeSendHeaders||__moriEvent();"
	       "chrome.webRequest.onHeadersReceived=chrome.webRequest.onHeadersReceived||__moriEvent();"
	       "chrome.webRequest.onBeforeRedirect=chrome.webRequest.onBeforeRedirect||__moriEvent();"
	       "chrome.webRequest.onAuthRequired=chrome.webRequest.onAuthRequired||__moriEvent();"
	       "chrome.webRequest.onCompleted=chrome.webRequest.onCompleted||__moriEvent();"
       "chrome.webRequest.onErrorOccurred=chrome.webRequest.onErrorOccurred||__moriEvent();"
       "chrome.webRequest.handlerBehaviorChanged=chrome.webRequest.handlerBehaviorChanged||function(cb){"
       "if(typeof cb==='function')cb();return Promise.resolve();"
       "};"
       "chrome.cookies=chrome.cookies||{};"
       "chrome.cookies.onChanged=chrome.cookies.onChanged||__moriEvent();"
       "chrome.downloads=chrome.downloads||{};"
       "chrome.downloads.onCreated=chrome.downloads.onCreated||__moriEvent();"
       "chrome.downloads.onChanged=chrome.downloads.onChanged||__moriEvent();"
       "chrome.downloads.onErased=chrome.downloads.onErased||__moriEvent();"
	       "chrome.commands=chrome.commands||{};"
	       "chrome.commands.onCommand=chrome.commands.onCommand||__moriEvent();"
	       "chrome.commands.getAll=chrome.commands.getAll||function(cb){"
	       "var commands=manifest.commands||{};"
       "var result=Object.keys(commands).map(function(name){"
       "var info=commands[name]||{};"
       "var suggested=info.suggested_key||{};"
       "var shortcut=typeof suggested==='string'?suggested:(suggested.mac||suggested.default||'');"
       "return {name:name,description:info.description||'',shortcut:shortcut};"
       "});"
	       "if(typeof cb==='function')cb(result);"
	       "return Promise.resolve(result);"
	       "};"
	       "chrome.permissions=chrome.permissions||{};"
	       "chrome.permissions.onAdded=chrome.permissions.onAdded||__moriEvent();"
	       "chrome.permissions.onRemoved=chrome.permissions.onRemoved||__moriEvent();"
	       "chrome.permissions.contains=chrome.permissions.contains||function(permissions,cb){"
	       "var p=__moriExtCall('permissions.contains',{permissions:permissions||{}});"
	       "if(typeof cb==='function')p.then(cb);return p;"
	       "};"
	       "chrome.permissions.getAll=chrome.permissions.getAll||function(cb){"
	       "var p=__moriExtCall('permissions.getAll',{});"
	       "if(typeof cb==='function')p.then(cb);return p;"
	       "};"
	       "chrome.permissions.request=chrome.permissions.request||function(permissions,cb){"
	       "var p=__moriExtCall('permissions.request',{permissions:permissions||{}});"
	       "if(typeof cb==='function')p.then(cb);return p;"
	       "};"
	       "chrome.permissions.remove=chrome.permissions.remove||function(permissions,cb){"
	       "var p=__moriExtCall('permissions.remove',{permissions:permissions||{}});"
	       "if(typeof cb==='function')p.then(cb);return p;"
	       "};"
	       "function __moriPrivacySetting(value){return {"
	       "get:function(details,cb){var result={levelOfControl:'not_controllable',value:value};if(typeof cb==='function')cb(result);return Promise.resolve(result);},"
	       "set:function(details,cb){if(typeof cb==='function')cb();return Promise.resolve();},"
	       "clear:function(details,cb){if(typeof cb==='function')cb();return Promise.resolve();}"
	       "};}"
	       "chrome.privacy=chrome.privacy||{};"
	       "chrome.privacy.network=chrome.privacy.network||{};"
	       "chrome.privacy.network.networkPredictionEnabled=chrome.privacy.network.networkPredictionEnabled||__moriPrivacySetting(false);"
	       "chrome.privacy.services=chrome.privacy.services||{};"
	       "chrome.privacy.services.passwordSavingEnabled=chrome.privacy.services.passwordSavingEnabled||__moriPrivacySetting(false);"
	       "chrome.privacy.services.autofillEnabled=chrome.privacy.services.autofillEnabled||__moriPrivacySetting(false);"
	       "chrome.privacy.websites=chrome.privacy.websites||{};"
	       "chrome.privacy.websites.thirdPartyCookiesAllowed=chrome.privacy.websites.thirdPartyCookiesAllowed||__moriPrivacySetting(true);"
	       "chrome.privacy.websites.hyperlinkAuditingEnabled=chrome.privacy.websites.hyperlinkAuditingEnabled||__moriPrivacySetting(false);"
	       "chrome.proxy=chrome.proxy||{};"
	       "chrome.proxy.settings=chrome.proxy.settings||{"
	       "onChange:__moriEvent(),"
	       "get:function(details,cb){var p=__moriExtCall('proxy.settings.get',{details:details||{}});if(typeof cb==='function')p.then(cb);return p;},"
	       "set:function(details,cb){var p=__moriExtCall('proxy.settings.set',{details:details||{}});if(typeof cb==='function')p.then(function(){cb();});return p;},"
	       "clear:function(details,cb){var p=__moriExtCall('proxy.settings.clear',{details:details||{}});if(typeof cb==='function')p.then(function(){cb();});return p;}"
	       "};"
		       "runtime.ContextType=runtime.ContextType||{"
		       "BACKGROUND:'BACKGROUND',POPUP:'POPUP',OFFSCREEN_DOCUMENT:'OFFSCREEN_DOCUMENT',TAB:'TAB'};"
		       "function __moriLocalExtensionContexts(filter){"
		       "filter=filter||{};var contexts=[];var href=String(location.href);"
		       "var path=(location.pathname||'').replace(/^\\/+/, '');"
		       "try{path=decodeURIComponent(path);}catch(e){}"
		       "function allowed(type){return !Array.isArray(filter.contextTypes)||filter.contextTypes.indexOf(type)>=0;}"
		       "function urlAllowed(url){return !Array.isArray(filter.documentUrls)||filter.documentUrls.indexOf(url)>=0;}"
		       "function idAllowed(id){return !Array.isArray(filter.contextIds)||filter.contextIds.indexOf(id)>=0;}"
		       "function popupPath(){var a=(manifest&&(manifest.action||manifest.browser_action||manifest.page_action))||{};"
		       "var p=String(a.default_popup||'').replace(/^\\/+/, '');try{return decodeURIComponent(p);}catch(e){return p;}}"
	       "function currentType(){"
	       "if(document.documentElement&&document.documentElement.dataset.moriExtensionBackground==='true')return 'BACKGROUND';"
	       "if(path==='offscreen.html')return 'OFFSCREEN_DOCUMENT';"
	       "if(location.protocol==='mori-extension:'&&popupPath()&&path===popupPath())return 'POPUP';"
	       "return 'TAB';"
	       "}"
		       "function pushContext(type,id,url,frameId){if(allowed(type)&&urlAllowed(url)&&idAllowed(id)){"
		       "contexts.push({contextId:id,contextType:type,documentUrl:url,frameId:frameId||0,incognito:false});}}"
	       "var type=currentType();"
	       "if(type==='BACKGROUND'){"
	       "pushContext('BACKGROUND','background:'+extId,href,0);"
	       "}else if(type==='OFFSCREEN_DOCUMENT'){"
	       "pushContext('OFFSCREEN_DOCUMENT','offscreen:'+extId,href,0);"
	       "}else{"
	       "pushContext(type,type.toLowerCase()+':'+extId+':'+href,href,0);"
	       "}"
	       "var frames=document.querySelectorAll('iframe[data-mori-offscreen-extension=\"'+extId+'\"]');"
		       "frames.forEach(function(frame,index){var url=frame.src||'';if(allowed('OFFSCREEN_DOCUMENT')&&urlAllowed(url)){"
		       "contexts.push({contextId:'offscreen:'+extId+':'+index,contextType:'OFFSCREEN_DOCUMENT',documentUrl:url,frameId:index+1,incognito:false});"
		       "}});"
		       "return contexts;"
		       "}"
		       "runtime.getContexts=runtime.getContexts||function(filter,cb){"
		       "filter=filter||{};var local=__moriLocalExtensionContexts(filter);"
		       "var p=__moriExtCall('runtime.getContexts',{filter:filter}).then(function(nativeContexts){"
		       "var seen={},merged=[];"
		       "function add(context){if(!context)return;var key=String(context.contextId||'')+'|'+String(context.contextType||'')+'|'+String(context.documentUrl||'');"
		       "if(seen[key])return;seen[key]=true;merged.push(context);}"
		       "(Array.isArray(nativeContexts)?nativeContexts:[]).forEach(add);local.forEach(add);return merged;"
		       "},function(){return local;});"
		       "if(typeof cb==='function')p.then(cb);return p;"
		       "};"
	       "chrome.offscreen=chrome.offscreen||{};"
	       "chrome.offscreen.Reason=chrome.offscreen.Reason||{"
	       "TESTING:'TESTING',AUDIO_PLAYBACK:'AUDIO_PLAYBACK',IFRAME_SCRIPTING:'IFRAME_SCRIPTING',DOM_SCRAPING:'DOM_SCRAPING',BLOBS:'BLOBS',DOM_PARSER:'DOM_PARSER',USER_MEDIA:'USER_MEDIA',DISPLAY_MEDIA:'DISPLAY_MEDIA',WEB_RTC:'WEB_RTC',CLIPBOARD:'CLIPBOARD',LOCAL_STORAGE:'LOCAL_STORAGE',WORKERS:'WORKERS',BATTERY_STATUS:'BATTERY_STATUS',MATCH_MEDIA:'MATCH_MEDIA',GEOLOCATION:'GEOLOCATION'};"
	       "function __moriOffscreenRoot(){"
	       "var id='__mori_offscreen_documents__';var root=document.getElementById(id);"
	       "if(!root){root=document.createElement('div');root.id=id;root.style.cssText='display:none!important;width:0;height:0;overflow:hidden';"
	       "(document.body||document.documentElement).appendChild(root);}"
	       "return root;"
	       "}"
	       "function __moriOffscreenFrames(){return Array.prototype.slice.call(document.querySelectorAll('iframe[data-mori-offscreen-extension=\"'+extId+'\"]'));}"
	       "chrome.offscreen.hasDocument=chrome.offscreen.hasDocument||function(cb){"
	       "var result=__moriOffscreenFrames().length>0;"
	       "if(typeof cb==='function')cb(result);return Promise.resolve(result);"
	       "};"
	       "chrome.offscreen.createDocument=chrome.offscreen.createDocument||function(details,cb){"
	       "details=details||{};var raw=String(details.url||'offscreen.html');"
	       "var url=raw.indexOf('://')>=0?raw:runtime.getURL(raw);"
	       "var existing=__moriOffscreenFrames().filter(function(frame){return frame.src===url;})[0];"
	       "var p=existing?Promise.resolve():new Promise(function(resolve,reject){"
	       "var frame=document.createElement('iframe');"
	       "frame.dataset.moriOffscreenExtension=extId;frame.dataset.moriOffscreenReason=JSON.stringify(details.reasons||[]);"
	       "frame.allow='clipboard-read; clipboard-write';"
	       "frame.style.cssText='position:absolute;width:0;height:0;border:0;opacity:0;pointer-events:none';"
	       "frame.onload=function(){resolve();};frame.onerror=function(){reject(new Error('Failed to load offscreen document'));};"
	       "frame.src=url;__moriOffscreenRoot().appendChild(frame);"
	       "setTimeout(resolve,500);"
	       "});"
	       "if(typeof cb==='function')p.then(function(){cb();});return p;"
	       "};"
	       "chrome.offscreen.closeDocument=chrome.offscreen.closeDocument||function(cb){"
	       "__moriOffscreenFrames().forEach(function(frame){frame.remove();});"
	       "if(typeof cb==='function')cb();return Promise.resolve();"
	       "};"
	       "chrome.scripting=chrome.scripting||{};"
	       "chrome.declarativeNetRequest=chrome.declarativeNetRequest||{};"
       "chrome.declarativeNetRequest.getEnabledRulesets=chrome.declarativeNetRequest.getEnabledRulesets||function(cb){"
       "var p=__moriExtCall('declarativeNetRequest.getEnabledRulesets',{});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.declarativeNetRequest.updateEnabledRulesets=chrome.declarativeNetRequest.updateEnabledRulesets||function(details,cb){"
       "var p=__moriExtCall('declarativeNetRequest.updateEnabledRulesets',{details:details||{}});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.declarativeNetRequest.getAvailableStaticRuleCount=chrome.declarativeNetRequest.getAvailableStaticRuleCount||function(cb){"
       "var p=__moriExtCall('declarativeNetRequest.getAvailableStaticRuleCount',{});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.declarativeNetRequest.getDynamicRules=chrome.declarativeNetRequest.getDynamicRules||function(filter,cb){"
       "if(typeof filter==='function'){cb=filter;filter={};}"
       "var p=__moriExtCall('declarativeNetRequest.getDynamicRules',{filter:filter||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.declarativeNetRequest.updateDynamicRules=chrome.declarativeNetRequest.updateDynamicRules||function(details,cb){"
       "var p=__moriExtCall('declarativeNetRequest.updateDynamicRules',{details:details||{}});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.declarativeNetRequest.getSessionRules=chrome.declarativeNetRequest.getSessionRules||function(filter,cb){"
       "if(typeof filter==='function'){cb=filter;filter={};}"
       "var p=__moriExtCall('declarativeNetRequest.getSessionRules',{filter:filter||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.declarativeNetRequest.updateSessionRules=chrome.declarativeNetRequest.updateSessionRules||function(details,cb){"
       "var p=__moriExtCall('declarativeNetRequest.updateSessionRules',{details:details||{}});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.declarativeNetRequest.isRegexSupported=chrome.declarativeNetRequest.isRegexSupported||function(info,cb){"
       "var p=__moriExtCall('declarativeNetRequest.isRegexSupported',{details:info||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.alarms=chrome.alarms||{};"
       "chrome.alarms.onAlarm=chrome.alarms.onAlarm||__moriEvent();"
       "chrome.action=chrome.action||{};"
       "chrome.action.onClicked=chrome.action.onClicked||__moriEvent();"
       "function __moriActionCall(name,details,cb){"
       "var p=__moriExtCall('action.'+name,{details:details||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "}"
       "chrome.action.setBadgeText=chrome.action.setBadgeText||function(details,cb){"
       "return __moriActionCall('setBadgeText',details,function(){cb&&cb();});"
       "};"
       "chrome.action.getBadgeText=chrome.action.getBadgeText||function(details,cb){"
       "return __moriActionCall('getBadgeText',details,cb);"
       "};"
       "chrome.action.setTitle=chrome.action.setTitle||function(details,cb){"
       "return __moriActionCall('setTitle',details,function(){cb&&cb();});"
       "};"
       "chrome.action.getTitle=chrome.action.getTitle||function(details,cb){"
       "return __moriActionCall('getTitle',details,cb);"
       "};"
       "chrome.action.setPopup=chrome.action.setPopup||function(details,cb){"
       "return __moriActionCall('setPopup',details,function(){cb&&cb();});"
       "};"
       "chrome.action.getPopup=chrome.action.getPopup||function(details,cb){"
       "return __moriActionCall('getPopup',details,cb);"
       "};"
       "chrome.action.enable=chrome.action.enable||function(tabId,cb){"
       "if(typeof tabId==='function'){cb=tabId;tabId=null;}"
       "return __moriActionCall('enable',{tabId:tabId},function(){cb&&cb();});"
       "};"
       "chrome.action.disable=chrome.action.disable||function(tabId,cb){"
       "if(typeof tabId==='function'){cb=tabId;tabId=null;}"
       "return __moriActionCall('disable',{tabId:tabId},function(){cb&&cb();});"
       "};"
       "chrome.action.isEnabled=chrome.action.isEnabled||function(details,cb){"
       "if(typeof details==='function'){cb=details;details={};}"
       "return __moriActionCall('isEnabled',details||{},cb);"
       "};"
       "chrome.action.openPopup=chrome.action.openPopup||function(cb){"
       "return __moriActionCall('openPopup',{},function(){cb&&cb();});"
       "};"
       "chrome.action.getUserSettings=chrome.action.getUserSettings||function(cb){"
       "return __moriActionCall('getUserSettings',{},cb);"
       "};"
       "chrome.action.setBadgeBackgroundColor=chrome.action.setBadgeBackgroundColor||function(details,cb){"
       "return __moriActionCall('setBadgeBackgroundColor',details,function(){cb&&cb();});"
       "};"
       "chrome.action.getBadgeBackgroundColor=chrome.action.getBadgeBackgroundColor||function(details,cb){"
       "if(typeof details==='function'){cb=details;details={};}"
       "return __moriActionCall('getBadgeBackgroundColor',details||{},cb);"
       "};"
       "chrome.action.setBadgeTextColor=chrome.action.setBadgeTextColor||function(details,cb){"
       "return __moriActionCall('setBadgeTextColor',details,function(){cb&&cb();});"
       "};"
       "chrome.action.getBadgeTextColor=chrome.action.getBadgeTextColor||function(details,cb){"
       "if(typeof details==='function'){cb=details;details={};}"
       "return __moriActionCall('getBadgeTextColor',details||{},cb);"
       "};"
       "chrome.action.setIcon=chrome.action.setIcon||function(details,cb){"
       "return __moriActionCall('setIcon',details,function(){cb&&cb();});"
       "};"
       "chrome.browserAction=chrome.browserAction||chrome.action;"
       "chrome.pageAction=chrome.pageAction||chrome.action;"
       "chrome.sidePanel=chrome.sidePanel||{};"
       "function __moriSidePanelCall(name,args,cb){"
       "var p=__moriExtCall('sidePanel.'+name,args||{});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "}"
       "chrome.sidePanel.setOptions=chrome.sidePanel.setOptions||function(options,cb){"
       "return __moriSidePanelCall('setOptions',{details:options||{}},function(){cb&&cb();});"
       "};"
       "chrome.sidePanel.getOptions=chrome.sidePanel.getOptions||function(options,cb){"
       "if(typeof options==='function'){cb=options;options={};}"
       "return __moriSidePanelCall('getOptions',{details:options||{}},cb);"
       "};"
       "chrome.sidePanel.open=chrome.sidePanel.open||function(options,cb){"
       "if(typeof options==='function'){cb=options;options={};}"
       "return __moriSidePanelCall('open',{details:options||{}},function(){cb&&cb();});"
       "};"
       "chrome.sidePanel.close=chrome.sidePanel.close||function(options,cb){"
       "if(typeof options==='function'){cb=options;options={};}"
       "return __moriSidePanelCall('close',{details:options||{}},function(){cb&&cb();});"
       "};"
       "chrome.sidePanel.setPanelBehavior=chrome.sidePanel.setPanelBehavior||function(behavior,cb){"
       "return __moriSidePanelCall('setPanelBehavior',{behavior:behavior||{}},function(){cb&&cb();});"
       "};"
       "chrome.sidePanel.getPanelBehavior=chrome.sidePanel.getPanelBehavior||function(cb){"
       "return __moriSidePanelCall('getPanelBehavior',{},cb);"
       "};"
       "var __moriAlarms=window.__moriAlarms=window.__moriAlarms||{};"
       "function __moriAlarmClone(alarm){return alarm?{name:alarm.name,scheduledTime:alarm.scheduledTime,periodInMinutes:alarm.periodInMinutes}:undefined;}"
       "function __moriAlarmSchedule(alarm){"
       "if(alarm.timer)clearTimeout(alarm.timer);"
       "var delay=Math.max(0,alarm.scheduledTime-Date.now());"
       "alarm.timer=setTimeout(function(){"
       "chrome.alarms.onAlarm._fire(__moriAlarmClone(alarm));"
       "if(alarm.periodInMinutes){alarm.scheduledTime=Date.now()+alarm.periodInMinutes*60000;__moriAlarmSchedule(alarm);}"
       "else{delete __moriAlarms[alarm.name];}"
       "},delay);"
       "}"
       "chrome.alarms.create=chrome.alarms.create||function(nameOrInfo,alarmInfo){"
       "var name='',info=alarmInfo||{};"
       "if(typeof nameOrInfo==='string'){name=nameOrInfo;}"
       "else{info=nameOrInfo||{};}"
       "var period=Number(info.periodInMinutes)||0;"
       "var when=Number(info.when)||0;"
       "if(!when){when=Date.now()+((Number(info.delayInMinutes)||period||1)*60000);}"
       "if(__moriAlarms[name]&&__moriAlarms[name].timer)clearTimeout(__moriAlarms[name].timer);"
       "var alarm=__moriAlarms[name]={name:name,scheduledTime:when,periodInMinutes:period||undefined};"
       "__moriAlarmSchedule(alarm);"
       "return Promise.resolve();"
       "};"
       "chrome.alarms.get=chrome.alarms.get||function(name,cb){"
       "if(typeof name==='function'){cb=name;name='';}"
       "var result=__moriAlarmClone(__moriAlarms[name||'']);"
       "if(typeof cb==='function')cb(result);"
       "return Promise.resolve(result);"
       "};"
       "chrome.alarms.getAll=chrome.alarms.getAll||function(cb){"
       "var result=Object.keys(__moriAlarms).map(function(k){return __moriAlarmClone(__moriAlarms[k]);});"
       "if(typeof cb==='function')cb(result);"
       "return Promise.resolve(result);"
       "};"
       "chrome.alarms.clear=chrome.alarms.clear||function(name,cb){"
       "if(typeof name==='function'){cb=name;name='';}"
       "name=name||'';"
       "var existed=!!__moriAlarms[name];"
       "if(existed){clearTimeout(__moriAlarms[name].timer);delete __moriAlarms[name];}"
       "if(typeof cb==='function')cb(existed);"
       "return Promise.resolve(existed);"
       "};"
       "chrome.alarms.clearAll=chrome.alarms.clearAll||function(cb){"
       "var keys=Object.keys(__moriAlarms);"
       "keys.forEach(function(k){clearTimeout(__moriAlarms[k].timer);delete __moriAlarms[k];});"
       "var result=keys.length>0;"
       "if(typeof cb==='function')cb(result);"
       "return Promise.resolve(result);"
       "};"
       "function __moriExtCall(method,args){"
       "var rid=extId+':'+Date.now()+':'+Math.random().toString(36).slice(2);"
       "window.__moriExtCallbacks=window.__moriExtCallbacks||{};"
       "var promise=new Promise(function(resolve,reject){"
       "window.__moriExtCallbacks[rid]={resolve:resolve,reject:reject};"
       "});"
       "__moriNativeConsoleInfo('__MORI_EXTENSION__'+JSON.stringify({"
       "requestId:rid,extensionId:extId,method:method,args:args||{}"
       "}));"
       "return promise;"
       "}"
       "window.__moriExtResolve=window.__moriExtResolve||function(response){"
       "var cb=window.__moriExtCallbacks&&window.__moriExtCallbacks[response.requestId];"
       "if(response.extensionId===extId&&response.storageChange){"
       "chrome.storage.onChanged._fire(response.storageChange,response.storageArea||'local');"
       "}"
       "if(response.extensionId===extId&&response.runtimeMessage){"
       "runtime.onMessage._fire(response.runtimeMessage,{id:extId,url:String(location.href)},function(){});"
       "}"
       "if(!cb)return;"
       "if(response.deferred)return;"
       "delete window.__moriExtCallbacks[response.requestId];"
       "if(response.error)cb.reject(new Error(response.error));"
       // A soft decline (no context answered) resolves to undefined, matching
       // Chrome's runtime.sendMessage contract, so callers that branch on an
       // undefined reply behave correctly instead of seeing a literal null.
       "else if(response.noResponse)cb.resolve(undefined);"
       "else cb.resolve(response.result);"
       "};"
       "if(String(location.protocol)==='mori-extension:'&&!globalThis.__moriClipboardWrapped){"
       "globalThis.__moriClipboardWrapped=true;"
       "var __moriClipboardCache='';"
       "var __moriClipboardCacheReady=false;"
       "function __moriClipboardRefresh(){"
       "return __moriExtCall('clipboard.readText',{}).then(function(text){"
       "__moriClipboardCache=String(text||'');__moriClipboardCacheReady=true;return __moriClipboardCache;"
       "},function(){return __moriClipboardCache;});"
       "}"
       "var __moriClipboardApi={"
       "readText:function(){return __moriClipboardRefresh();},"
       "writeText:function(text){__moriClipboardCache=String(text||'');__moriClipboardCacheReady=true;"
       "return __moriExtCall('clipboard.writeText',{text:__moriClipboardCache}).then(function(){});}"
       "};"
       "try{Object.defineProperty(Navigator.prototype,'clipboard',{configurable:true,get:function(){return __moriClipboardApi;}});}catch(_e){}"
       "try{Object.defineProperty(navigator,'clipboard',{configurable:true,value:__moriClipboardApi});}catch(_e){}"
       "setTimeout(function(){__moriClipboardRefresh();},0);"
       "addEventListener('focus',function(){__moriClipboardRefresh();},true);"
       "var __moriOriginalExecCommand=document.execCommand&&document.execCommand.bind(document);"
       "var __moriClipboardLastEditable=null;"
       "function __moriClipboardRemember(el){"
       "if(!el)return null;var tag=String(el.tagName||'').toUpperCase();"
       "if(tag==='TEXTAREA'||tag==='INPUT'||el.isContentEditable){__moriClipboardLastEditable=el;return el;}"
       "return null;"
       "}"
       "try{var __moriOriginalFocus=HTMLElement.prototype.focus;"
       "HTMLElement.prototype.focus=function(){var result=__moriOriginalFocus.apply(this,arguments);__moriClipboardRemember(this);return result;};"
       "}catch(_e){}"
       "try{[HTMLInputElement&&HTMLInputElement.prototype,HTMLTextAreaElement&&HTMLTextAreaElement.prototype].forEach(function(proto){"
       "if(!proto||!proto.select)return;var originalSelect=proto.select;"
       "proto.select=function(){var result=originalSelect.apply(this,arguments);__moriClipboardRemember(this);return result;};"
       "});}catch(_e){}"
       "function __moriClipboardEditable(){"
       "var el=document.activeElement;if(!el)return null;"
       "var tag=String(el.tagName||'').toUpperCase();"
       "if(tag==='TEXTAREA'||tag==='INPUT'||el.isContentEditable)return __moriClipboardRemember(el);"
       "if(__moriClipboardLastEditable&&document.contains(__moriClipboardLastEditable))return __moriClipboardLastEditable;"
       "return null;"
       "}"
       "function __moriClipboardEditableText(el){"
       "el=el||__moriClipboardEditable();if(!el)return '';"
       "var tag=String(el.tagName||'').toUpperCase();"
       "if(tag==='TEXTAREA'||tag==='INPUT')return String(el.value||'');"
       "return String(el.textContent||'');"
       "}"
       "function __moriClipboardSelectedText(){"
       "var el=__moriClipboardEditable();"
       "if(el){var tag=String(el.tagName||'').toUpperCase();"
       "if(tag==='TEXTAREA'||tag==='INPUT'){var value=String(el.value||'');"
       "var start=typeof el.selectionStart==='number'?el.selectionStart:0;"
       "var end=typeof el.selectionEnd==='number'?el.selectionEnd:value.length;"
       "return start!==end?value.slice(start,end):value;}"
       "if(el.isContentEditable){"
       "try{var selection=globalThis.getSelection&&globalThis.getSelection();var selected=selection?String(selection):'';return selected||String(el.textContent||'');}catch(_e){return String(el.textContent||'');}"
       "}}"
       "try{var selection=globalThis.getSelection&&globalThis.getSelection();return selection?String(selection):'';}catch(_e){return '';}"
       "}"
       "function __moriClipboardInsertText(text){"
       "text=String(text||'');var el=__moriClipboardEditable();if(!el)return false;"
       "var tag=String(el.tagName||'').toUpperCase();"
       "if(tag==='TEXTAREA'||tag==='INPUT'){"
       "var value=String(el.value||'');"
       "var start=typeof el.selectionStart==='number'?el.selectionStart:value.length;"
       "var end=typeof el.selectionEnd==='number'?el.selectionEnd:start;"
       "el.value=value.slice(0,start)+text+value.slice(end);"
       "try{el.selectionStart=el.selectionEnd=start+text.length;}catch(_e){}"
       "try{el.dispatchEvent(new Event('input',{bubbles:true}));}catch(_e){}"
       "return true;"
       "}"
       "if(el.isContentEditable&&__moriOriginalExecCommand){"
       "try{return !!__moriOriginalExecCommand('insertText',false,text);}catch(_e){}"
       "}"
       "return false;"
       "}"
       "if(__moriOriginalExecCommand){var __moriExecCommandWrapper=function(command,showUI,value){"
       "var name=String(command||'').toLowerCase();"
       "if(name==='copy'||name==='cut'){"
       "var selected=__moriClipboardSelectedText();var ok=false;"
       "try{ok=!!__moriOriginalExecCommand(command,showUI,value);}catch(_e){}"
       "if(selected.length>0||ok){__moriClipboardApi.writeText(selected).catch(function(){});}"
       "return ok||selected.length>0;"
       "}"
       "if(name==='paste'){"
       "var before=__moriClipboardEditableText();"
       "var ok=false;try{ok=!!__moriOriginalExecCommand(command,showUI,value);}catch(_e){}"
       "if(ok&&__moriClipboardEditableText()!==before)return true;"
       "if(__moriClipboardCacheReady){__moriClipboardInsertText(__moriClipboardCache);}"
       "__moriClipboardRefresh().then(function(text){"
       "var empty=__moriClipboardEditableText().length===0;"
       "if(empty)__moriClipboardInsertText(text);"
       "});"
       "return true;"
       "}"
       "return __moriOriginalExecCommand(command,showUI,value);"
       "};"
       "try{Object.defineProperty(document,'execCommand',{configurable:true,value:__moriExecCommandWrapper});}"
       "catch(_e){try{document.execCommand=__moriExecCommandWrapper;}catch(_e2){}}"
       "}"
       "}"
       "function __moriExtHeadersObject(raw){"
       "var out={};if(!raw)return out;"
       "try{new Headers(raw).forEach(function(value,key){out[key]=value;});return out;}catch(e){}"
       "if(Array.isArray(raw)){raw.forEach(function(item){if(item&&item.length>=2)out[String(item[0])]=String(item[1]);});return out;}"
       "try{Object.keys(raw).forEach(function(key){out[key]=String(raw[key]);});}catch(e){}"
       "return out;"
       "}"
       "function __moriExtBase64Bytes(value){"
       "var binary=atob(String(value||''));"
       "var bytes=new Uint8Array(binary.length);"
       "for(var i=0;i<binary.length;i++)bytes[i]=binary.charCodeAt(i);"
       "return bytes;"
       "}"
       "if(String(location.protocol)==='mori-extension:'&&globalThis.fetch&&globalThis.Response&&!globalThis.__moriFetchWrapped){"
       "globalThis.__moriFetchWrapped=true;"
       "var __moriOriginalFetch=globalThis.fetch.bind(globalThis);"
       "globalThis.fetch=function(input,init){"
       "init=init||{};"
       "var url=typeof input==='string'||input instanceof URL?String(input):(input&&input.url?String(input.url):'');"
       "if(!/^https?:\\/\\//i.test(url))return __moriOriginalFetch(input,init);"
       "var method=String(init.method||(input&&input.method)||'GET').toUpperCase();"
       "var headers=__moriExtHeadersObject(input&&input.headers);"
       "var initHeaders=__moriExtHeadersObject(init.headers);"
       "Object.keys(initHeaders).forEach(function(key){headers[key]=initHeaders[key];});"
       "var body=Object.prototype.hasOwnProperty.call(init,'body')?init.body:null;"
       "if(body instanceof URLSearchParams)body=body.toString();"
       "if(body!=null&&typeof body!=='string')return __moriOriginalFetch(input,init);"
       "var credentials=String(init.credentials||(input&&input.credentials)||'same-origin');"
       "return __moriExtCall('runtime.fetch',{url:url,method:method,headers:headers,body:body==null?null:String(body),credentials:credentials}).then(function(result){"
       "return new Response(__moriExtBase64Bytes(result&&result.bodyBase64),{"
       "status:(result&&result.status)||200,"
       "statusText:(result&&result.statusText)||'',"
       "headers:(result&&result.headers)||{}"
       "});"
       "});"
       "};"
       "}"
	       "window.__moriExtDispatchMessage=window.__moriExtDispatchMessage||function(extensionId,message,requestId,sourceUrl,sourceOrigin,toContentScript,external,sourceTabId){"
	       "if(extensionId!==extId)return;"
	       // chrome.runtime.sendMessage never echoes back to the sending document;
	       // the bridge broadcasts to every view, so skip the originator here. Else
	       // the sender's own no-listener branch races a null response ahead of the
	       // real reply from the background worker.
	       "if(sourceUrl&&String(sourceUrl)===String(location.href))return;"
	       // Internal runtime.sendMessage reaches extension contexts only (the
	       // background worker, popup, options page, offscreen documents) — never
	       // content scripts, matching Chrome. The bridge still broadcasts to every
	       // view, so web-page frames bail out here. This is what keeps a content
	       // script's declining onMessage handler from emitting a response that
	       // races ahead of the background worker's real reply (the symptom: an
	       // extension popup that goes blank because its sendMessage resolved to
	       // null). tabs.sendMessage sets toContentScript to reach a tab on purpose.
	       "if(!toContentScript&&String(location.protocol)!=='mori-extension:')return;"
	       // An external message (from an externally_connectable web page, e.g. the
	       // account.proton.me sign-in fork) fires onMessageExternal with a sender
	       // that carries url/origin but NO id, so the extension classifies it as
	       // external (sender.id !== runtime.id). Internal messages fire onMessage
	       // with sender.id set to the extension id.
	       "var listeners=(external?(runtime.onMessageExternal&&runtime.onMessageExternal._listeners):(runtime.onMessage&&runtime.onMessage._listeners))||[];"
	       "var tabId=Number(sourceTabId);"
	       "var sourceTab=isFinite(tabId)&&tabId>=0?{id:tabId,windowId:1,index:-1,active:true,highlighted:true,selected:true,pinned:false,incognito:false,status:'complete',url:sourceUrl?String(sourceUrl):''}:undefined;"
	       "var sender=external?{url:sourceUrl?String(sourceUrl):'',origin:sourceOrigin?String(sourceOrigin):undefined}:{id:extId,url:sourceUrl?String(sourceUrl):String(location.href)};"
	       "if(sourceTab)sender.tab=sourceTab;"
	       "if(!external&&sourceOrigin)sender.origin=String(sourceOrigin);"
	       "var responded=false,pending=false;"
	       "listeners.slice().forEach(function(fn){"
	       "function sendResponse(value){"
	       "if(responded||!requestId)return;"
	       "responded=true;"
	       "__moriExtCall('runtime.messageResponse',{requestId:requestId,response:value===undefined?null:value});"
	       "}"
	       "try{"
	       "var result=fn(message,sender,sendResponse);"
	       "if(result&&typeof result.then==='function'){"
	       "pending=true;"
	       "result.then(sendResponse,function(error){sendResponse({error:error&&error.message?error.message:String(error)});});"
	       "}else if(result===true){pending=true;}"
	       "else if(result!==undefined&&result!==false){sendResponse(result);}"
	       "}catch(e){console.error(e);sendResponse({error:e&&e.message?e.message:String(e)});}"
	       "});"
	       // No synchronous answer in this context. Report a *soft* decline instead
	       // of an immediate null: another extension context (typically the
	       // background worker) may still answer asynchronously, and the bridge
	       // settles the request the instant a real reply arrives. The decline only
	       // resolves the sender — to undefined, as Chrome does — if every context
	       // stays silent past a short grace period.
	       "if(!responded&&!pending&&requestId){"
	       "__moriExtCall('runtime.messageNoResponse',{requestId:requestId});"
	       "}"
	       "};"
       "window.__moriExtDispatchEvent=window.__moriExtDispatchEvent||function(eventName,args,extensionId){"
       "if(extensionId&&extensionId!==extId)return;"
       "var target=chrome;"
       "String(eventName||'').split('.').forEach(function(part){target=target&&target[part];});"
       "if(target&&typeof target._fire==='function')target._fire.apply(null,Array.isArray(args)?args:[]);"
       "};"
       "window.__moriExtPorts=window.__moriExtPorts||{};"
       "function __moriMakePort(portId,name,sender){"
       "var ports=window.__moriExtPorts;"
       "if(ports[portId])return ports[portId];"
       "var disconnected=false;"
       "var port={name:String(name||''),sender:sender||{id:extId,url:String(location.href)},"
       "onMessage:__moriEvent(),onDisconnect:__moriEvent(),"
       "postMessage:function(message){if(disconnected)return;"
       "__moriExtCall('runtime.portMessage',Object.assign({portId:portId,message:message},__moriSourceInfo()));},"
       "disconnect:function(){if(disconnected)return;disconnected=true;"
       "__moriExtCall('runtime.portDisconnect',Object.assign({portId:portId},__moriSourceInfo()));"
       "port.onDisconnect._fire(port);delete ports[portId];}};"
       "ports[portId]=port;return port;"
       "}"
       "function __moriOpenPort(method,args,name){"
       "var portId=extId+':port:'+Date.now()+':'+Math.random().toString(36).slice(2);"
       "var port=__moriMakePort(portId,name,{id:extId,url:String(location.href)});"
       "args=Object.assign(args||{},__moriSourceInfo());args.portId=portId;args.name=String(name||'');"
       "__moriExtCall(method,args);return port;"
       "}"
       "runtime.connect=runtime.connect||function(extensionIdOrConnectInfo,connectInfo){"
       "var target=extId,info={};"
       "if(typeof extensionIdOrConnectInfo==='string'){target=extensionIdOrConnectInfo;info=connectInfo||{};}"
       "else{info=extensionIdOrConnectInfo||{};}"
       "return __moriOpenPort('runtime.connect',{targetExtensionId:target},info.name||'');"
       "};"
       "chrome.tabs.connect=chrome.tabs.connect||function(tabId,connectInfo){"
       "connectInfo=connectInfo||{};"
       "return __moriOpenPort('tabs.connect',{tabId:tabId},connectInfo.name||'');"
       "};"
	       "window.__moriExtDispatchConnect=window.__moriExtDispatchConnect||function(extensionId,portId,name,sender,sourceUrl,external){"
	       "if(extensionId!==extId||String(sourceUrl||'')===String(location.href))return;"
	       "var port=__moriMakePort(portId,name,sender||{id:extId,url:String(location.href)});"
		      "(external&&runtime.onConnectExternal?runtime.onConnectExternal:runtime.onConnect)._fire(port);"
	       "};"
       "window.__moriExtDispatchPortMessage=window.__moriExtDispatchPortMessage||function(extensionId,portId,message,sourceUrl){"
       "if(extensionId!==extId||String(sourceUrl||'')===String(location.href))return;"
       "var port=window.__moriExtPorts&&window.__moriExtPorts[portId];"
       "if(port)port.onMessage._fire(message,port);"
       "};"
       "window.__moriExtDispatchPortDisconnect=window.__moriExtDispatchPortDisconnect||function(extensionId,portId,sourceUrl){"
       "if(extensionId!==extId||String(sourceUrl||'')===String(location.href))return;"
       "var ports=window.__moriExtPorts||{};var port=ports[portId];"
       "if(port){port.onDisconnect._fire(port);delete ports[portId];}"
       "};"
       "window.__moriExtDispatchNativePortMessage=window.__moriExtDispatchNativePortMessage||function(extensionId,portId,message){"
       "if(extensionId!==extId)return;"
       "var port=window.__moriExtPorts&&window.__moriExtPorts[portId];"
       "if(port)port.onMessage._fire(message,port);"
       "};"
       "window.__moriExtDispatchNativePortDisconnect=window.__moriExtDispatchNativePortDisconnect||function(extensionId,portId){"
       "if(extensionId!==extId)return;"
       "var ports=window.__moriExtPorts||{};var port=ports[portId];"
       "if(port){port.onDisconnect._fire(port);delete ports[portId];}"
       "};"
      "function __moriStorageArea(area){"
      "var target=chrome.storage[area]=chrome.storage[area]||{};"
      "if(area==='local'){target.QUOTA_BYTES=target.QUOTA_BYTES||10485760;}"
      "if(area==='sync'){"
      "target.QUOTA_BYTES=target.QUOTA_BYTES||102400;"
      "target.QUOTA_BYTES_PER_ITEM=target.QUOTA_BYTES_PER_ITEM||8192;"
      "target.MAX_ITEMS=target.MAX_ITEMS||512;"
      "target.MAX_WRITE_OPERATIONS_PER_HOUR=target.MAX_WRITE_OPERATIONS_PER_HOUR||1800;"
      "target.MAX_WRITE_OPERATIONS_PER_MINUTE=target.MAX_WRITE_OPERATIONS_PER_MINUTE||120;"
      "}"
      "if(area==='session'){target.QUOTA_BYTES=target.QUOTA_BYTES||10485760;}"
      "target.get=target.get||function(keys,cb){"
       "var p=__moriExtCall('storage.'+area+'.get',{keys:keys});"
       "if(typeof cb==='function')p.then(cb);return p;};"
       "target.set=target.set||function(items,cb){"
       "var p=__moriExtCall('storage.'+area+'.set',{items:items||{}});"
       "if(typeof cb==='function')p.then(function(){cb();});return p;};"
       "target.remove=target.remove||function(keys,cb){"
       "var p=__moriExtCall('storage.'+area+'.remove',{keys:keys});"
       "if(typeof cb==='function')p.then(function(){cb();});return p;};"
	       "target.clear=target.clear||function(cb){"
	       "var p=__moriExtCall('storage.'+area+'.clear',{});"
	       "if(typeof cb==='function')p.then(function(){cb();});return p;};"
	       "target.getBytesInUse=target.getBytesInUse||function(keys,cb){"
	       "var p=__moriExtCall('storage.'+area+'.getBytesInUse',{keys:keys});"
	       "if(typeof cb==='function')p.then(cb);return p;};"
	       "target.getKeys=target.getKeys||function(cb){"
	       "var p=__moriExtCall('storage.'+area+'.getKeys',{});"
	       "if(typeof cb==='function')p.then(cb);return p;};"
	       "target.setAccessLevel=target.setAccessLevel||function(accessOptions,cb){"
	       "var p=__moriExtCall('storage.'+area+'.setAccessLevel',{accessOptions:accessOptions||{}});"
	       "if(typeof cb==='function')p.then(function(){cb();});return p;};"
	       "}"
	       "__moriStorageArea('local');__moriStorageArea('sync');__moriStorageArea('session');__moriStorageArea('managed');"
       "chrome.tabs.query=chrome.tabs.query||function(queryInfo,cb){"
       "var p=__moriExtCall('tabs.query',{queryInfo:queryInfo||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.tabs.get=chrome.tabs.get||function(tabId,cb){"
       "var p=__moriExtCall('tabs.get',{tabId:tabId});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.tabs.getCurrent=chrome.tabs.getCurrent||function(cb){"
       "var p=__moriExtCall('tabs.getCurrent',{});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.tabs.create=chrome.tabs.create||function(createProperties,cb){"
       "var p=__moriExtCall('tabs.create',{createProperties:createProperties||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.tabs.duplicate=chrome.tabs.duplicate||function(tabId,cb){"
       "var p=__moriExtCall('tabs.duplicate',{tabId:tabId});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.tabs.reload=chrome.tabs.reload||function(tabId,reloadProperties,cb){"
       "if(typeof tabId==='function'){cb=tabId;tabId=null;reloadProperties={};}"
       "else if(typeof tabId==='object'){cb=reloadProperties;reloadProperties=tabId;tabId=null;}"
       "else if(typeof reloadProperties==='function'){cb=reloadProperties;reloadProperties={};}"
       "var p=__moriExtCall('tabs.reload',{tabId:tabId,reloadProperties:reloadProperties||{}});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.tabs.goBack=chrome.tabs.goBack||function(tabId,cb){"
       "if(typeof tabId==='function'){cb=tabId;tabId=null;}"
       "var p=__moriExtCall('tabs.goBack',{tabId:tabId});"
       "if(typeof cb==='function')p.then(function(){cb();});return p;"
       "};"
       "chrome.tabs.goForward=chrome.tabs.goForward||function(tabId,cb){"
       "if(typeof tabId==='function'){cb=tabId;tabId=null;}"
       "var p=__moriExtCall('tabs.goForward',{tabId:tabId});"
       "if(typeof cb==='function')p.then(function(){cb();});return p;"
       "};"
       "chrome.tabs.getZoom=chrome.tabs.getZoom||function(tabId,cb){"
       "if(typeof tabId==='function'){cb=tabId;tabId=null;}"
       "var p=__moriExtCall('tabs.getZoom',{tabId:tabId});"
       "if(typeof cb==='function')p.then(cb);return p;"
       "};"
       "chrome.tabs.setZoom=chrome.tabs.setZoom||function(tabId,zoomFactor,cb){"
       "if(typeof tabId==='number'&&typeof zoomFactor==='function'){cb=zoomFactor;zoomFactor=1;}"
       "else if(typeof tabId!=='number'){cb=zoomFactor;zoomFactor=tabId;tabId=null;}"
       "var p=__moriExtCall('tabs.setZoom',{tabId:tabId,zoomFactor:Number(zoomFactor)||1});"
       "if(typeof cb==='function')p.then(function(){cb();});return p;"
       "};"
       "chrome.tabs.getZoomSettings=chrome.tabs.getZoomSettings||function(tabId,cb){"
       "if(typeof tabId==='function'){cb=tabId;tabId=null;}"
       "var p=__moriExtCall('tabs.getZoomSettings',{tabId:tabId});"
       "if(typeof cb==='function')p.then(cb);return p;"
       "};"
       "chrome.tabs.setZoomSettings=chrome.tabs.setZoomSettings||function(tabId,zoomSettings,cb){"
       "if(typeof tabId==='object'){cb=zoomSettings;zoomSettings=tabId;tabId=null;}"
       "else if(typeof zoomSettings==='function'){cb=zoomSettings;zoomSettings={};}"
       "var p=__moriExtCall('tabs.setZoomSettings',{tabId:tabId,zoomSettings:zoomSettings||{}});"
       "if(typeof cb==='function')p.then(function(){cb();});return p;"
       "};"
       "chrome.tabs.detectLanguage=chrome.tabs.detectLanguage||function(tabId,cb){"
       "if(typeof tabId==='function'){cb=tabId;tabId=null;}"
       "var result=(navigator.language||uiLanguage||'en').split('-')[0];"
       "if(typeof cb==='function')cb(result);return Promise.resolve(result);"
       "};"
       "chrome.tabs.remove=chrome.tabs.remove||function(tabIds,cb){"
       "var p=__moriExtCall('tabs.remove',{tabIds:tabIds});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.tabs.move=chrome.tabs.move||function(tabIds,moveProperties,cb){"
       "var p=__moriExtCall('tabs.move',{tabIds:tabIds,moveProperties:moveProperties||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.tabs.update=chrome.tabs.update||function(tabId,updateProperties,cb){"
       "if(typeof tabId==='object'){cb=updateProperties;updateProperties=tabId;tabId=null;}"
       "var p=__moriExtCall('tabs.update',{tabId:tabId,updateProperties:updateProperties||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.tabs.highlight=chrome.tabs.highlight||function(highlightInfo,cb){"
       "var p=__moriExtCall('tabs.highlight',{highlightInfo:highlightInfo||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.tabs.sendMessage=chrome.tabs.sendMessage||function(tabId,message,options,cb){"
       "if(typeof options==='function'){cb=options;options={};}"
       "var p=__moriExtCall('tabs.sendMessage',Object.assign({tabId:tabId,message:message,options:options||{}},__moriSourceInfo()));"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "function __moriLegacyTabInjectArgs(tabId,details,cb){"
       "if(typeof tabId==='function'){cb=tabId;details={};tabId=null;}"
       "else if(typeof tabId==='object'||tabId==null){cb=details;details=tabId||{};tabId=null;}"
       "if(typeof details==='function'){cb=details;details={};}"
       "details=details||{};"
       "return {tabId:tabId,details:details,cb:cb};"
       "}"
       "chrome.tabs.executeScript=chrome.tabs.executeScript||function(tabId,details,cb){"
       "var args=__moriLegacyTabInjectArgs(tabId,details,cb);"
       "var payload={target:{tabId:args.tabId||undefined,allFrames:!!args.details.allFrames},"
       "files:args.details.file?[args.details.file]:(args.details.files||null),code:args.details.code||null};"
       "if(Number.isFinite(Number(args.details.frameId)))payload.target.frameIds=[Number(args.details.frameId)];"
       "var p=chrome.scripting.executeScript(payload);"
       "if(typeof args.cb==='function')p.then(args.cb);"
       "return p;"
       "};"
       "chrome.tabs.insertCSS=chrome.tabs.insertCSS||function(tabId,details,cb){"
       "var args=__moriLegacyTabInjectArgs(tabId,details,cb);"
       "var payload={target:{tabId:args.tabId||undefined,allFrames:!!args.details.allFrames},"
       "files:args.details.file?[args.details.file]:(args.details.files||null),css:args.details.code||args.details.css||null};"
       "if(Number.isFinite(Number(args.details.frameId)))payload.target.frameIds=[Number(args.details.frameId)];"
       "var p=chrome.scripting.insertCSS(payload);"
       "if(typeof args.cb==='function')p.then(function(){args.cb();});"
       "return p;"
       "};"
       "chrome.tabs.removeCSS=chrome.tabs.removeCSS||function(tabId,details,cb){"
       "var args=__moriLegacyTabInjectArgs(tabId,details,cb);"
       "var payload={target:{tabId:args.tabId||undefined,allFrames:!!args.details.allFrames},"
       "files:args.details.file?[args.details.file]:(args.details.files||null),css:args.details.code||args.details.css||null};"
       "if(Number.isFinite(Number(args.details.frameId)))payload.target.frameIds=[Number(args.details.frameId)];"
       "var p=chrome.scripting.removeCSS(payload);"
       "if(typeof args.cb==='function')p.then(function(){args.cb();});"
       "return p;"
       "};"
	       "chrome.tabs.captureVisibleTab=chrome.tabs.captureVisibleTab||function(windowId,options,cb){"
	       "if(typeof windowId==='function'){cb=windowId;windowId=null;options={};}"
	       "else if(typeof windowId==='object'){cb=options;options=windowId;windowId=null;}"
	       "var p=__moriExtCall('tabs.captureVisibleTab',{windowId:windowId,options:options||{}});"
	       "if(typeof cb==='function')p.then(cb);return p;"
	       "};"
       "chrome.windows.getCurrent=chrome.windows.getCurrent||function(getInfo,cb){"
       "if(typeof getInfo==='function'){cb=getInfo;getInfo={};}"
       "var p=__moriExtCall('windows.getCurrent',{populate:!!(getInfo&&getInfo.populate)});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.windows.getLastFocused=chrome.windows.getLastFocused||function(getInfo,cb){"
       "if(typeof getInfo==='function'){cb=getInfo;getInfo={};}"
       "var p=__moriExtCall('windows.getLastFocused',{populate:!!(getInfo&&getInfo.populate)});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.windows.get=chrome.windows.get||function(windowId,getInfo,cb){"
       "if(typeof getInfo==='function'){cb=getInfo;getInfo={};}"
       "var p=__moriExtCall('windows.get',{windowId:windowId,getInfo:getInfo||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.windows.getAll=chrome.windows.getAll||function(getInfo,cb){"
       "if(typeof getInfo==='function'){cb=getInfo;getInfo={};}"
       "var p=__moriExtCall('windows.getAll',{getInfo:getInfo||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.windows.create=chrome.windows.create||function(createData,cb){"
       "if(typeof createData==='function'){cb=createData;createData={};}"
       "var p=__moriExtCall('windows.create',{createData:createData||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.windows.update=chrome.windows.update||function(windowId,updateInfo,cb){"
       "var p=__moriExtCall('windows.update',{windowId:windowId,updateInfo:updateInfo||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.windows.remove=chrome.windows.remove||function(windowId,cb){"
       "var p=__moriExtCall('windows.remove',{windowId:windowId});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.cookies.get=chrome.cookies.get||function(details,cb){"
       "var p=__moriExtCall('cookies.get',{details:details||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.cookies.getAll=chrome.cookies.getAll||function(details,cb){"
       "var p=__moriExtCall('cookies.getAll',{details:details||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.cookies.set=chrome.cookies.set||function(details,cb){"
       "var p=__moriExtCall('cookies.set',{details:details||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.cookies.remove=chrome.cookies.remove||function(details,cb){"
       "var p=__moriExtCall('cookies.remove',{details:details||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.cookies.getAllCookieStores=chrome.cookies.getAllCookieStores||function(cb){"
       "var p=__moriExtCall('cookies.getAllCookieStores',{});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.downloads.download=chrome.downloads.download||function(options,cb){"
       "var p=__moriExtCall('downloads.download',{options:options||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.downloads.search=chrome.downloads.search||function(query,cb){"
       "var p=__moriExtCall('downloads.search',{query:query||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.downloads.open=chrome.downloads.open||function(downloadId){"
       "return __moriExtCall('downloads.open',{downloadId:downloadId});"
       "};"
       "chrome.downloads.show=chrome.downloads.show||function(downloadId){"
       "return __moriExtCall('downloads.show',{downloadId:downloadId});"
       "};"
       "chrome.downloads.showDefaultFolder=chrome.downloads.showDefaultFolder||function(){"
       "return __moriExtCall('downloads.showDefaultFolder',{});"
       "};"
       "chrome.downloads.erase=chrome.downloads.erase||function(query,cb){"
       "var p=__moriExtCall('downloads.erase',{query:query||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.downloads.cancel=chrome.downloads.cancel||function(downloadId,cb){"
       "var p=__moriExtCall('downloads.cancel',{downloadId:downloadId});"
       "if(typeof cb==='function')p.then(function(){cb();});return p;"
       "};"
       "chrome.downloads.pause=chrome.downloads.pause||function(downloadId,cb){"
       "if(typeof cb==='function')cb();return Promise.resolve();"
       "};"
       "chrome.downloads.resume=chrome.downloads.resume||function(downloadId,cb){"
       "if(typeof cb==='function')cb();return Promise.resolve();"
       "};"
       "chrome.downloads.getFileIcon=chrome.downloads.getFileIcon||function(downloadId,options,cb){"
       "if(typeof options==='function'){cb=options;options={};}"
       "var result='';if(typeof cb==='function')cb(result);return Promise.resolve(result);"
       "};"
       "chrome.downloads.removeFile=chrome.downloads.removeFile||function(downloadId,cb){"
       "var p=__moriExtCall('downloads.removeFile',{downloadId:downloadId});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.scripting.executeScript=chrome.scripting.executeScript||function(details,cb){"
       "details=details||{};"
       "var payload={target:details.target||{},files:details.files||null,args:details.args||[],code:details.code||null};"
       "var fn=details.func||details.function;"
       "if(typeof fn==='function')payload.funcSource=String(fn);"
       "var p=__moriExtCall('scripting.executeScript',{details:payload});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.scripting.insertCSS=chrome.scripting.insertCSS||function(details,cb){"
       "details=details||{};"
       "var p=__moriExtCall('scripting.insertCSS',{details:{"
       "target:details.target||{},files:details.files||null,css:details.css||null"
       "}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.scripting.registerContentScripts=chrome.scripting.registerContentScripts||function(scripts,cb){"
       "var p=__moriExtCall('scripting.registerContentScripts',{scripts:scripts||[]});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.scripting.getRegisteredContentScripts=chrome.scripting.getRegisteredContentScripts||function(filter,cb){"
       "if(typeof filter==='function'){cb=filter;filter={};}"
       "var p=__moriExtCall('scripting.getRegisteredContentScripts',{filter:filter||{}});"
       "if(typeof cb==='function')p.then(cb);"
       "return p;"
       "};"
       "chrome.scripting.updateContentScripts=chrome.scripting.updateContentScripts||function(scripts,cb){"
       "var p=__moriExtCall('scripting.updateContentScripts',{scripts:scripts||[]});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
       "chrome.scripting.unregisterContentScripts=chrome.scripting.unregisterContentScripts||function(filter,cb){"
       "if(typeof filter==='function'){cb=filter;filter={};}"
       "var p=__moriExtCall('scripting.unregisterContentScripts',{filter:filter||{}});"
       "if(typeof cb==='function')p.then(function(){cb();});"
       "return p;"
       "};"
	       "chrome.scripting.removeCSS=chrome.scripting.removeCSS||function(details,cb){"
	       "details=details||{};"
	       "var p=__moriExtCall('scripting.removeCSS',{details:{"
	       "target:details.target||{},files:details.files||null,css:details.css||null"
	       "}});"
	       "if(typeof cb==='function')p.then(function(){cb();});"
	       "return p;"
	       "};"
	       "function __moriMirrorBrowserAPI(){"
	       "try{"
	       "var b=globalThis.browser=globalThis.browser||chrome;"
	       "['runtime','i18n','storage','tabs','scripting','declarativeNetRequest','sidePanel','action','browserAction','pageAction','notifications','alarms','offscreen','commands','permissions','history','topSites','cookies','browsingData','downloads','sessions','management','bookmarks','contextMenus','menus','windows','identity','webNavigation','webRequest','idle','privacy','proxy','extension'].forEach(function(name){"
	       "var src=chrome[name];if(!src)return;"
	       "if(!b[name]){b[name]=src;return;}"
	       "if((typeof src==='object'||typeof src==='function')&&(typeof b[name]==='object'||typeof b[name]==='function')){"
	       "Object.keys(src).forEach(function(key){if(b[name][key]===undefined)b[name][key]=src[key];});"
	       "}"
	       "});"
	       "b.name=b.name||hostBrowserInfo.name;"
	       "b.version=b.version||hostBrowserInfo.version;"
	       "}catch(e){}"
	       "}"
	       "__moriMirrorBrowserAPI();"
	       "if(document.documentElement&&document.documentElement.dataset.moriExtensionBackground==='true'&&!globalThis.importScripts){"
       "globalThis.importScripts=function(){"
       "Array.prototype.slice.call(arguments).forEach(function(raw){"
       "var url=String(raw||'');"
       "if(url.indexOf('://')<0)url=runtime.getURL(url);"
       "var xhr=new XMLHttpRequest();"
       "xhr.open('GET',url,false);"
       "xhr.send(null);"
       "if(xhr.status&&xhr.status>=400)throw new Error('importScripts failed: '+url);"
       "(0,eval)(String(xhr.responseText||'')+'\\n//# sourceURL='+url);"
       "});"
       "};"
       "}"
       "if(document.documentElement&&document.documentElement.dataset.moriExtensionBackground==='true'&&!window.__moriBackgroundBooted){"
       "window.__moriBackgroundBooted=true;"
       "setTimeout(function(){"
       "var version=String((manifest&&manifest.version)||'');"
       "var key='__mori_onInstalled_version_'+extId;"
       "var previous=null;"
       "try{previous=localStorage.getItem(key);}catch(e){}"
       "if(version&&previous!==version){"
       "var details={reason:previous?'update':'install'};"
       "if(previous)details.previousVersion=previous;"
       "runtime.onInstalled._fire(details);"
       "try{localStorage.setItem(key,version);}catch(e){}"
       "}"
       "runtime.onStartup._fire();"
       "},0);"
       "}"
      "})();",
      JSStringLiteral(identifier), JSONStringLiteral(localizedManifest),
      JSONStringLiteral(messages ?: @{}), JSStringLiteral(uiLanguage),
      JSONStringLiteral(browserInfo)];
}

void InjectExtensionPageRuntime(CefRefPtr<CefFrame> frame) {
  NSDictionary* ext = ExtensionRecordForFrame(frame);
  if (!ext) return;
  NSDictionary* manifest = ManifestForExtension(ext);
  NSMutableString* js = [NSMutableString stringWithString:@"(function(){try{"];
  [js appendString:ExtensionRuntimeShim(ext, manifest)];
  [js appendString:@"}catch(e){console.error('[Mori extension runtime]',e);}})();"];
  frame->ExecuteJavaScript(CefString(js.UTF8String), frame->GetURL(), 0);
}

NSString* DynamicContentScriptsDefaultsKey(NSString* extensionId) {
  return [@"mori.dynamicContentScripts." stringByAppendingString:extensionId ?: @""];
}

NSArray<NSDictionary*>* RegisteredContentScripts(NSString* extensionID) {
  NSArray* stored = [[NSUserDefaults standardUserDefaults]
      arrayForKey:DynamicContentScriptsDefaultsKey(extensionID)];
  NSMutableArray<NSDictionary*>* out = [NSMutableArray array];
  for (id item in stored) {
    if ([item isKindOfClass:NSDictionary.class]) [out addObject:item];
  }
  return out;
}

void PersistRegisteredContentScripts(NSString* extensionID,
                                     NSArray<NSDictionary*>* scripts) {
  [[NSUserDefaults standardUserDefaults] setObject:scripts ?: @[]
                                            forKey:DynamicContentScriptsDefaultsKey(extensionID)];
}

BOOL ContentScriptAllFrames(NSDictionary* script) {
  id raw = script[@"all_frames"] ?: script[@"allFrames"];
  return [raw respondsToSelector:@selector(boolValue)] && [raw boolValue];
}

NSString* ContentScriptRunAt(NSDictionary* script) {
  NSString* runAt = [script[@"run_at"] isKindOfClass:NSString.class]
      ? script[@"run_at"]
      : ([script[@"runAt"] isKindOfClass:NSString.class] ? script[@"runAt"] : nil);
  return runAt.length > 0 ? runAt : @"document_idle";
}

NSDictionary* APIContentScriptRecord(NSDictionary* script) {
  NSMutableDictionary* out = [NSMutableDictionary dictionary];
  NSString* identifier = [script[@"id"] isKindOfClass:NSString.class] ? script[@"id"] : @"";
  if (identifier.length > 0) out[@"id"] = identifier;
  if ([script[@"matches"] isKindOfClass:NSArray.class]) out[@"matches"] = script[@"matches"];
  NSArray* excludes = [script[@"exclude_matches"] isKindOfClass:NSArray.class]
      ? script[@"exclude_matches"]
      : ([script[@"excludeMatches"] isKindOfClass:NSArray.class] ? script[@"excludeMatches"] : nil);
  if (excludes) out[@"excludeMatches"] = excludes;
  if ([script[@"js"] isKindOfClass:NSArray.class]) out[@"js"] = script[@"js"];
  if ([script[@"css"] isKindOfClass:NSArray.class]) out[@"css"] = script[@"css"];
  out[@"allFrames"] = @(ContentScriptAllFrames(script));
  out[@"runAt"] = ContentScriptRunAt(script);
  if ([script[@"persistAcrossSessions"] respondsToSelector:@selector(boolValue)]) {
    out[@"persistAcrossSessions"] = @([script[@"persistAcrossSessions"] boolValue]);
  }
  if ([script[@"world"] isKindOfClass:NSString.class]) out[@"world"] = script[@"world"];
  return out;
}

NSDictionary* NormalizeRegisteredContentScript(NSDictionary* raw,
                                               NSDictionary* existing,
                                               BOOL requireMatches,
                                               NSString** error) {
  if (![raw isKindOfClass:NSDictionary.class]) {
    if (error) *error = @"Content script must be an object.";
    return nil;
  }

  NSMutableDictionary* out =
      existing ? [existing mutableCopy] : [NSMutableDictionary dictionary];
  NSString* identifier = [raw[@"id"] isKindOfClass:NSString.class]
      ? raw[@"id"]
      : ([existing[@"id"] isKindOfClass:NSString.class] ? existing[@"id"] : nil);
  if (identifier.length == 0) {
    if (error) *error = @"Content script is missing an id.";
    return nil;
  }
  out[@"id"] = identifier;

  NSArray* matches = [raw[@"matches"] isKindOfClass:NSArray.class] ? raw[@"matches"] : nil;
  if (matches) out[@"matches"] = matches;
  if (requireMatches && ![out[@"matches"] isKindOfClass:NSArray.class]) {
    if (error) *error = @"Content script is missing matches.";
    return nil;
  }

  NSArray* excludes = [raw[@"excludeMatches"] isKindOfClass:NSArray.class]
      ? raw[@"excludeMatches"]
      : ([raw[@"exclude_matches"] isKindOfClass:NSArray.class] ? raw[@"exclude_matches"] : nil);
  if (excludes) out[@"exclude_matches"] = excludes;
  NSArray* js = [raw[@"js"] isKindOfClass:NSArray.class] ? raw[@"js"] : nil;
  if (js) out[@"js"] = js;
  NSArray* css = [raw[@"css"] isKindOfClass:NSArray.class] ? raw[@"css"] : nil;
  if (css) out[@"css"] = css;
  if (![out[@"js"] isKindOfClass:NSArray.class] && ![out[@"css"] isKindOfClass:NSArray.class]) {
    if (error) *error = @"Content script is missing js or css files.";
    return nil;
  }

  id allFrames = raw[@"allFrames"] ?: raw[@"all_frames"];
  if ([allFrames respondsToSelector:@selector(boolValue)]) out[@"all_frames"] = @([allFrames boolValue]);
  NSString* runAt = [raw[@"runAt"] isKindOfClass:NSString.class]
      ? raw[@"runAt"]
      : ([raw[@"run_at"] isKindOfClass:NSString.class] ? raw[@"run_at"] : nil);
  if (runAt.length > 0) out[@"run_at"] = runAt;
  if (![out[@"run_at"] isKindOfClass:NSString.class]) out[@"run_at"] = @"document_idle";
  if ([raw[@"persistAcrossSessions"] respondsToSelector:@selector(boolValue)]) {
    out[@"persistAcrossSessions"] = @([raw[@"persistAcrossSessions"] boolValue]);
  }
  if ([raw[@"world"] isKindOfClass:NSString.class]) out[@"world"] = raw[@"world"];
  return out;
}

NSArray<NSString*>* RegisteredContentScriptIDsFromFilter(NSDictionary* filter) {
  NSArray* ids = [filter[@"ids"] isKindOfClass:NSArray.class] ? filter[@"ids"] : nil;
  NSMutableArray<NSString*>* out = [NSMutableArray array];
  for (id item in ids) {
    if ([item isKindOfClass:NSString.class]) [out addObject:item];
  }
  return out;
}

NSDictionary* HandleRegisteredContentScripts(NSString* method,
                                             NSDictionary* args,
                                             NSString* extensionID) {
  NSMutableArray<NSDictionary*>* scripts =
      [RegisteredContentScripts(extensionID) mutableCopy] ?: [NSMutableArray array];
  NSArray* rawScripts = [args[@"scripts"] isKindOfClass:NSArray.class] ? args[@"scripts"] : @[];
  NSDictionary* filter = [args[@"filter"] isKindOfClass:NSDictionary.class]
      ? args[@"filter"]
      : @{};

  if ([method isEqualToString:@"scripting.getRegisteredContentScripts"]) {
    NSArray<NSString*>* ids = RegisteredContentScriptIDsFromFilter(filter);
    NSMutableArray<NSDictionary*>* result = [NSMutableArray array];
    for (NSDictionary* script in scripts) {
      NSString* identifier = [script[@"id"] isKindOfClass:NSString.class] ? script[@"id"] : @"";
      if (ids.count > 0 && ![ids containsObject:identifier]) continue;
      [result addObject:APIContentScriptRecord(script)];
    }
    return @{@"result" : result};
  }

  if ([method isEqualToString:@"scripting.unregisterContentScripts"]) {
    NSArray<NSString*>* ids = RegisteredContentScriptIDsFromFilter(filter);
    if (ids.count == 0) {
      [scripts removeAllObjects];
    } else {
      [scripts filterUsingPredicate:[NSPredicate predicateWithBlock:^BOOL(
          NSDictionary* script, NSDictionary* bindings) {
        NSString* identifier = [script[@"id"] isKindOfClass:NSString.class] ? script[@"id"] : @"";
        return ![ids containsObject:identifier];
      }]];
    }
    PersistRegisteredContentScripts(extensionID, scripts);
    return @{@"result" : [NSNull null]};
  }

  BOOL updating = [method isEqualToString:@"scripting.updateContentScripts"];
  if (![method isEqualToString:@"scripting.registerContentScripts"] && !updating) {
    return @{@"error" : [NSString stringWithFormat:@"Unsupported scripting method: %@", method]};
  }

  for (id item in rawScripts) {
    if (![item isKindOfClass:NSDictionary.class]) {
      return @{@"error" : @"Content script must be an object."};
    }
    NSDictionary* raw = (NSDictionary*)item;
    NSString* identifier = [raw[@"id"] isKindOfClass:NSString.class] ? raw[@"id"] : @"";
    NSUInteger existingIndex = [scripts indexOfObjectPassingTest:^BOOL(
        NSDictionary* script, NSUInteger idx, BOOL* stop) {
      NSString* existingID = [script[@"id"] isKindOfClass:NSString.class] ? script[@"id"] : @"";
      return [existingID isEqualToString:identifier];
    }];
    NSDictionary* existing = existingIndex == NSNotFound ? nil : scripts[existingIndex];
    if (updating && !existing) {
      return @{@"error" : [NSString stringWithFormat:@"No registered content script with id %@.", identifier]};
    }
    NSString* error = nil;
    NSDictionary* normalized = NormalizeRegisteredContentScript(raw, existing, !updating, &error);
    if (!normalized) return @{@"error" : error ?: @"Invalid content script."};
    if (existingIndex == NSNotFound) {
      [scripts addObject:normalized];
    } else {
      scripts[existingIndex] = normalized;
    }
  }

  PersistRegisteredContentScripts(extensionID, scripts);
  return @{@"result" : [NSNull null]};
}

void InjectExtensionContentScripts(CefRefPtr<CefFrame> frame, NSString* phase) {
  if (!frame) return;
  NSString* urlString = @(frame->GetURL().ToString().c_str());
  NSURL* url = [NSURL URLWithString:urlString];
  if (!url) return;

  for (NSDictionary* ext in EnabledExtensionRecords()) {
    NSDictionary* manifest = ManifestForExtension(ext);
    NSMutableArray* scripts = [NSMutableArray array];
    if ([manifest[@"content_scripts"] isKindOfClass:[NSArray class]]) {
      [scripts addObjectsFromArray:manifest[@"content_scripts"]];
    }
    NSString* extensionID = [ext[@"id"] isKindOfClass:NSString.class] ? ext[@"id"] : @"";
    [scripts addObjectsFromArray:RegisteredContentScripts(extensionID)];
    for (id item in scripts) {
      if (![item isKindOfClass:[NSDictionary class]]) continue;
      NSDictionary* script = (NSDictionary*)item;
      BOOL allFrames = ContentScriptAllFrames(script);
      if (!allFrames && !frame->IsMain()) continue;
	      NSString* runAt = ContentScriptRunAt(script);
	      if (![runAt isEqualToString:phase] || !ScriptMatchesURL(script, url)) {
	        continue;
	      }

      NSMutableString* js = [NSMutableString stringWithString:@"(function(){try{"];
      [js appendString:ExtensionRuntimeShim(ext, manifest)];

      NSArray* cssFiles = [script[@"css"] isKindOfClass:[NSArray class]]
          ? script[@"css"]
          : nil;
      for (id cssPath in cssFiles) {
        if (![cssPath isKindOfClass:[NSString class]]) continue;
        NSString* css = ExtensionFileText(ext, cssPath);
        if (css.length == 0) continue;
        [js appendFormat:
            @"var s=document.createElement('style');"
             "s.dataset.moriExtension=%@;"
             "s.textContent=%@;"
             "(document.head||document.documentElement).appendChild(s);",
            JSStringLiteral(ext[@"id"]), JSStringLiteral(css)];
      }

      NSArray* jsFiles = [script[@"js"] isKindOfClass:[NSArray class]]
          ? script[@"js"]
          : nil;
      for (id jsPath in jsFiles) {
        if (![jsPath isKindOfClass:[NSString class]]) continue;
        NSString* source = ExtensionFileText(ext, jsPath);
        if (source.length == 0) continue;
        [js appendString:source];
        [js appendString:@"\n"];
      }
      [js appendString:@"}catch(e){console.error('[Mori extension]',e);}})();"];

      frame->ExecuteJavaScript(CefString(js.UTF8String), frame->GetURL(), 0);
    }
  }
}

NSString* ExtensionStorageDefaultsKey(NSString* extensionId) {
  return [@"mori.extensionStorage." stringByAppendingString:extensionId ?: @""];
}

NSString* ExtensionStorageDefaultsKey(NSString* extensionId, NSString* area) {
  NSString* cleanArea = area.length > 0 ? area : @"local";
  if ([cleanArea isEqualToString:@"local"]) {
    return ExtensionStorageDefaultsKey(extensionId);
  }
  return [NSString stringWithFormat:@"mori.extensionStorage.%@.%@",
                                    cleanArea, extensionId ?: @""];
}

NSString* DNRDynamicRulesDefaultsKey(NSString* extensionId) {
  return [@"mori.dnr.dynamicRules." stringByAppendingString:extensionId ?: @""];
}

NSString* DNREnabledRulesetsDefaultsKey(NSString* extensionId) {
  return [@"mori.dnr.enabledRulesets." stringByAppendingString:extensionId ?: @""];
}

NSString* ContextMenusDefaultsKey(NSString* extensionId) {
  return [@"mori.contextMenus." stringByAppendingString:extensionId ?: @""];
}

NSString* PermissionsDefaultsKey(NSString* extensionId) {
  return [@"mori.extensionPermissions." stringByAppendingString:extensionId ?: @""];
}

NSString* ProxySettingsDefaultsKey() {
  return @"mori.proxySettings";
}

NSArray<NSString*>* PermissionStringArray(id raw) {
  NSMutableArray<NSString*>* out = [NSMutableArray array];
  if (![raw isKindOfClass:NSArray.class]) return out;
  for (id item in (NSArray*)raw) {
    if ([item isKindOfClass:NSString.class] && [item length] > 0) {
      [out addObject:item];
    }
  }
  return out;
}

NSArray<NSString*>* PermissionManifestPermissions(NSDictionary* manifest,
                                                  NSString* key) {
  NSMutableArray<NSString*>* out = [NSMutableArray array];
  for (NSString* value in PermissionStringArray(manifest[key])) {
    if ([value isEqualToString:@"nativeMessaging"]) continue;
    if ([value containsString:@"://"] || [value isEqualToString:@"<all_urls>"]) {
      continue;
    }
    if (![out containsObject:value]) [out addObject:value];
  }
  return out;
}

NSArray<NSString*>* PermissionManifestOrigins(NSDictionary* manifest,
                                              NSString* key) {
  NSMutableArray<NSString*>* out = [NSMutableArray array];
  for (NSString* value in PermissionStringArray(manifest[key])) {
    if ([value containsString:@"://"] || [value isEqualToString:@"<all_urls>"]) {
      if (![out containsObject:value]) [out addObject:value];
    }
  }
  return out;
}

NSMutableArray<NSString*>* PermissionUniqueMutableArray(NSArray<NSString*>* values) {
  NSMutableArray<NSString*>* out = [NSMutableArray array];
  for (NSString* value in values ?: @[]) {
    if (value.length > 0 && ![out containsObject:value]) [out addObject:value];
  }
  return out;
}

NSDictionary* StoredOptionalPermissions(NSString* extensionID) {
  NSDictionary* stored = [[NSUserDefaults standardUserDefaults]
      dictionaryForKey:PermissionsDefaultsKey(extensionID)];
  return [stored isKindOfClass:NSDictionary.class] ? stored : @{};
}

NSDictionary* EffectivePermissions(NSDictionary* manifest, NSString* extensionID) {
  NSDictionary* stored = StoredOptionalPermissions(extensionID);
  NSMutableArray<NSString*>* permissions =
      PermissionUniqueMutableArray(PermissionManifestPermissions(manifest, @"permissions"));
  for (NSString* permission in PermissionStringArray(stored[@"permissions"])) {
    if (![permissions containsObject:permission]) [permissions addObject:permission];
  }

  NSMutableArray<NSString*>* origins =
      PermissionUniqueMutableArray(PermissionManifestOrigins(manifest, @"permissions"));
  for (NSString* origin in PermissionManifestOrigins(manifest, @"host_permissions")) {
    if (![origins containsObject:origin]) [origins addObject:origin];
  }
  for (NSString* origin in PermissionStringArray(stored[@"origins"])) {
    if (![origins containsObject:origin]) [origins addObject:origin];
  }

  return @{@"permissions" : permissions, @"origins" : origins};
}

BOOL ExtensionHasEffectivePermission(NSString* extensionID, NSString* permission) {
  if (extensionID.length == 0 || permission.length == 0) return NO;
  NSDictionary* ext = EnabledExtensionRecordForID(extensionID);
  NSDictionary* manifest = ManifestForExtension(ext ?: @{}) ?: @{};
  NSDictionary* effective = EffectivePermissions(manifest, extensionID);
  NSArray<NSString*>* permissions = PermissionStringArray(effective[@"permissions"]);
  return [permissions containsObject:permission];
}

NSString* MoriClipboardReadText() {
  __block NSString* text = @"";
  void (^readBlock)(void) = ^{
    NSString* value =
        [[NSPasteboard generalPasteboard] stringForType:NSPasteboardTypeString];
    text = value ?: @"";
  };
  if ([NSThread isMainThread]) {
    readBlock();
  } else {
    dispatch_sync(dispatch_get_main_queue(), readBlock);
  }
  return text ?: @"";
}

BOOL MoriClipboardWriteText(NSString* text) {
  __block BOOL ok = NO;
  void (^writeBlock)(void) = ^{
    NSPasteboard* pasteboard = [NSPasteboard generalPasteboard];
    [pasteboard clearContents];
    ok = [pasteboard setString:text ?: @"" forType:NSPasteboardTypeString];
  };
  if ([NSThread isMainThread]) {
    writeBlock();
  } else {
    dispatch_sync(dispatch_get_main_queue(), writeBlock);
  }
  return ok;
}

NSDictionary* HandleClipboard(NSString* method,
                              NSDictionary* args,
                              NSString* extensionID) {
  if ([method isEqualToString:@"clipboard.readText"]) {
    if (!ExtensionHasEffectivePermission(extensionID, @"clipboardRead")) {
      return @{@"error" : @"Extension does not have clipboardRead permission."};
    }
    return @{@"result" : MoriClipboardReadText()};
  }

  if ([method isEqualToString:@"clipboard.writeText"]) {
    if (!ExtensionHasEffectivePermission(extensionID, @"clipboardWrite")) {
      return @{@"error" : @"Extension does not have clipboardWrite permission."};
    }
    NSString* text = [args[@"text"] isKindOfClass:NSString.class]
        ? args[@"text"]
        : @"";
    if (!MoriClipboardWriteText(text)) {
      return @{@"error" : @"Could not write to the clipboard."};
    }
    return @{@"result" : @YES};
  }

  return @{@"error" : [NSString stringWithFormat:@"Unsupported clipboard method: %@", method]};
}

// Chrome match-pattern coverage: does a granted pattern (e.g. host_permission
// "https://*/*") cover a requested origin (e.g. "https://account.proton.me/*")?
// Exact-string matching alone leaves extensions (Proton Pass) stuck on a "grant
// permissions" screen because their broad host_permission doesn't literally
// equal the specific origin they probe via permissions.contains().
BOOL MoriParseMatchPattern(NSString* pattern, NSString** scheme,
                             NSString** host, NSString** path) {
  if ([pattern isEqualToString:@"<all_urls>"]) {
    *scheme = @"*"; *host = @"*"; *path = @"/*"; return YES;
  }
  NSRange sep = [pattern rangeOfString:@"://"];
  if (sep.location == NSNotFound) return NO;
  *scheme = [pattern substringToIndex:sep.location];
  NSString* rest = [pattern substringFromIndex:NSMaxRange(sep)];
  NSRange slash = [rest rangeOfString:@"/"];
  if (slash.location == NSNotFound) { *host = rest; *path = @"/*"; }
  else { *host = [rest substringToIndex:slash.location];
         *path = [rest substringFromIndex:slash.location]; }
  return YES;
}

BOOL MoriGlobCovers(NSString* glob, NSString* value) {
  if (glob.length == 0 || [glob isEqualToString:@"*"]) return YES;
  NSMutableString* rx = [NSMutableString stringWithString:@"^"];
  for (NSUInteger i = 0; i < glob.length; i++) {
    unichar c = [glob characterAtIndex:i];
    if (c == '*') { [rx appendString:@".*"]; }
    else { [rx appendString:[NSRegularExpression escapedPatternForString:
                                 [NSString stringWithCharacters:&c length:1]]]; }
  }
  [rx appendString:@"$"];
  NSRegularExpression* re = [NSRegularExpression regularExpressionWithPattern:rx
                                                                     options:0
                                                                       error:nil];
  return re && [re numberOfMatchesInString:value options:0
                                     range:NSMakeRange(0, value.length)] > 0;
}

BOOL MoriMatchPatternCovers(NSString* grant, NSString* req) {
  if ([grant isEqualToString:req] || [grant isEqualToString:@"<all_urls>"]) return YES;
  NSString *gs, *gh, *gp, *rs, *rh, *rp;
  if (!MoriParseMatchPattern(grant, &gs, &gh, &gp)) return NO;
  if (!MoriParseMatchPattern(req, &rs, &rh, &rp)) return NO;
  BOOL schemeOK = [gs isEqualToString:@"*"]
      ? ([rs isEqualToString:@"http"] || [rs isEqualToString:@"https"] ||
         [rs isEqualToString:@"ws"] || [rs isEqualToString:@"wss"])
      : [gs isEqualToString:rs];
  if (!schemeOK) return NO;
  BOOL hostOK;
  if ([gh isEqualToString:@"*"]) {
    hostOK = YES;
  } else if ([gh hasPrefix:@"*."]) {
    NSString* base = [gh substringFromIndex:2];
    hostOK = [rh isEqualToString:base] ||
             [rh hasSuffix:[@"." stringByAppendingString:base]];
  } else {
    hostOK = [gh isEqualToString:rh];
  }
  if (!hostOK) return NO;
  return MoriGlobCovers(gp, rp);
}

BOOL PermissionOriginGranted(NSArray<NSString*>* origins, NSString* origin) {
  if (origin.length == 0) return YES;
  for (NSString* granted in origins) {
    if (MoriMatchPatternCovers(granted, origin)) return YES;
  }
  return NO;
}

BOOL PermissionSetContains(NSDictionary* set, NSDictionary* request) {
  NSArray<NSString*>* grantedPermissions = PermissionStringArray(set[@"permissions"]);
  NSArray<NSString*>* grantedOrigins = PermissionStringArray(set[@"origins"]);
  for (NSString* permission in PermissionStringArray(request[@"permissions"])) {
    if (![grantedPermissions containsObject:permission]) return NO;
  }
  for (NSString* origin in PermissionStringArray(request[@"origins"])) {
    if (!PermissionOriginGranted(grantedOrigins, origin)) return NO;
  }
  return YES;
}

BOOL PermissionRequestAllowed(NSDictionary* manifest, NSDictionary* request) {
  NSMutableArray<NSString*>* allowedPermissions =
      PermissionUniqueMutableArray(PermissionManifestPermissions(manifest, @"permissions"));
  for (NSString* permission in PermissionManifestPermissions(manifest, @"optional_permissions")) {
    if (![allowedPermissions containsObject:permission]) [allowedPermissions addObject:permission];
  }

  NSMutableArray<NSString*>* allowedOrigins =
      PermissionUniqueMutableArray(PermissionManifestOrigins(manifest, @"permissions"));
  for (NSString* origin in PermissionManifestOrigins(manifest, @"host_permissions")) {
    if (![allowedOrigins containsObject:origin]) [allowedOrigins addObject:origin];
  }
  for (NSString* origin in PermissionManifestOrigins(manifest, @"optional_permissions")) {
    if (![allowedOrigins containsObject:origin]) [allowedOrigins addObject:origin];
  }
  for (NSString* origin in PermissionManifestOrigins(manifest, @"optional_host_permissions")) {
    if (![allowedOrigins containsObject:origin]) [allowedOrigins addObject:origin];
  }

  for (NSString* permission in PermissionStringArray(request[@"permissions"])) {
    if (![allowedPermissions containsObject:permission]) return NO;
  }
  for (NSString* origin in PermissionStringArray(request[@"origins"])) {
    if (!PermissionOriginGranted(allowedOrigins, origin)) return NO;
  }
  return YES;
}

NSDictionary* NormalizedPermissionRequest(NSDictionary* raw) {
  return @{
    @"permissions" : PermissionStringArray(raw[@"permissions"]),
    @"origins" : PermissionStringArray(raw[@"origins"])
  };
}

NSDictionary* HandlePermissions(NSString* method,
                                NSDictionary* args,
                                NSString* extensionID) {
  NSDictionary* ext = EnabledExtensionRecordForID(extensionID);
  NSDictionary* manifest = ManifestForExtension(ext ?: @{}) ?: @{};
  NSDictionary* rawRequest = [args[@"permissions"] isKindOfClass:NSDictionary.class]
      ? args[@"permissions"]
      : @{};
  NSDictionary* request = NormalizedPermissionRequest(rawRequest);
  NSDictionary* effective = EffectivePermissions(manifest, extensionID);

  if ([method isEqualToString:@"permissions.contains"]) {
    return @{@"result" : @(PermissionSetContains(effective, request))};
  }

  if ([method isEqualToString:@"permissions.getAll"]) {
    return @{@"result" : effective};
  }

  if ([method isEqualToString:@"permissions.request"]) {
    if (!PermissionRequestAllowed(manifest, request)) {
      return @{@"result" : @NO};
    }
    NSDictionary* stored = StoredOptionalPermissions(extensionID);
    NSMutableArray<NSString*>* permissions =
        PermissionUniqueMutableArray(PermissionStringArray(stored[@"permissions"]));
    NSMutableArray<NSString*>* origins =
        PermissionUniqueMutableArray(PermissionStringArray(stored[@"origins"]));
    for (NSString* permission in PermissionStringArray(request[@"permissions"])) {
      if (![permissions containsObject:permission]) [permissions addObject:permission];
    }
    for (NSString* origin in PermissionStringArray(request[@"origins"])) {
      if (![origins containsObject:origin]) [origins addObject:origin];
    }
    [[NSUserDefaults standardUserDefaults] setObject:@{
      @"permissions" : permissions,
      @"origins" : origins
    } forKey:PermissionsDefaultsKey(extensionID)];
    [MoriBrowserView dispatchExtensionEvent:@"permissions.onAdded"
                                          args:@[ request ]
                                forExtensionID:extensionID];
    return @{@"result" : @YES};
  }

  if ([method isEqualToString:@"permissions.remove"]) {
    NSDictionary* stored = StoredOptionalPermissions(extensionID);
    NSMutableArray<NSString*>* permissions =
        PermissionUniqueMutableArray(PermissionStringArray(stored[@"permissions"]));
    NSMutableArray<NSString*>* origins =
        PermissionUniqueMutableArray(PermissionStringArray(stored[@"origins"]));
    BOOL removed = NO;
    for (NSString* permission in PermissionStringArray(request[@"permissions"])) {
      if ([permissions containsObject:permission]) {
        [permissions removeObject:permission];
        removed = YES;
      }
    }
    for (NSString* origin in PermissionStringArray(request[@"origins"])) {
      if ([origins containsObject:origin]) {
        [origins removeObject:origin];
        removed = YES;
      }
    }
    [[NSUserDefaults standardUserDefaults] setObject:@{
      @"permissions" : permissions,
      @"origins" : origins
    } forKey:PermissionsDefaultsKey(extensionID)];
    if (removed) {
      [MoriBrowserView dispatchExtensionEvent:@"permissions.onRemoved"
                                            args:@[ request ]
                                  forExtensionID:extensionID];
    }
    return @{@"result" : @(removed)};
  }

  return @{@"error" : [NSString stringWithFormat:@"Unsupported permissions method: %@", method]};
}

NSDictionary* ProxyDefaultValue() {
  return @{@"mode" : @"system"};
}

NSDictionary* StoredProxySettings() {
  id raw = [[NSUserDefaults standardUserDefaults]
      objectForKey:ProxySettingsDefaultsKey()];
  if ([raw isKindOfClass:NSData.class]) {
    id json = [NSJSONSerialization JSONObjectWithData:raw options:0 error:nil];
    return [json isKindOfClass:NSDictionary.class] ? (NSDictionary*)json : @{};
  }
  return [raw isKindOfClass:NSDictionary.class] ? (NSDictionary*)raw : @{};
}

void StoreProxySettings(NSString* extensionID, NSDictionary* value) {
  NSDictionary* record = @{
    @"extensionId" : extensionID ?: @"",
    @"value" : value ?: ProxyDefaultValue()
  };
  NSData* data = [NSJSONSerialization dataWithJSONObject:record
                                                 options:0
                                                   error:nil];
  NSUserDefaults* defaults = [NSUserDefaults standardUserDefaults];
  if (data) {
    [defaults setObject:data forKey:ProxySettingsDefaultsKey()];
  } else {
    [defaults setObject:record forKey:ProxySettingsDefaultsKey()];
  }
  [defaults synchronize];
}

NSString* ProxyServerSpec(NSDictionary* proxy) {
  if (![proxy isKindOfClass:NSDictionary.class]) return @"";
  NSString* host = [proxy[@"host"] isKindOfClass:NSString.class]
      ? proxy[@"host"]
      : @"";
  if (host.length == 0) return @"";
  NSString* scheme = [proxy[@"scheme"] isKindOfClass:NSString.class]
      ? [(NSString*)proxy[@"scheme"] lowercaseString]
      : @"http";
  NSInteger port = [proxy[@"port"] respondsToSelector:@selector(integerValue)]
      ? [proxy[@"port"] integerValue]
      : 0;
  if (port <= 0 || port > 65535) {
    return [NSString stringWithFormat:@"%@://%@", scheme, host];
  }
  return [NSString stringWithFormat:@"%@://%@:%ld",
                                    scheme, host, (long)port];
}

NSString* ProxyRulesServerString(NSDictionary* rules) {
  if (![rules isKindOfClass:NSDictionary.class]) return @"";
  NSString* single = ProxyServerSpec(rules[@"singleProxy"]);
  if (single.length > 0) return single;

  NSMutableArray<NSString*>* parts = [NSMutableArray array];
  NSDictionary<NSString*, NSString*>* names = @{
    @"proxyForHttp" : @"http",
    @"proxyForHttps" : @"https",
    @"proxyForFtp" : @"ftp",
    @"fallbackProxy" : @"fallback"
  };
  for (NSString* key in @[
         @"proxyForHttp", @"proxyForHttps", @"proxyForFtp", @"fallbackProxy"
       ]) {
    NSString* spec = ProxyServerSpec(rules[key]);
    if (spec.length > 0) {
      [parts addObject:[NSString stringWithFormat:@"%@=%@",
                                                  names[key], spec]];
    }
  }
  return [parts componentsJoinedByString:@";"];
}

NSString* ProxyBypassListString(id bypassList) {
  NSMutableArray<NSString*>* out = [NSMutableArray array];
  if ([bypassList isKindOfClass:NSArray.class]) {
    for (id item in (NSArray*)bypassList) {
      if ([item isKindOfClass:NSString.class] && [item length] > 0) {
        [out addObject:item];
      }
    }
  }
  return [out componentsJoinedByString:@";"];
}

CefRefPtr<CefValue> CefProxyPreferenceValue(NSDictionary* value) {
  NSString* mode = [value[@"mode"] isKindOfClass:NSString.class]
      ? [(NSString*)value[@"mode"] lowercaseString]
      : @"system";
  CefRefPtr<CefDictionaryValue> dict = CefDictionaryValue::Create();
  dict->SetString(CefString("mode"), CefString(mode.UTF8String));

  if ([mode isEqualToString:@"pac_script"]) {
    NSDictionary* pac = [value[@"pacScript"] isKindOfClass:NSDictionary.class]
        ? value[@"pacScript"]
        : @{};
    NSString* pacURL = [pac[@"url"] isKindOfClass:NSString.class] ? pac[@"url"] : @"";
    if (pacURL.length > 0) {
      dict->SetString(CefString("pac_url"), CefString(pacURL.UTF8String));
    }
  } else if ([mode isEqualToString:@"fixed_servers"]) {
    NSDictionary* rules = [value[@"rules"] isKindOfClass:NSDictionary.class]
        ? value[@"rules"]
        : @{};
    NSString* server = ProxyRulesServerString(rules);
    if (server.length > 0) {
      dict->SetString(CefString("server"), CefString(server.UTF8String));
    }
    NSString* bypass = ProxyBypassListString(rules[@"bypassList"]);
    if (bypass.length > 0) {
      dict->SetString(CefString("bypass_list"), CefString(bypass.UTF8String));
    }
  }

  CefRefPtr<CefValue> pref = CefValue::Create();
  pref->SetDictionary(dict);
  return pref;
}

BOOL ApplyProxySettingsToCEF(NSDictionary* value, NSString** error) {
  CefRefPtr<CefRequestContext> context = CefRequestContext::GetGlobalContext();
  if (!context) {
    if (error) *error = @"CEF request context is unavailable.";
    return NO;
  }

  CefString cefError;
  CefRefPtr<CefValue> pref = CefProxyPreferenceValue(value ?: ProxyDefaultValue());
  bool ok = context->SetPreference(CefString("proxy"), pref, cefError);
  if (!ok) {
    NSString* mode = [value[@"mode"] isKindOfClass:NSString.class]
        ? [(NSString*)value[@"mode"] lowercaseString]
        : @"system";
    if ([mode isEqualToString:@"system"]) {
      cefError.clear();
      ok = context->SetPreference(CefString("proxy"), nullptr, cefError);
    }
  }
  if (!ok && error) {
    std::string messageUTF8 = cefError.ToString();
    NSString* message = [NSString stringWithUTF8String:messageUTF8.c_str()] ?: @"";
    *error = message.length > 0 ? message : @"Could not apply proxy settings.";
  }
  return ok;
}

BOOL ProxyModeAllowed(NSString* mode) {
  return [@[
    @"direct", @"auto_detect", @"pac_script", @"fixed_servers", @"system"
  ] containsObject:mode ?: @""];
}

NSDictionary* HandleProxySettings(NSString* method,
                                  NSDictionary* args,
                                  NSString* extensionID) {
  NSDictionary* stored = StoredProxySettings();
  NSString* owner = [stored[@"extensionId"] isKindOfClass:NSString.class]
      ? stored[@"extensionId"]
      : @"";
  NSDictionary* currentValue = [stored[@"value"] isKindOfClass:NSDictionary.class]
      ? stored[@"value"]
      : ProxyDefaultValue();
  BOOL controlledByThis =
      owner.length > 0 &&
      [owner caseInsensitiveCompare:extensionID ?: @""] == NSOrderedSame;
  NSString* level = controlledByThis ? @"controlled_by_this_extension"
      : (owner.length > 0 ? @"controlled_by_other_extensions"
                          : @"controllable_by_this_extension");

  if ([method isEqualToString:@"proxy.settings.get"]) {
    return @{@"result" : @{@"levelOfControl" : level,
                           @"value" : currentValue}};
  }

  if ([method isEqualToString:@"proxy.settings.set"]) {
    if (owner.length > 0 && !controlledByThis) {
      return @{@"error" : @"Proxy settings are controlled by another extension."};
    }
    NSDictionary* details = [args[@"details"] isKindOfClass:NSDictionary.class]
        ? args[@"details"]
        : @{};
    NSDictionary* value = [details[@"value"] isKindOfClass:NSDictionary.class]
        ? details[@"value"]
        : nil;
    NSString* mode = [value[@"mode"] isKindOfClass:NSString.class]
        ? [(NSString*)value[@"mode"] lowercaseString]
        : @"";
    if (!value || !ProxyModeAllowed(mode)) {
      return @{@"error" : @"proxy.settings.set requires a valid proxy mode."};
    }
    NSString* applyError = nil;
    if (!ApplyProxySettingsToCEF(value, &applyError)) {
      return @{@"error" : applyError ?: @"Could not apply proxy settings."};
    }
    StoreProxySettings(extensionID, value);
    NSDictionary* change = @{@"levelOfControl" : @"controlled_by_this_extension",
                             @"value" : value};
    [MoriBrowserView dispatchExtensionEvent:@"proxy.settings.onChange"
                                       args:@[ change ]
                             forExtensionID:extensionID];
    return @{@"result" : [NSNull null]};
  }

  if ([method isEqualToString:@"proxy.settings.clear"]) {
    if (owner.length > 0 && !controlledByThis) {
      return @{@"error" : @"Proxy settings are controlled by another extension."};
    }
    NSString* applyError = nil;
    NSDictionary* value = ProxyDefaultValue();
    if (!ApplyProxySettingsToCEF(value, &applyError)) {
      return @{@"error" : applyError ?: @"Could not clear proxy settings."};
    }
    NSUserDefaults* defaults = [NSUserDefaults standardUserDefaults];
    [defaults removeObjectForKey:ProxySettingsDefaultsKey()];
    [defaults synchronize];
    NSDictionary* change = @{@"levelOfControl" : @"controllable_by_this_extension",
                             @"value" : value};
    [MoriBrowserView dispatchExtensionEvent:@"proxy.settings.onChange"
                                       args:@[ change ]
                             forExtensionID:extensionID];
    return @{@"result" : [NSNull null]};
  }

  return @{@"error" : [NSString stringWithFormat:@"Unsupported proxy method: %@", method]};
}

// Extension storage is persisted as JSON-encoded NSData rather than a raw
// dictionary. chrome.storage values come straight from JSON, so they can hold
// `null` (NSNull), which NSUserDefaults' property-list store cannot serialize —
// a single null field anywhere in the object made the whole write throw
// "Attempt to insert non-property list object", silently dropping the write.
// That is exactly how a signed-in session (Proton Pass stores it under
// chrome.storage.session with several optional null fields) failed to persist,
// signing the user back out. JSON round-trips null and nested structures
// faithfully, so the session survives.
NSMutableDictionary* ExtensionStorage(NSString* extensionId, NSString* area) {
  NSString* key = ExtensionStorageDefaultsKey(extensionId, area);
  NSUserDefaults* defaults = [NSUserDefaults standardUserDefaults];
  id raw = [defaults objectForKey:key];
  if ([raw isKindOfClass:[NSData class]]) {
    id parsed = [NSJSONSerialization JSONObjectWithData:raw
                                                options:NSJSONReadingMutableContainers
                                                  error:nil];
    if ([parsed isKindOfClass:[NSDictionary class]]) {
      return [parsed mutableCopy];
    }
  } else if ([raw isKindOfClass:[NSDictionary class]]) {
    // Legacy plist-dictionary value written before the JSON migration; adopt it
    // (the next write re-encodes it as JSON Data).
    return [raw mutableCopy];
  }
  return [NSMutableDictionary dictionary];
}

NSMutableDictionary* ExtensionStorage(NSString* extensionId) {
  return ExtensionStorage(extensionId, @"local");
}

// Persist a storage area as JSON Data. Returns nil on success, or an error
// string if the store cannot be encoded.
NSString* WriteExtensionStorage(NSString* extensionId, NSString* area,
                                NSDictionary* store) {
  NSString* key = ExtensionStorageDefaultsKey(extensionId, area);
  if (![NSJSONSerialization isValidJSONObject:store]) {
    return @"Storage value is not JSON-serializable.";
  }
  NSError* error = nil;
  NSData* data = [NSJSONSerialization dataWithJSONObject:store
                                                 options:0
                                                   error:&error];
  if (!data) {
    return error.localizedDescription ?: @"Failed to encode storage value.";
  }
  [[NSUserDefaults standardUserDefaults] setObject:data forKey:key];
  return nil;
}

BOOL ParseStorageMethod(NSString* method, NSString** area, NSString** operation) {
  NSArray<NSString*>* parts = [method componentsSeparatedByString:@"."];
  if (parts.count != 3 || ![parts[0] isEqualToString:@"storage"]) return NO;
  NSString* candidateArea = parts[1];
	  if (![candidateArea isEqualToString:@"local"] &&
	      ![candidateArea isEqualToString:@"sync"] &&
	      ![candidateArea isEqualToString:@"session"] &&
	      ![candidateArea isEqualToString:@"managed"]) {
	    return NO;
	  }
  if (area) *area = candidateArea;
  if (operation) *operation = parts[2];
  return YES;
}

id StorageGetResult(NSDictionary* store, id keys) {
  if (!keys || keys == [NSNull null]) {
    return store ?: @{};
  }
  if ([keys isKindOfClass:[NSString class]]) {
    id value = store[keys];
    return value ? @{keys : value} : @{};
  }
  if ([keys isKindOfClass:[NSArray class]]) {
    NSMutableDictionary* out = [NSMutableDictionary dictionary];
    for (id key in (NSArray*)keys) {
      if (![key isKindOfClass:[NSString class]]) continue;
      id value = store[key];
      if (value) out[key] = value;
    }
    return out;
  }
  if ([keys isKindOfClass:[NSDictionary class]]) {
    NSMutableDictionary* out = [NSMutableDictionary dictionary];
    for (id key in (NSDictionary*)keys) {
      if (![key isKindOfClass:[NSString class]]) continue;
      id value = store[key] ?: ((NSDictionary*)keys)[key];
      if (value && value != [NSNull null]) out[key] = value;
    }
    return out;
  }
	return @{};
}

NSUInteger JSONByteCount(id object) {
  if (!object || object == [NSNull null]) return 0;
  if (![NSJSONSerialization isValidJSONObject:object]) {
    return [[object description] lengthOfBytesUsingEncoding:NSUTF8StringEncoding];
  }
  NSData* data = [NSJSONSerialization dataWithJSONObject:object options:0 error:nil];
  return data.length;
}

NSNumber* StorageBytesInUse(NSDictionary* store, id keys) {
  id subset = StorageGetResult(store ?: @{}, keys);
  return @(JSONByteCount(subset ?: @{}));
}

NSMutableDictionary<NSNumber*, NSDictionary*>* ContextMenuCommandRegistry() {
  static NSMutableDictionary<NSNumber*, NSDictionary*>* registry = nil;
  static dispatch_once_t onceToken;
  dispatch_once(&onceToken, ^{
    registry = [NSMutableDictionary dictionary];
  });
  return registry;
}

NSString* ContextMenuItemID(id raw) {
  if ([raw isKindOfClass:NSString.class]) return (NSString*)raw;
  if ([raw respondsToSelector:@selector(stringValue)]) return [raw stringValue];
  return nil;
}

NSArray<NSDictionary*>* ContextMenuItems(NSString* extensionID) {
  NSArray* stored = [[NSUserDefaults standardUserDefaults]
      arrayForKey:ContextMenusDefaultsKey(extensionID)];
  NSMutableArray<NSDictionary*>* out = [NSMutableArray array];
  for (id item in stored) {
    if ([item isKindOfClass:NSDictionary.class]) [out addObject:item];
  }
  return out;
}

void PersistContextMenuItems(NSString* extensionID, NSArray<NSDictionary*>* items) {
  [[NSUserDefaults standardUserDefaults] setObject:items ?: @[]
                                            forKey:ContextMenusDefaultsKey(extensionID)];
}

NSDictionary* HandleContextMenus(NSString* method,
                                 NSDictionary* args,
                                 NSString* extensionID) {
  if (!EnabledExtensionRecordForID(extensionID)) {
    return @{@"error" : @"Extension is not enabled."};
  }

  NSMutableArray<NSDictionary*>* items =
      [ContextMenuItems(extensionID) mutableCopy] ?: [NSMutableArray array];

  if ([method isEqualToString:@"contextMenus.create"]) {
    NSDictionary* props =
        [args[@"createProperties"] isKindOfClass:NSDictionary.class]
            ? args[@"createProperties"]
            : @{};
    NSMutableDictionary* item = [props mutableCopy];
    NSString* itemID = ContextMenuItemID(item[@"id"]);
    if (itemID.length == 0) itemID = NSUUID.UUID.UUIDString;
    item[@"id"] = itemID;
    item[@"extensionId"] = extensionID;

    NSIndexSet* existing = [items indexesOfObjectsPassingTest:^BOOL(
        NSDictionary* obj, NSUInteger idx, BOOL* stop) {
      return [ContextMenuItemID(obj[@"id"]) isEqualToString:itemID];
    }];
    if (existing.count > 0) [items removeObjectsAtIndexes:existing];
    [items addObject:item];
    PersistContextMenuItems(extensionID, items);
    return @{@"result" : itemID};
  }

  if ([method isEqualToString:@"contextMenus.update"]) {
    NSString* itemID = ContextMenuItemID(args[@"id"]);
    NSDictionary* props =
        [args[@"updateProperties"] isKindOfClass:NSDictionary.class]
            ? args[@"updateProperties"]
            : @{};
    if (itemID.length == 0) return @{@"error" : @"Missing context menu id."};
    for (NSUInteger i = 0; i < items.count; i++) {
      if ([ContextMenuItemID(items[i][@"id"]) isEqualToString:itemID]) {
        NSMutableDictionary* next = [items[i] mutableCopy];
        [next addEntriesFromDictionary:props];
        next[@"id"] = itemID;
        next[@"extensionId"] = extensionID;
        items[i] = next;
        PersistContextMenuItems(extensionID, items);
        return @{@"result" : [NSNull null]};
      }
    }
    return @{@"error" : @"No context menu item with that id."};
  }

  if ([method isEqualToString:@"contextMenus.remove"]) {
    NSString* itemID = ContextMenuItemID(args[@"id"]);
    NSIndexSet* remove = [items indexesOfObjectsPassingTest:^BOOL(
        NSDictionary* obj, NSUInteger idx, BOOL* stop) {
      return [ContextMenuItemID(obj[@"id"]) isEqualToString:itemID];
    }];
    if (remove.count > 0) [items removeObjectsAtIndexes:remove];
    PersistContextMenuItems(extensionID, items);
    return @{@"result" : [NSNull null]};
  }

  if ([method isEqualToString:@"contextMenus.removeAll"]) {
    PersistContextMenuItems(extensionID, @[]);
    return @{@"result" : [NSNull null]};
  }

  return @{@"error" : [NSString stringWithFormat:@"Unsupported contextMenus method: %@", method]};
}

NSString* ContextMenuParamString(const CefString& value) {
  std::string utf8 = value.ToString();
  return [NSString stringWithUTF8String:utf8.c_str()] ?: @"";
}

NSString* ContextMenuMediaType(CefRefPtr<CefContextMenuParams> params) {
  if (!params) return @"";
  switch (params->GetMediaType()) {
    case CM_MEDIATYPE_IMAGE:
      return @"image";
    case CM_MEDIATYPE_VIDEO:
      return @"video";
    case CM_MEDIATYPE_AUDIO:
      return @"audio";
    default:
      return @"";
  }
}

NSDictionary* ContextMenuClickInfo(CefRefPtr<CefContextMenuParams> params,
                                   NSDictionary* item) {
  NSMutableDictionary* info = [NSMutableDictionary dictionary];
  info[@"menuItemId"] = item[@"id"] ?: @"";
  if (item[@"parentId"]) info[@"parentMenuItemId"] = item[@"parentId"];
  if (params) {
    NSString* pageURL = ContextMenuParamString(params->GetPageUrl());
    NSString* frameURL = ContextMenuParamString(params->GetFrameUrl());
    NSString* linkURL = ContextMenuParamString(params->GetLinkUrl());
    NSString* sourceURL = ContextMenuParamString(params->GetSourceUrl());
    NSString* selection = ContextMenuParamString(params->GetSelectionText());
    NSString* mediaType = ContextMenuMediaType(params);
    if (pageURL.length > 0) info[@"pageUrl"] = pageURL;
    if (frameURL.length > 0) info[@"frameUrl"] = frameURL;
    if (linkURL.length > 0) info[@"linkUrl"] = linkURL;
    if (sourceURL.length > 0) info[@"srcUrl"] = sourceURL;
    if (selection.length > 0) info[@"selectionText"] = selection;
    if (mediaType.length > 0) info[@"mediaType"] = mediaType;
    info[@"editable"] = @(params->IsEditable());
  }
  if ([item[@"checked"] respondsToSelector:@selector(boolValue)]) {
    info[@"checked"] = @([item[@"checked"] boolValue]);
  }
  return info;
}

BOOL ContextMenuArrayContains(NSArray* values, NSString* value) {
  for (id item in values) {
    if ([item isKindOfClass:NSString.class] &&
        [((NSString*)item) caseInsensitiveCompare:value] == NSOrderedSame) {
      return YES;
    }
  }
  return NO;
}

BOOL ContextMenuItemMatches(NSDictionary* item,
                            CefRefPtr<CefContextMenuParams> params) {
  if ([item[@"visible"] respondsToSelector:@selector(boolValue)] &&
      ![item[@"visible"] boolValue]) {
    return NO;
  }

  NSArray* contexts = [item[@"contexts"] isKindOfClass:NSArray.class]
      ? item[@"contexts"]
      : @[ @"page" ];
  if (ContextMenuArrayContains(contexts, @"all")) return YES;

  NSString* mediaType = ContextMenuMediaType(params);
  if (ContextMenuArrayContains(contexts, @"page")) return YES;
  if (params) {
    if (params->IsEditable() && ContextMenuArrayContains(contexts, @"editable")) return YES;
    if (ContextMenuParamString(params->GetSelectionText()).length > 0 &&
        ContextMenuArrayContains(contexts, @"selection")) return YES;
    if (ContextMenuParamString(params->GetLinkUrl()).length > 0 &&
        ContextMenuArrayContains(contexts, @"link")) return YES;
    if (mediaType.length > 0 && ContextMenuArrayContains(contexts, mediaType)) return YES;
    NSString* pageURL = ContextMenuParamString(params->GetPageUrl());
    NSString* frameURL = ContextMenuParamString(params->GetFrameUrl());
    if (frameURL.length > 0 && ![frameURL isEqualToString:pageURL] &&
        ContextMenuArrayContains(contexts, @"frame")) return YES;
  }
  return NO;
}

NSString* ContextMenuDisplayTitle(NSDictionary* item,
                                  CefRefPtr<CefContextMenuParams> params) {
  NSString* title = [item[@"title"] isKindOfClass:NSString.class]
      ? item[@"title"]
      : @"";
  if (title.length == 0) return nil;
  NSString* selection = params ? ContextMenuParamString(params->GetSelectionText()) : @"";
  if (selection.length > 0) {
    title = [title stringByReplacingOccurrencesOfString:@"%s" withString:selection];
  }
  return title;
}

NSArray<NSDictionary*>* MatchingContextMenuItems(CefRefPtr<CefContextMenuParams> params) {
  NSMutableArray<NSDictionary*>* out = [NSMutableArray array];
  for (NSDictionary* ext in EnabledExtensionRecords()) {
    NSString* extensionID = [ext[@"id"] isKindOfClass:NSString.class] ? ext[@"id"] : @"";
    for (NSDictionary* item in ContextMenuItems(extensionID)) {
      if (ContextMenuItemMatches(item, params)) [out addObject:item];
    }
  }
  return out;
}

NSString* ExtensionExecuteScriptSource(NSDictionary* ext,
                                       NSDictionary* details,
                                       NSString* requestId,
                                       NSString* extensionId) {
  NSMutableString* source = [NSMutableString string];

  NSArray* files = [details[@"files"] isKindOfClass:[NSArray class]]
      ? details[@"files"]
      : nil;
  for (id file in files) {
    if (![file isKindOfClass:[NSString class]]) continue;
    NSString* fileSource = ExtensionFileText(ext, file);
    if (fileSource.length == 0) continue;
    [source appendString:fileSource];
    [source appendString:@"\n"];
  }

  NSString* inlineCode = [details[@"code"] isKindOfClass:[NSString class]]
      ? details[@"code"]
      : nil;
  if (inlineCode.length > 0) {
    [source appendString:inlineCode];
    [source appendString:@"\n"];
  }

  NSString* funcSource = [details[@"funcSource"] isKindOfClass:[NSString class]]
      ? details[@"funcSource"]
      : nil;
  id funcArgs = [details[@"args"] isKindOfClass:[NSArray class]]
      ? details[@"args"]
      : @[];
  if (funcSource.length > 0) {
    [source appendFormat:
        @"\n(function(){"
         "function __moriResolve(__moriValue){"
         "var __moriResult=__moriValue===undefined?null:__moriValue;"
         "try{JSON.stringify(__moriResult);}"
         "catch(__moriJSONError){__moriResult=String(__moriResult);}"
         "console.info('__MORI_SCRIPTING_RESULT__'+JSON.stringify({"
         "requestId:%@,extensionId:%@,"
         "result:[{frameId:0,result:__moriResult}]"
         "}));"
         "}"
         "function __moriReject(__moriError){"
         "console.info('__MORI_SCRIPTING_RESULT__'+JSON.stringify({"
         "requestId:%@,extensionId:%@,"
         "error:String((__moriError&&__moriError.message)||__moriError)"
         "}));"
         "}"
         "try{"
         "var __moriValue=(%@).apply(null,%@);"
         "if(__moriValue&&typeof __moriValue.then==='function'){"
         "__moriValue.then(__moriResolve,__moriReject);"
         "}else{__moriResolve(__moriValue);}"
         "}catch(__moriError){__moriReject(__moriError);}"
         "})();\n",
        JSStringLiteral(requestId), JSStringLiteral(extensionId),
        JSStringLiteral(requestId), JSStringLiteral(extensionId),
        funcSource, JSONStringLiteral(funcArgs)];
  } else if (requestId.length > 0 && extensionId.length > 0) {
    [source appendFormat:
        @"\nconsole.info('__MORI_SCRIPTING_RESULT__'+JSON.stringify({"
         "requestId:%@,extensionId:%@,result:[{frameId:0,result:null}]"
         "}));\n",
        JSStringLiteral(requestId), JSStringLiteral(extensionId)];
  }

  return source.length > 0 ? source : nil;
}

NSString* ExtensionInsertCSSSource(NSDictionary* ext, NSDictionary* details) {
  NSMutableString* css = [NSMutableString string];

  NSString* inlineCSS = [details[@"css"] isKindOfClass:[NSString class]]
      ? details[@"css"]
      : nil;
  if (inlineCSS.length > 0) {
    [css appendString:inlineCSS];
    [css appendString:@"\n"];
  }

  NSArray* files = [details[@"files"] isKindOfClass:[NSArray class]]
      ? details[@"files"]
      : nil;
  for (id file in files) {
    if (![file isKindOfClass:[NSString class]]) continue;
    NSString* fileCSS = ExtensionFileText(ext, file);
    if (fileCSS.length == 0) continue;
    [css appendString:fileCSS];
    [css appendString:@"\n"];
  }

  if (css.length == 0) return nil;
  NSString* extensionID =
      [ext[@"id"] isKindOfClass:[NSString class]] ? ext[@"id"] : @"";
  return [NSString stringWithFormat:
      @"(function(){var s=document.createElement('style');"
       "s.dataset.moriScripting=%@;"
       "s.dataset.moriScriptingCss=%@;"
       "s.textContent=%@;"
       "(document.head||document.documentElement).appendChild(s);})();",
      JSStringLiteral(extensionID), JSStringLiteral(css), JSStringLiteral(css)];
}

NSString* ExtensionRemoveCSSSource(NSDictionary* ext, NSDictionary* details) {
  NSString* inserted = ExtensionInsertCSSSource(ext, details);
  if (inserted.length == 0) return nil;

  NSMutableString* css = [NSMutableString string];
  NSString* inlineCSS = [details[@"css"] isKindOfClass:[NSString class]]
      ? details[@"css"]
      : nil;
  if (inlineCSS.length > 0) {
    [css appendString:inlineCSS];
    [css appendString:@"\n"];
  }

  NSArray* files = [details[@"files"] isKindOfClass:[NSArray class]]
      ? details[@"files"]
      : nil;
  for (id file in files) {
    if (![file isKindOfClass:[NSString class]]) continue;
    NSString* fileCSS = ExtensionFileText(ext, file);
    if (fileCSS.length == 0) continue;
    [css appendString:fileCSS];
    [css appendString:@"\n"];
  }

  NSString* extensionID =
      [ext[@"id"] isKindOfClass:[NSString class]] ? ext[@"id"] : @"";
  return [NSString stringWithFormat:
      @"(function(){"
       "var ext=%@,css=%@;"
       "document.querySelectorAll('style[data-mori-scripting]').forEach(function(s){"
       "if(s.dataset.moriScripting===ext&&s.dataset.moriScriptingCss===css){"
       "s.remove();"
       "}"
       "});"
       "})();",
      JSStringLiteral(extensionID), JSStringLiteral(css)];
}

NSDictionary* BuildScriptingBridgePayload(NSString* method,
                                          NSDictionary* args,
                                          NSDictionary* ext,
                                          NSString* requestId,
                                          NSString* extensionId) {
  NSDictionary* details = [args[@"details"] isKindOfClass:[NSDictionary class]]
      ? args[@"details"]
      : @{};
  NSDictionary* target = [details[@"target"] isKindOfClass:[NSDictionary class]]
      ? details[@"target"]
      : @{};

  NSString* source = nil;
  if ([method isEqualToString:@"scripting.executeScript"]) {
    source = ExtensionExecuteScriptSource(ext, details, requestId, extensionId);
  } else if ([method isEqualToString:@"scripting.insertCSS"]) {
    source = ExtensionInsertCSSSource(ext, details);
  } else if ([method isEqualToString:@"scripting.removeCSS"]) {
    source = ExtensionRemoveCSSSource(ext, details);
  }

  if (source.length == 0) {
    return @{@"error" : @"No extension script source found."};
  }
  NSMutableDictionary* payload =
      [@{@"target" : target, @"source" : source} mutableCopy];
  if ([method isEqualToString:@"scripting.executeScript"]) {
    payload[@"deferred"] = @YES;
    payload[@"requestId"] = requestId ?: @"";
    payload[@"extensionId"] = extensionId ?: @"";
  }
  return payload;
}

NSString* CookieString(const cef_string_t& value) {
  return @(CefString(&value).ToString().c_str());
}

NSNumber* CookieTime(cef_basetime_t value) {
  cef_time_t cef_time{};
  double seconds = 0;
  if (cef_time_from_basetime(value, &cef_time) &&
      cef_time_to_doublet(&cef_time, &seconds) && seconds > 0) {
    return @(seconds);
  }
  return nil;
}

cef_basetime_t CookieBaseTimeFromSeconds(double seconds) {
  cef_time_t cef_time{};
  cef_basetime_t base_time{};
  if (seconds > 0 && cef_time_from_doublet(seconds, &cef_time)) {
    cef_time_to_basetime(&cef_time, &base_time);
  }
  return base_time;
}

NSString* CookieURL(NSDictionary* details) {
  NSString* explicitURL = [details[@"url"] isKindOfClass:NSString.class]
      ? details[@"url"]
      : nil;
  if (explicitURL.length > 0) return explicitURL;

  NSString* domain = [details[@"domain"] isKindOfClass:NSString.class]
      ? details[@"domain"]
      : @"";
  if (domain.length == 0) return nil;
  NSString* host = [domain hasPrefix:@"."] ? [domain substringFromIndex:1] : domain;
  BOOL secure = [details[@"secure"] respondsToSelector:@selector(boolValue)] &&
                [details[@"secure"] boolValue];
  NSString* path = [details[@"path"] isKindOfClass:NSString.class]
      ? details[@"path"]
      : @"/";
  if (![path hasPrefix:@"/"]) path = [@"/" stringByAppendingString:path];
  return [NSString stringWithFormat:@"%@://%@%@", secure ? @"https" : @"http",
                                    host, path];
}

NSDictionary* CookieDictionary(const CefCookie& cookie) {
  NSMutableDictionary* result = [NSMutableDictionary dictionary];
  NSString* name = CookieString(cookie.name);
  NSString* value = CookieString(cookie.value);
  NSString* domain = CookieString(cookie.domain);
  NSString* path = CookieString(cookie.path);
  result[@"name"] = name ?: @"";
  result[@"value"] = value ?: @"";
  result[@"domain"] = domain ?: @"";
  result[@"hostOnly"] = @(!(domain.length > 0 && [domain hasPrefix:@"."]));
  result[@"path"] = path.length > 0 ? path : @"/";
  result[@"secure"] = @(cookie.secure != 0);
  result[@"httpOnly"] = @(cookie.httponly != 0);
  result[@"session"] = @(cookie.has_expires == 0);
  result[@"storeId"] = @"0";
  if (cookie.has_expires) {
    NSNumber* expirationDate = CookieTime(cookie.expires);
    if (expirationDate) result[@"expirationDate"] = expirationDate;
  }
  NSString* sameSite = @"unspecified";
  if (cookie.same_site == CEF_COOKIE_SAME_SITE_LAX_MODE) {
    sameSite = @"lax";
  } else if (cookie.same_site == CEF_COOKIE_SAME_SITE_STRICT_MODE) {
    sameSite = @"strict";
  } else if (cookie.same_site == CEF_COOKIE_SAME_SITE_NO_RESTRICTION) {
    sameSite = @"no_restriction";
  }
  result[@"sameSite"] = sameSite;
  return result;
}

BOOL CookieMatches(NSDictionary* cookie, NSDictionary* filter) {
  NSString* name = [filter[@"name"] isKindOfClass:NSString.class] ? filter[@"name"] : nil;
  if (name.length > 0 && ![cookie[@"name"] isEqualToString:name]) return NO;

  NSString* domain = [filter[@"domain"] isKindOfClass:NSString.class]
      ? ((NSString*)filter[@"domain"]).lowercaseString
      : nil;
  if (domain.length > 0) {
    NSString* cookieDomain =
        [cookie[@"domain"] isKindOfClass:NSString.class] ? cookie[@"domain"] : @"";
    NSString* normalizedCookieDomain =
        [cookieDomain hasPrefix:@"."] ? [cookieDomain substringFromIndex:1] : cookieDomain;
    NSString* normalizedDomain =
        [domain hasPrefix:@"."] ? [domain substringFromIndex:1] : domain;
    if ([normalizedCookieDomain.lowercaseString rangeOfString:normalizedDomain].location ==
        NSNotFound) {
      return NO;
    }
  }

  NSString* path = [filter[@"path"] isKindOfClass:NSString.class] ? filter[@"path"] : nil;
  if (path.length > 0 && ![cookie[@"path"] isEqualToString:path]) return NO;

  if ([filter[@"secure"] respondsToSelector:@selector(boolValue)] &&
      [filter[@"secure"] boolValue] != [cookie[@"secure"] boolValue]) {
    return NO;
  }
  if ([filter[@"session"] respondsToSelector:@selector(boolValue)] &&
      [filter[@"session"] boolValue] != [cookie[@"session"] boolValue]) {
    return NO;
  }
  return YES;
}

NSString* CookieHeaderValue(NSArray<NSDictionary*>* cookies) {
  NSMutableArray<NSString*>* parts = [NSMutableArray array];
  for (NSDictionary* cookie in cookies) {
    if (![cookie isKindOfClass:NSDictionary.class]) continue;
    NSString* name = [cookie[@"name"] isKindOfClass:NSString.class]
        ? cookie[@"name"]
        : @"";
    NSString* value = [cookie[@"value"] isKindOfClass:NSString.class]
        ? cookie[@"value"]
        : @"";
    if (name.length == 0) continue;
    [parts addObject:[NSString stringWithFormat:@"%@=%@", name, value]];
  }
  return [parts componentsJoinedByString:@"; "];
}

void StoreResponseCookiesInCEF(NSURL* url, NSHTTPURLResponse* response) {
  if (!url || !response) return;
  NSArray<NSHTTPCookie*>* cookies =
      [NSHTTPCookie cookiesWithResponseHeaderFields:response.allHeaderFields
                                            forURL:url];
  if (cookies.count == 0) return;

  CefRefPtr<CefCookieManager> manager =
      CefCookieManager::GetGlobalManager(nullptr);
  if (!manager) return;

  for (NSHTTPCookie* nsCookie in cookies) {
    if (nsCookie.name.length == 0) continue;
    CefCookie cookie;
    CefString(&cookie.name) = std::string(nsCookie.name.UTF8String);
    CefString(&cookie.value) = std::string((nsCookie.value ?: @"").UTF8String);
    if (nsCookie.domain.length > 0) {
      CefString(&cookie.domain) = std::string(nsCookie.domain.UTF8String);
    }
    CefString(&cookie.path) =
        std::string((nsCookie.path.length > 0 ? nsCookie.path : @"/").UTF8String);
    cookie.secure = nsCookie.isSecure ? 1 : 0;
    cookie.httponly = nsCookie.isHTTPOnly ? 1 : 0;
    if (nsCookie.expiresDate) {
      cookie.has_expires = 1;
      cookie.expires =
          CookieBaseTimeFromSeconds(nsCookie.expiresDate.timeIntervalSince1970);
    }
    manager->SetCookie(CefString(url.absoluteString.UTF8String), cookie, nullptr);
  }
}

void ResolveExtensionBridge(NSString* requestId,
                            NSString* extensionId,
                            id result,
                            NSString* error = nil) {
  NSMutableDictionary* response =
      [@{@"requestId" : requestId ?: @"",
         @"extensionId" : extensionId ?: @""} mutableCopy];
  if (error.length > 0) {
    response[@"error"] = error;
  } else {
    response[@"result"] = result ?: [NSNull null];
  }
  [MoriBrowserView dispatchExtensionBridgeResponse:response];
}

void DispatchCookieChanged(NSDictionary* cookie, BOOL removed, NSString* cause) {
  if (![cookie isKindOfClass:NSDictionary.class]) return;
  [MoriBrowserView dispatchExtensionEvent:@"cookies.onChanged"
                                       args:@[ @{@"removed" : @(removed),
                                                @"cookie" : cookie,
                                                @"cause" : cause ?: @"explicit"} ]
                             forExtensionID:nil];
}

class CookieVisitHandler : public CefCookieVisitor {
 public:
  CookieVisitHandler(NSString* request_id,
                     NSString* extension_id,
                     NSDictionary* filter,
                     BOOL first_only)
      : request_id_([request_id copy]),
        extension_id_([extension_id copy]),
        filter_([filter copy] ?: @{}),
        first_only_(first_only) {}

  bool Visit(const CefCookie& cookie,
             int count,
             int total,
             bool& deleteCookie) override {
    @autoreleasepool {
      NSDictionary* record = CookieDictionary(cookie);
      if (CookieMatches(record, filter_)) {
        [cookies_ addObject:record];
        if (first_only_) {
          SendIfNeeded();
          return false;
        }
      }
      if (count + 1 >= total) {
        SendIfNeeded();
      }
      return true;
    }
  }

  void SendIfNeeded() {
    if (sent_) return;
    sent_ = true;
    id result = first_only_ ? (cookies_.firstObject ?: (id)[NSNull null])
                            : (id)[cookies_ copy];
    ResolveExtensionBridge(request_id_, extension_id_, result);
  }

 private:
  NSString* request_id_;
  NSString* extension_id_;
  NSDictionary* filter_;
  NSMutableArray* cookies_ = [NSMutableArray array];
  BOOL first_only_;
  BOOL sent_ = NO;

  IMPLEMENT_REFCOUNTING(CookieVisitHandler);
};

class CookieSetCallback : public CefSetCookieCallback {
 public:
  CookieSetCallback(NSString* request_id,
                    NSString* extension_id,
                    NSDictionary* cookie)
      : request_id_([request_id copy]),
        extension_id_([extension_id copy]),
        cookie_([cookie copy] ?: @{}) {}

  void OnComplete(bool success) override {
    if (success) {
      ResolveExtensionBridge(request_id_, extension_id_, cookie_);
      DispatchCookieChanged(cookie_, NO, @"explicit");
    } else {
      ResolveExtensionBridge(request_id_, extension_id_, nil, @"Could not set cookie.");
    }
  }

 private:
  NSString* request_id_;
  NSString* extension_id_;
  NSDictionary* cookie_;

  IMPLEMENT_REFCOUNTING(CookieSetCallback);
};

class CookieDeleteCallback : public CefDeleteCookiesCallback {
 public:
  CookieDeleteCallback(NSString* request_id,
                       NSString* extension_id,
                       NSDictionary* details)
      : request_id_([request_id copy]),
        extension_id_([extension_id copy]),
        details_([details copy] ?: @{}) {}

  void OnComplete(int num_deleted) override {
    id result = num_deleted > 0 ? details_ : (id)[NSNull null];
    ResolveExtensionBridge(request_id_, extension_id_, result);
    if (num_deleted > 0) {
      DispatchCookieChanged(details_, YES, @"explicit");
    }
  }

 private:
  NSString* request_id_;
  NSString* extension_id_;
  NSDictionary* details_;

  IMPLEMENT_REFCOUNTING(CookieDeleteCallback);
};

void ScheduleCookieVisitFallback(CefRefPtr<CookieVisitHandler> visitor) {
  dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.2 * NSEC_PER_SEC)),
                 dispatch_get_main_queue(), ^{
                   visitor->SendIfNeeded();
                 });
}

NSDictionary* HandleExtensionCookies(NSString* method,
                                     NSDictionary* args,
                                     NSString* extensionId,
                                     NSString* requestId) {
  CefRefPtr<CefCookieManager> manager = CefCookieManager::GetGlobalManager(nullptr);
  if (!manager) return @{@"error" : @"Cookie manager is not available."};

  if ([method isEqualToString:@"cookies.getAll"] ||
      [method isEqualToString:@"cookies.get"]) {
    NSDictionary* details = [args[@"details"] isKindOfClass:NSDictionary.class]
        ? args[@"details"]
        : @{};
    BOOL firstOnly = [method isEqualToString:@"cookies.get"];
    CefRefPtr<CookieVisitHandler> visitor =
        new CookieVisitHandler(requestId, extensionId, details, firstOnly);
    NSString* url = [details[@"url"] isKindOfClass:NSString.class] ? details[@"url"] : nil;
    bool ok = url.length > 0
        ? manager->VisitUrlCookies(CefString(url.UTF8String), true, visitor)
        : manager->VisitAllCookies(visitor);
    if (!ok) return @{@"error" : @"Could not read cookies."};
    ScheduleCookieVisitFallback(visitor);
    return @{@"deferred" : @YES, @"result" : [NSNull null]};
  }

  if ([method isEqualToString:@"cookies.set"]) {
    NSDictionary* details = [args[@"details"] isKindOfClass:NSDictionary.class]
        ? args[@"details"]
        : @{};
    NSString* url = CookieURL(details);
    NSString* name = [details[@"name"] isKindOfClass:NSString.class]
        ? details[@"name"]
        : @"";
    NSString* value = [details[@"value"] isKindOfClass:NSString.class]
        ? details[@"value"]
        : @"";
    if (url.length == 0 || name.length == 0) {
      return @{@"error" : @"cookies.set requires url and name."};
    }

    CefCookie cookie;
    CefString(&cookie.name) = std::string(name.UTF8String);
    CefString(&cookie.value) = std::string(value.UTF8String);
    NSString* domain = [details[@"domain"] isKindOfClass:NSString.class]
        ? details[@"domain"]
        : @"";
    if (domain.length > 0) {
      CefString(&cookie.domain) = std::string(domain.UTF8String);
    }
    NSString* path = [details[@"path"] isKindOfClass:NSString.class]
        ? details[@"path"]
        : @"/";
    CefString(&cookie.path) = std::string(path.UTF8String);
    cookie.secure = [details[@"secure"] respondsToSelector:@selector(boolValue)] &&
                    [details[@"secure"] boolValue];
    cookie.httponly = [details[@"httpOnly"] respondsToSelector:@selector(boolValue)] &&
                      [details[@"httpOnly"] boolValue];
    if ([details[@"expirationDate"] respondsToSelector:@selector(doubleValue)]) {
      cookie.has_expires = 1;
      cookie.expires = CookieBaseTimeFromSeconds([details[@"expirationDate"] doubleValue]);
    }

    NSDictionary* cookieRecord = @{
      @"name" : name,
      @"value" : value,
      @"domain" : domain,
      @"hostOnly" : @(domain.length == 0 || ![domain hasPrefix:@"."]),
      @"path" : path.length > 0 ? path : @"/",
      @"secure" : @(cookie.secure != 0),
      @"httpOnly" : @(cookie.httponly != 0),
      @"session" : @(cookie.has_expires == 0),
      @"storeId" : @"0"
    };
    CefRefPtr<CookieSetCallback> callback =
        new CookieSetCallback(requestId, extensionId, cookieRecord);
    if (!manager->SetCookie(CefString(url.UTF8String), cookie, callback)) {
      return @{@"error" : @"Could not set cookie."};
    }
    return @{@"deferred" : @YES, @"result" : [NSNull null]};
  }

  if ([method isEqualToString:@"cookies.remove"]) {
    NSDictionary* details = [args[@"details"] isKindOfClass:NSDictionary.class]
        ? args[@"details"]
        : @{};
    NSString* url = [details[@"url"] isKindOfClass:NSString.class] ? details[@"url"] : @"";
    NSString* name = [details[@"name"] isKindOfClass:NSString.class] ? details[@"name"] : @"";
    if (url.length == 0 || name.length == 0) {
      return @{@"error" : @"cookies.remove requires url and name."};
    }
    NSDictionary* result = @{@"url" : url, @"name" : name, @"storeId" : @"0"};
    CefRefPtr<CookieDeleteCallback> callback =
        new CookieDeleteCallback(requestId, extensionId, result);
    if (!manager->DeleteCookies(CefString(url.UTF8String),
                                CefString(name.UTF8String), callback)) {
      return @{@"error" : @"Could not remove cookie."};
    }
    return @{@"deferred" : @YES, @"result" : [NSNull null]};
  }

  if ([method isEqualToString:@"cookies.getAllCookieStores"]) {
    ResolveExtensionBridge(requestId, extensionId, @[ @{@"id" : @"0", @"tabIds" : @[]} ]);
    return @{@"deferred" : @YES, @"result" : [NSNull null]};
  }

  return @{@"error" : [NSString stringWithFormat:@"Unsupported cookies method: %@", method]};
}

NSDictionary* HTTPHeadersDictionary(NSDictionary* rawHeaders) {
  NSMutableDictionary* headers = [NSMutableDictionary dictionary];
  if (![rawHeaders isKindOfClass:NSDictionary.class]) return headers;
  for (id key in rawHeaders) {
    id value = rawHeaders[key];
    if (![key isKindOfClass:NSString.class]) continue;
    if ([value isKindOfClass:NSString.class]) {
      headers[key] = value;
    } else if ([value respondsToSelector:@selector(stringValue)]) {
      headers[key] = [value stringValue];
    }
  }
  return headers;
}

class RuntimeFetchCookieVisitor : public CefCookieVisitor {
 public:
  explicit RuntimeFetchCookieVisitor(
      void (^completion)(NSArray<NSDictionary*>* cookies))
      : completion_([completion copy]) {}

  bool Visit(const CefCookie& cookie,
             int count,
             int total,
             bool& deleteCookie) override {
    @autoreleasepool {
      [cookies_ addObject:CookieDictionary(cookie)];
      if (count + 1 >= total) {
        SendIfNeeded();
      }
      return true;
    }
  }

  void SendIfNeeded() {
    if (sent_) return;
    sent_ = YES;
    void (^completion)(NSArray<NSDictionary*>*) = completion_;
    if (completion) {
      completion([cookies_ copy]);
    }
  }

 private:
  void (^completion_)(NSArray<NSDictionary*>*) = nil;
  NSMutableArray<NSDictionary*>* cookies_ = [NSMutableArray array];
  BOOL sent_ = NO;

  IMPLEMENT_REFCOUNTING(RuntimeFetchCookieVisitor);
};

void ScheduleRuntimeFetchCookieFallback(
    CefRefPtr<RuntimeFetchCookieVisitor> visitor) {
  dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.2 * NSEC_PER_SEC)),
                 dispatch_get_main_queue(), ^{
                   visitor->SendIfNeeded();
                 });
}

void StartRuntimeFetch(NSString* extensionId,
                       NSString* requestId,
                       NSString* rawURL,
                       NSMutableURLRequest* request) {
  NSURLSessionConfiguration* configuration =
      NSURLSessionConfiguration.ephemeralSessionConfiguration;
  configuration.HTTPShouldSetCookies = NO;
  configuration.HTTPCookieAcceptPolicy = NSHTTPCookieAcceptPolicyNever;
  __block NSURLSession* session =
      [NSURLSession sessionWithConfiguration:configuration];
  NSURLSessionDataTask* task =
      [session dataTaskWithRequest:request
                 completionHandler:^(NSData* data,
                                     NSURLResponse* urlResponse,
                                     NSError* error) {
        NSMutableDictionary* bridgeResponse = [@{
          @"requestId" : requestId ?: @"",
          @"extensionId" : extensionId ?: @""
        } mutableCopy];
        if (error) {
          bridgeResponse[@"error"] = error.localizedDescription ?: @"Fetch failed.";
        } else {
          NSHTTPURLResponse* http =
              [urlResponse isKindOfClass:NSHTTPURLResponse.class]
                  ? (NSHTTPURLResponse*)urlResponse
                  : nil;
          StoreResponseCookiesInCEF(urlResponse.URL ?: request.URL, http);
          NSMutableDictionary* responseHeaders = [NSMutableDictionary dictionary];
          for (id key in http.allHeaderFields) {
            id value = http.allHeaderFields[key];
            if ([key isKindOfClass:NSString.class]) {
              responseHeaders[key] =
                  [value isKindOfClass:NSString.class] ? value : [value description];
            }
          }
          bridgeResponse[@"result"] = @{
            @"url" : urlResponse.URL.absoluteString ?: rawURL,
            @"status" : http ? @(http.statusCode) : @200,
            @"statusText" : @"",
            @"headers" : responseHeaders,
            @"bodyBase64" : [(data ?: [NSData data])
                base64EncodedStringWithOptions:0]
          };
        }
        dispatch_async(dispatch_get_main_queue(), ^{
          [MoriBrowserView dispatchExtensionBridgeResponse:bridgeResponse];
        });
        [session finishTasksAndInvalidate];
      }];
  [task resume];
}

NSDictionary* HandleRuntimeFetch(NSString* extensionId,
                                 NSDictionary* args,
                                 NSString* requestId) {
  NSString* rawURL = [args[@"url"] isKindOfClass:NSString.class]
      ? args[@"url"]
      : @"";
  NSURL* url = [NSURL URLWithString:rawURL];
  NSString* scheme = url.scheme.lowercaseString ?: @"";
  if (!url || ![@[@"http", @"https"] containsObject:scheme]) {
    return @{@"error" : @"runtime.fetch only supports http(s) URLs."};
  }

  NSDictionary* ext = EnabledExtensionRecordForID(extensionId);
  NSDictionary* manifest = ManifestForExtension(ext);
  if (!ExtensionHostPermissionsAllow(manifest, url)) {
    return @{@"error" : @"Extension host permissions do not allow this URL."};
  }

  NSString* method = [args[@"method"] isKindOfClass:NSString.class]
      ? [(NSString*)args[@"method"] uppercaseString]
      : @"GET";
  if (method.length == 0) method = @"GET";

  NSMutableURLRequest* request = [NSMutableURLRequest requestWithURL:url];
  request.HTTPMethod = method;
  request.timeoutInterval = 30.0;
  request.cachePolicy = NSURLRequestReloadIgnoringLocalCacheData;
  NSString* credentials = [args[@"credentials"] isKindOfClass:NSString.class]
      ? args[@"credentials"]
      : @"same-origin";
  BOOL includeCookies =
      [credentials isEqualToString:@"include"] ||
      [credentials isEqualToString:@"same-origin"];
  request.HTTPShouldHandleCookies = NO;

  NSDictionary* headers = HTTPHeadersDictionary(args[@"headers"]);
  for (NSString* name in headers) {
    [request setValue:headers[name] forHTTPHeaderField:name];
  }

  NSString* body = [args[@"body"] isKindOfClass:NSString.class]
      ? args[@"body"]
      : nil;
  if (body) {
    request.HTTPBody = [body dataUsingEncoding:NSUTF8StringEncoding];
  }

  if (includeCookies) {
    CefRefPtr<CefCookieManager> manager =
        CefCookieManager::GetGlobalManager(nullptr);
    CefRefPtr<RuntimeFetchCookieVisitor> visitor =
        new RuntimeFetchCookieVisitor(^(NSArray<NSDictionary*>* cookies) {
          NSString* cookieHeader = CookieHeaderValue(cookies);
          if (cookieHeader.length > 0) {
            [request setValue:cookieHeader forHTTPHeaderField:@"Cookie"];
          }
          StartRuntimeFetch(extensionId, requestId, rawURL, request);
        });
    if (manager &&
        manager->VisitUrlCookies(CefString(rawURL.UTF8String), true, visitor)) {
      ScheduleRuntimeFetchCookieFallback(visitor);
      return @{@"deferred" : @YES, @"result" : [NSNull null]};
    }
  }

  StartRuntimeFetch(extensionId, requestId, rawURL, request);
  return @{@"deferred" : @YES, @"result" : [NSNull null]};
}

NSDictionary* HandleExtensionBridgeRequest(NSString* requestJSON,
                                           int sourceTabID) {
  NSData* data = [requestJSON dataUsingEncoding:NSUTF8StringEncoding];
  if (!data) return nil;
  id parsed = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
  if (![parsed isKindOfClass:[NSDictionary class]]) return nil;
  NSDictionary* request = (NSDictionary*)parsed;

  NSString* requestId = [request[@"requestId"] isKindOfClass:[NSString class]]
      ? request[@"requestId"]
      : @"";
  NSString* extensionId = [request[@"extensionId"] isKindOfClass:[NSString class]]
      ? request[@"extensionId"]
      : @"";
  NSString* method = [request[@"method"] isKindOfClass:[NSString class]]
      ? request[@"method"]
      : @"";
  NSDictionary* args = [request[@"args"] isKindOfClass:[NSDictionary class]]
      ? request[@"args"]
      : @{};

  NSMutableDictionary* response =
      [@{@"requestId" : requestId, @"extensionId" : extensionId} mutableCopy];
  if (extensionId.length == 0 || method.length == 0) {
    response[@"error"] = @"Malformed extension bridge request.";
    return response;
  }

  NSUserDefaults* defaults = [NSUserDefaults standardUserDefaults];
  NSString* storageArea = nil;
  NSString* storageOperation = nil;

	if (ParseStorageMethod(method, &storageArea, &storageOperation)) {
	  NSMutableDictionary* store = ExtensionStorage(extensionId, storageArea);
	  response[@"result"] = StorageGetResult(store, args[@"keys"]);
	  if ([storageOperation isEqualToString:@"getBytesInUse"]) {
	    response[@"result"] = StorageBytesInUse(store, args[@"keys"]);
	  } else if ([storageOperation isEqualToString:@"getKeys"]) {
	    response[@"result"] = [[store allKeys] sortedArrayUsingSelector:@selector(compare:)];
	  } else if ([storageOperation isEqualToString:@"setAccessLevel"]) {
	    response[@"result"] = @{};
	  } else if ([storageOperation isEqualToString:@"set"]) {
	    if ([storageArea isEqualToString:@"managed"]) {
	      response[@"error"] = @"storage.managed is read-only.";
	      [defaults synchronize];
	      return response;
	    }
	    NSDictionary* items = [args[@"items"] isKindOfClass:[NSDictionary class]]
	        ? args[@"items"]
	        : @{};
      NSMutableDictionary* changes = [NSMutableDictionary dictionary];
      for (id key in items) {
        if (![key isKindOfClass:[NSString class]]) continue;
        id newValue = items[key] ?: [NSNull null];
        NSMutableDictionary* change = [NSMutableDictionary dictionary];
        id oldValue = store[key];
        if (oldValue) change[@"oldValue"] = oldValue;
        if (newValue && newValue != [NSNull null]) change[@"newValue"] = newValue;
        changes[key] = change;
      }
      [store addEntriesFromDictionary:items];
      NSString* writeError =
          WriteExtensionStorage(extensionId, storageArea, store);
      if (writeError) {
        response[@"error"] = writeError;
        return response;
      }
      response[@"result"] = @{};
      if (changes.count > 0) {
        response[@"storageChange"] = changes;
        response[@"storageArea"] = storageArea;
      }
	  } else if ([storageOperation isEqualToString:@"remove"]) {
	    if ([storageArea isEqualToString:@"managed"]) {
	      response[@"error"] = @"storage.managed is read-only.";
	      [defaults synchronize];
	      return response;
	    }
	    id keys = args[@"keys"];
	    NSMutableDictionary* changes = [NSMutableDictionary dictionary];
      if ([keys isKindOfClass:[NSString class]]) {
        id oldValue = store[keys];
        if (oldValue) changes[keys] = @{@"oldValue" : oldValue};
        [store removeObjectForKey:keys];
      } else if ([keys isKindOfClass:[NSArray class]]) {
        for (id key in (NSArray*)keys) {
          if ([key isKindOfClass:[NSString class]]) {
            id oldValue = store[key];
            if (oldValue) changes[key] = @{@"oldValue" : oldValue};
            [store removeObjectForKey:key];
          }
        }
      }
      NSString* writeError =
          WriteExtensionStorage(extensionId, storageArea, store);
      if (writeError) {
        response[@"error"] = writeError;
        return response;
      }
      response[@"result"] = @{};
      if (changes.count > 0) {
        response[@"storageChange"] = changes;
        response[@"storageArea"] = storageArea;
      }
	  } else if ([storageOperation isEqualToString:@"clear"]) {
	    if ([storageArea isEqualToString:@"managed"]) {
	      response[@"error"] = @"storage.managed is read-only.";
	      [defaults synchronize];
	      return response;
	    }
	    NSMutableDictionary* changes = [NSMutableDictionary dictionary];
      for (id key in store) {
        if (![key isKindOfClass:[NSString class]]) continue;
        id oldValue = store[key];
        if (oldValue) changes[key] = @{@"oldValue" : oldValue};
      }
      [defaults removeObjectForKey:ExtensionStorageDefaultsKey(extensionId, storageArea)];
      response[@"result"] = @{};
      if (changes.count > 0) {
        response[@"storageChange"] = changes;
        response[@"storageArea"] = storageArea;
      }
    } else if (![storageOperation isEqualToString:@"get"]) {
      response[@"error"] =
          [NSString stringWithFormat:@"Unsupported storage method: %@", method];
    }
  } else if ([method isEqualToString:@"runtime.messageResponse"]) {
    NSString* targetRequestId = [args[@"requestId"] isKindOfClass:[NSString class]]
        ? args[@"requestId"]
        : @"";
    if (targetRequestId.length == 0) {
      response[@"error"] = @"Missing message response request id.";
    } else {
      [MoriBrowserView dispatchExtensionBridgeResponse:@{
        @"requestId" : targetRequestId,
        @"extensionId" : extensionId,
        @"result" : args[@"response"] ?: [NSNull null]
      }];
      response[@"result"] = @{};
    }
  } else if ([method isEqualToString:@"runtime.messageNoResponse"]) {
    NSString* targetRequestId = [args[@"requestId"] isKindOfClass:[NSString class]]
        ? args[@"requestId"]
        : @"";
    if (targetRequestId.length == 0) {
      response[@"error"] = @"Missing message response request id.";
    } else {
      // A context had no synchronous answer for this message. Resolve the
      // sender only after a grace period: an asynchronous responder (the
      // background worker reading storage, say) should win the race, and
      // dispatchExtensionBridgeResponse is a no-op once a real reply has already
      // settled the request. The noResponse flag makes the sender see undefined.
      NSString* targetExtensionId = extensionId;
      dispatch_after(
          dispatch_time(DISPATCH_TIME_NOW, (int64_t)(5.0 * NSEC_PER_SEC)),
          dispatch_get_main_queue(), ^{
            [MoriBrowserView dispatchExtensionBridgeResponse:@{
              @"requestId" : targetRequestId,
              @"extensionId" : targetExtensionId,
              @"result" : [NSNull null],
              @"noResponse" : @YES
            }];
          });
      response[@"result"] = @{};
    }
  } else if ([method isEqualToString:@"runtime.sendNativeMessage"]) {
    NSDictionary* nativeResponse = HandleNativeMessagingSend(extensionId, args);
    NSString* error = [nativeResponse[@"error"] isKindOfClass:[NSString class]]
        ? nativeResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = nativeResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method isEqualToString:@"runtime.connectNative"]) {
    NSDictionary* nativeResponse = StartNativeMessagingPort(extensionId, args);
    NSString* error = [nativeResponse[@"error"] isKindOfClass:[NSString class]]
        ? nativeResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = nativeResponse[@"result"] ?: @{};
    }
  } else if ([method isEqualToString:@"runtime.nativePortMessage"]) {
    NSDictionary* nativeResponse = NativeMessagingPortPostMessage(extensionId, args);
    NSString* error = [nativeResponse[@"error"] isKindOfClass:[NSString class]]
        ? nativeResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = nativeResponse[@"result"] ?: @{};
    }
  } else if ([method isEqualToString:@"runtime.nativePortDisconnect"]) {
    NSDictionary* nativeResponse = DisconnectNativeMessagingPort(extensionId, args);
    NSString* error = [nativeResponse[@"error"] isKindOfClass:[NSString class]]
        ? nativeResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = nativeResponse[@"result"] ?: @{};
    }
  } else if ([method isEqualToString:@"runtime.fetch"]) {
    NSDictionary* fetchResponse = HandleRuntimeFetch(extensionId, args, requestId);
    NSString* error = [fetchResponse[@"error"] isKindOfClass:[NSString class]]
        ? fetchResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      if ([fetchResponse[@"deferred"] boolValue]) {
        response[@"deferred"] = @YES;
      }
      response[@"result"] = fetchResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method isEqualToString:@"runtime.sendMessage"]) {
    NSString* targetExtensionId =
        [args[@"targetExtensionId"] isKindOfClass:NSString.class]
            ? args[@"targetExtensionId"]
            : extensionId;
    NSString* sourceURL = [args[@"sourceUrl"] isKindOfClass:NSString.class]
        ? args[@"sourceUrl"]
        : nil;
    NSString* sourceOrigin =
        [args[@"sourceOrigin"] isKindOfClass:NSString.class]
            ? args[@"sourceOrigin"]
            : nil;
    id message = args[@"message"] ?: [NSNull null];
    BOOL selfTarget =
        [targetExtensionId caseInsensitiveCompare:extensionId] == NSOrderedSame;
    BOOL allowsExternal =
        ExtensionAllowsExternalConnect(targetExtensionId, sourceURL);
    BOOL selfExternalAccountMessage =
        selfTarget && allowsExternal &&
        ExtensionMessageIsExternallyConnectableAccountType(message);
    BOOL external = (!selfTarget && allowsExternal) || selfExternalAccountMessage;
    [MoriBrowserView dispatchExtensionMessage:message
                                 forExtensionID:targetExtensionId
                                      requestID:requestId
                                      sourceURL:sourceURL
                                   sourceOrigin:sourceOrigin
                                    sourceTabID:sourceTabID
                                       external:external];
    response[@"deferred"] = @YES;
    response[@"result"] = [NSNull null];
  } else if ([method isEqualToString:@"runtime.connect"]) {
    NSString* targetExtensionId =
        [args[@"targetExtensionId"] isKindOfClass:NSString.class]
            ? args[@"targetExtensionId"]
            : extensionId;
    NSString* portID = [args[@"portId"] isKindOfClass:NSString.class]
        ? args[@"portId"]
        : @"";
    NSString* name = [args[@"name"] isKindOfClass:NSString.class]
        ? args[@"name"]
        : @"";
    NSString* sourceURL = [args[@"sourceUrl"] isKindOfClass:NSString.class]
        ? args[@"sourceUrl"]
        : @"";
    NSString* sourceOrigin =
        [args[@"sourceOrigin"] isKindOfClass:NSString.class]
            ? args[@"sourceOrigin"]
            : @"";
    BOOL selfTarget =
        [targetExtensionId caseInsensitiveCompare:extensionId] == NSOrderedSame;
    BOOL external =
        !selfTarget && ExtensionAllowsExternalConnect(targetExtensionId, sourceURL);
    NSMutableDictionary* sender =
        external ? [NSMutableDictionary dictionary] : [@{@"id" : extensionId} mutableCopy];
    if (sourceURL.length > 0) sender[@"url"] = sourceURL;
    if (sourceOrigin.length > 0) {
      sender[@"origin"] = sourceOrigin;
    }
    NSDictionary* sourceTab = ExtensionSenderTab(sourceTabID, sourceURL);
    if (sourceTab) {
      sender[@"tab"] = sourceTab;
    }
    if ([args[@"frameId"] respondsToSelector:@selector(integerValue)]) {
      sender[@"frameId"] = @([args[@"frameId"] integerValue]);
    }
    BroadcastExtensionPortConnect(targetExtensionId, portID, name, sender,
                                  sourceURL, external);
    response[@"result"] = @{};
  } else if ([method isEqualToString:@"runtime.portMessage"]) {
    NSString* portID = [args[@"portId"] isKindOfClass:NSString.class]
        ? args[@"portId"]
        : @"";
    NSString* sourceURL = [args[@"sourceUrl"] isKindOfClass:NSString.class]
        ? args[@"sourceUrl"]
        : @"";
    BroadcastExtensionPortMessage(extensionId, portID,
                                  args[@"message"] ?: [NSNull null],
                                  sourceURL);
    response[@"result"] = @{};
  } else if ([method isEqualToString:@"runtime.portDisconnect"]) {
    NSString* portID = [args[@"portId"] isKindOfClass:NSString.class]
        ? args[@"portId"]
        : @"";
    NSString* sourceURL = [args[@"sourceUrl"] isKindOfClass:NSString.class]
        ? args[@"sourceUrl"]
        : @"";
    BroadcastExtensionPortDisconnect(extensionId, portID, sourceURL);
    response[@"result"] = @{};
  } else if ([method isEqualToString:@"runtime.openOptionsPage"] ||
             [method isEqualToString:@"runtime.getContexts"] ||
             [method isEqualToString:@"runtime.setUninstallURL"] ||
             [method isEqualToString:@"identity.launchWebAuthFlow"] ||
             [method hasPrefix:@"sidePanel."]) {
    NSMutableDictionary* runtimeArgs = [args mutableCopy];
    runtimeArgs[@"extensionId"] = extensionId;
    runtimeArgs[@"requestId"] = requestId;
    NSDictionary* runtimeResponse =
        [MoriRoot handleExtensionRuntime:method args:runtimeArgs];
    NSString* error = [runtimeResponse[@"error"] isKindOfClass:[NSString class]]
        ? runtimeResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else if ([runtimeResponse[@"deferred"] respondsToSelector:@selector(boolValue)] &&
               [runtimeResponse[@"deferred"] boolValue]) {
      response[@"deferred"] = @YES;
      response[@"result"] = runtimeResponse[@"result"] ?: [NSNull null];
    } else {
      response[@"result"] = runtimeResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"bookmarks."]) {
    NSDictionary* bookmarkResponse =
        [MoriRoot handleExtensionBookmarks:method args:args];
    NSString* error = [bookmarkResponse[@"error"] isKindOfClass:[NSString class]]
        ? bookmarkResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = bookmarkResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"history."] ||
             [method hasPrefix:@"topSites."]) {
    NSDictionary* historyResponse =
        [MoriRoot handleExtensionHistory:method args:args];
    NSString* error = [historyResponse[@"error"] isKindOfClass:[NSString class]]
        ? historyResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = historyResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"browsingData."]) {
    NSDictionary* browsingDataResponse =
        [MoriRoot handleExtensionBrowsingData:method args:args];
    NSString* error = [browsingDataResponse[@"error"] isKindOfClass:[NSString class]]
        ? browsingDataResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = browsingDataResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"sessions."]) {
    NSDictionary* sessionsResponse =
        [MoriRoot handleExtensionSessions:method args:args];
    NSString* error = [sessionsResponse[@"error"] isKindOfClass:[NSString class]]
        ? sessionsResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = sessionsResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"contextMenus."]) {
    NSDictionary* menuResponse = HandleContextMenus(method, args, extensionId);
    NSString* error = [menuResponse[@"error"] isKindOfClass:[NSString class]]
        ? menuResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = menuResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"management."]) {
    NSMutableDictionary* managementArgs = [args mutableCopy];
    managementArgs[@"extensionId"] = extensionId;
    NSDictionary* managementResponse =
        [MoriRoot handleExtensionManagement:method args:managementArgs];
    NSString* error = [managementResponse[@"error"] isKindOfClass:[NSString class]]
        ? managementResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = managementResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"notifications."]) {
    NSDictionary* notificationsResponse =
        HandleNotifications(method, args, extensionId);
    NSString* error = [notificationsResponse[@"error"] isKindOfClass:[NSString class]]
        ? notificationsResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = notificationsResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"clipboard."]) {
    NSDictionary* clipboardResponse = HandleClipboard(method, args, extensionId);
    NSString* error = [clipboardResponse[@"error"] isKindOfClass:[NSString class]]
        ? clipboardResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = clipboardResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"permissions."]) {
    NSDictionary* permissionsResponse = HandlePermissions(method, args, extensionId);
    NSString* error = [permissionsResponse[@"error"] isKindOfClass:[NSString class]]
        ? permissionsResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = permissionsResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"proxy.settings."]) {
    NSDictionary* proxyResponse = HandleProxySettings(method, args, extensionId);
    NSString* error = [proxyResponse[@"error"] isKindOfClass:[NSString class]]
        ? proxyResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = proxyResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"declarativeNetRequest."]) {
    NSDictionary* dnrResponse =
        HandleDeclarativeNetRequest(method, args, extensionId);
    NSString* error = [dnrResponse[@"error"] isKindOfClass:[NSString class]]
        ? dnrResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = dnrResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"action."]) {
    NSMutableDictionary* actionArgs = [args mutableCopy];
    actionArgs[@"extensionId"] = extensionId;
    NSDictionary* actionResponse =
        [MoriRoot handleExtensionAction:method args:actionArgs];
    NSString* error = [actionResponse[@"error"] isKindOfClass:[NSString class]]
        ? actionResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = actionResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method isEqualToString:@"tabs.connect"]) {
    NSString* portID = [args[@"portId"] isKindOfClass:NSString.class]
        ? args[@"portId"]
        : @"";
    NSString* name = [args[@"name"] isKindOfClass:NSString.class]
        ? args[@"name"]
        : @"";
    NSString* sourceURL = [args[@"sourceUrl"] isKindOfClass:NSString.class]
        ? args[@"sourceUrl"]
        : @"";
    NSMutableDictionary* sender = [@{@"id" : extensionId} mutableCopy];
    if (sourceURL.length > 0) sender[@"url"] = sourceURL;
    NSString* sourceOrigin =
        [args[@"sourceOrigin"] isKindOfClass:NSString.class]
            ? args[@"sourceOrigin"]
            : @"";
    if (sourceOrigin.length > 0) {
      sender[@"origin"] = sourceOrigin;
    }
    if ([args[@"frameId"] respondsToSelector:@selector(integerValue)]) {
      sender[@"frameId"] = @([args[@"frameId"] integerValue]);
    }
    if ([args[@"tabId"] respondsToSelector:@selector(integerValue)]) {
      sender[@"tab"] = @{@"id" : @([args[@"tabId"] integerValue])};
    }
    BroadcastExtensionPortConnect(extensionId, portID, name, sender, sourceURL, NO);
    response[@"result"] = @{};
  } else if ([method hasPrefix:@"tabs."]) {
    NSMutableDictionary* tabArgs = [args mutableCopy];
    tabArgs[@"extensionId"] = extensionId;
    if ([method isEqualToString:@"tabs.sendMessage"]) {
      tabArgs[@"messageRequestId"] = requestId;
    }
    if ([method isEqualToString:@"tabs.captureVisibleTab"]) {
      tabArgs[@"requestId"] = requestId;
    }
    NSDictionary* tabResponse = [MoriRoot handleExtensionTabs:method args:tabArgs];
    NSString* error = [tabResponse[@"error"] isKindOfClass:[NSString class]]
        ? tabResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      if ([method isEqualToString:@"tabs.sendMessage"] ||
          [tabResponse[@"deferred"] boolValue]) {
        response[@"deferred"] = @YES;
      }
      response[@"result"] = tabResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"windows."]) {
    NSDictionary* windowResponse =
        [MoriRoot handleExtensionWindows:method args:args];
    NSString* error = [windowResponse[@"error"] isKindOfClass:[NSString class]]
        ? windowResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      response[@"result"] = windowResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"cookies."]) {
    NSDictionary* cookieResponse =
        HandleExtensionCookies(method, args, extensionId, requestId);
    NSString* error = [cookieResponse[@"error"] isKindOfClass:[NSString class]]
        ? cookieResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      if ([cookieResponse[@"deferred"] boolValue]) {
        response[@"deferred"] = @YES;
      }
      response[@"result"] = cookieResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"downloads."]) {
    if ([method isEqualToString:@"downloads.cancel"]) {
      uint32_t downloadID = [args[@"downloadId"] respondsToSelector:@selector(unsignedIntValue)]
          ? [args[@"downloadId"] unsignedIntValue]
          : 0;
      NSDictionary* cancelResponse = CancelDownload(downloadID);
      NSString* error = [cancelResponse[@"error"] isKindOfClass:[NSString class]]
          ? cancelResponse[@"error"]
          : nil;
      if (error.length > 0) {
        response[@"error"] = error;
      } else {
        response[@"result"] = cancelResponse[@"result"] ?: [NSNull null];
      }
      return response;
    }
    NSMutableDictionary* downloadArgs = [args mutableCopy];
    downloadArgs[@"extensionId"] = extensionId;
    downloadArgs[@"requestId"] = requestId;
    NSDictionary* downloadResponse =
        [MoriRoot handleExtensionDownloads:method args:downloadArgs];
    NSString* error = [downloadResponse[@"error"] isKindOfClass:[NSString class]]
        ? downloadResponse[@"error"]
        : nil;
    if (error.length > 0) {
      response[@"error"] = error;
    } else {
      if ([downloadResponse[@"deferred"] boolValue]) {
        response[@"deferred"] = @YES;
      }
      response[@"result"] = downloadResponse[@"result"] ?: [NSNull null];
    }
  } else if ([method hasPrefix:@"scripting."]) {
    NSDictionary* ext = EnabledExtensionRecordForID(extensionId);
    if (!ext) {
      response[@"error"] = @"Extension is not enabled.";
    } else if ([method isEqualToString:@"scripting.registerContentScripts"] ||
               [method isEqualToString:@"scripting.getRegisteredContentScripts"] ||
               [method isEqualToString:@"scripting.updateContentScripts"] ||
               [method isEqualToString:@"scripting.unregisterContentScripts"]) {
      NSDictionary* scriptingResponse =
          HandleRegisteredContentScripts(method, args, extensionId);
      NSString* error = [scriptingResponse[@"error"] isKindOfClass:[NSString class]]
          ? scriptingResponse[@"error"]
          : nil;
      if (error.length > 0) {
        response[@"error"] = error;
      } else {
        response[@"result"] = scriptingResponse[@"result"] ?: [NSNull null];
      }
    } else {
	      NSDictionary* payload =
	          BuildScriptingBridgePayload(method, args, ext, requestId, extensionId);
      NSString* payloadError = [payload[@"error"] isKindOfClass:[NSString class]]
          ? payload[@"error"]
          : nil;
      if (payloadError.length > 0) {
        response[@"error"] = payloadError;
      } else {
        NSDictionary* scriptingResponse =
            [MoriRoot handleExtensionScripting:method args:payload];
        NSString* error = [scriptingResponse[@"error"] isKindOfClass:[NSString class]]
            ? scriptingResponse[@"error"]
            : nil;
        if (error.length > 0) {
          response[@"error"] = error;
	        } else {
	          if ([scriptingResponse[@"deferred"] boolValue]) {
	            response[@"deferred"] = @YES;
	          }
	          response[@"result"] = scriptingResponse[@"result"] ?: [NSNull null];
	        }
      }
    }
  } else {
    response[@"error"] =
        [NSString stringWithFormat:@"Unsupported extension method: %@", method];
  }

  [defaults synchronize];
  return response;
}

}  // namespace

// Exported so the extension scheme handler (CefAppImpl.mm) can inject the full
// chrome.* runtime into an extension page at serve time — i.e. before the
// page's own bundled scripts (webextension-polyfill, popup.js, …) run, instead
// of racing them via an async OnLoadStart IPC. Returns nil when the id doesn't
// resolve to an enabled extension. Safe to also run again from OnLoadStart: the
// shim guards every definition with `||`, and the background boot block is
// fired once via window.__moriBackgroundBooted.
NSString* MoriExtensionPageRuntimeJS(NSString* extensionID) {
  NSDictionary* ext = EnabledExtensionRecordForID(extensionID);
  if (!ext) return nil;
  NSDictionary* manifest = ManifestForExtension(ext);
  NSMutableString* js = [NSMutableString stringWithString:@"(function(){try{"];
  [js appendString:ExtensionRuntimeShim(ext, manifest)];
  [js appendString:@"}catch(e){console.error('[Mori extension runtime]',e);}})();"];
  return js;
}

void BrowserClient::OnBeforeContextMenu(CefRefPtr<CefBrowser> browser,
                                        CefRefPtr<CefFrame> frame,
                                        CefRefPtr<CefContextMenuParams> params,
                                        CefRefPtr<CefMenuModel> model) {
  CEF_REQUIRE_UI_THREAD();
  @autoreleasepool {
    NSArray<NSDictionary*>* items = MatchingContextMenuItems(params);
    NSMutableDictionary<NSNumber*, NSDictionary*>* registry =
        ContextMenuCommandRegistry();
    @synchronized(registry) {
      [registry removeAllObjects];
    }
    if (items.count == 0 || !model) return;

    if (model->GetCount() > 0) {
      model->AddSeparator();
    }

    static int nextCommand = MENU_ID_USER_FIRST;
    for (NSDictionary* item in items) {
      NSString* title = ContextMenuDisplayTitle(item, params);
      if (title.length == 0) continue;

      int commandID = nextCommand++;
      if (nextCommand > MENU_ID_USER_LAST) nextCommand = MENU_ID_USER_FIRST;

      NSString* type = [item[@"type"] isKindOfClass:NSString.class]
          ? ((NSString*)item[@"type"]).lowercaseString
          : @"normal";
      if ([type isEqualToString:@"separator"]) {
        model->AddSeparator();
        continue;
      }
      if ([type isEqualToString:@"checkbox"]) {
        model->AddCheckItem(commandID, CefString(title.UTF8String));
        model->SetChecked(commandID, [item[@"checked"] boolValue]);
      } else if ([type isEqualToString:@"radio"]) {
        model->AddRadioItem(commandID, CefString(title.UTF8String), 1);
        model->SetChecked(commandID, [item[@"checked"] boolValue]);
      } else {
        model->AddItem(commandID, CefString(title.UTF8String));
      }
      if ([item[@"enabled"] respondsToSelector:@selector(boolValue)]) {
        model->SetEnabled(commandID, [item[@"enabled"] boolValue]);
      }
      @synchronized(registry) {
        registry[@(commandID)] = item;
      }
    }
  }
}

bool BrowserClient::OnContextMenuCommand(CefRefPtr<CefBrowser> browser,
                                         CefRefPtr<CefFrame> frame,
                                         CefRefPtr<CefContextMenuParams> params,
                                         int command_id,
                                         EventFlags event_flags) {
  CEF_REQUIRE_UI_THREAD();
  if (command_id < MENU_ID_USER_FIRST || command_id > MENU_ID_USER_LAST) {
    return false;
  }
  @autoreleasepool {
    NSDictionary* item = nil;
    NSMutableDictionary<NSNumber*, NSDictionary*>* registry =
        ContextMenuCommandRegistry();
    @synchronized(registry) {
      item = registry[@(command_id)];
    }
    if (![item isKindOfClass:NSDictionary.class]) return false;
    NSString* extensionID = [item[@"extensionId"] isKindOfClass:NSString.class]
        ? item[@"extensionId"]
        : @"";
    if (extensionID.length == 0) return false;

    [MoriBrowserView dispatchExtensionEvent:@"contextMenus.onClicked"
                                         args:@[ ContextMenuClickInfo(params, item) ]
                               forExtensionID:extensionID];
    return true;
  }
}

CefRefPtr<CefResourceRequestHandler>
BrowserClient::GetResourceRequestHandler(CefRefPtr<CefBrowser> browser,
                                         CefRefPtr<CefFrame> frame,
                                         CefRefPtr<CefRequest> request,
                                         bool is_navigation,
                                         bool is_download,
                                         const CefString& request_initiator,
                                         bool& disable_default_handling) {
  @autoreleasepool {
    RememberExtensionRequestInitiator(request, request_initiator);
  }
  return this;
}

CefRefPtr<CefResourceHandler>
BrowserClient::GetResourceHandler(CefRefPtr<CefBrowser> browser,
                                  CefRefPtr<CefFrame> frame,
                                  CefRefPtr<CefRequest> request) {
  CEF_REQUIRE_IO_THREAD();
  @autoreleasepool {
    CefRefPtr<CefResourceHandler> handler =
        ExtensionPreflightResponse(frame, request);
    if (handler) ForgetExtensionRequestInitiator(request);
    return handler;
  }
}

bool BrowserClient::GetAuthCredentials(CefRefPtr<CefBrowser> browser,
                                       const CefString& origin_url,
                                       bool isProxy,
                                       const CefString& host,
                                       int port,
                                       const CefString& realm,
                                       const CefString& scheme,
                                       CefRefPtr<CefAuthCallback> callback) {
  CEF_REQUIRE_IO_THREAD();
  (void)browser;
  (void)callback;
  @autoreleasepool {
    int tabID = extension_tab_id_.load();
    NSDictionary* details =
        WebAuthRequestDetails(origin_url, isProxy, host, port, realm, scheme,
                              tabID);
    DispatchWebRequestEvent(@"webRequest.onAuthRequired", details);
  }
  return false;
}

BrowserClient::ReturnValue BrowserClient::OnBeforeResourceLoad(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    CefRefPtr<CefRequest> request,
    CefRefPtr<CefCallback> callback) {
  CEF_REQUIRE_IO_THREAD();
  @autoreleasepool {
    int tabID = extension_tab_id_.load();
    NSDictionary* details = WebRequestDetails(frame, request, tabID);
    DispatchWebRequestEvent(@"webRequest.onBeforeRequest", details);
    if (MoriAdBlockerShouldBlockRequest(request)) {
      DispatchWebRequestEvent(@"webRequest.onErrorOccurred", details,
                              @"net::ERR_BLOCKED_BY_CLIENT");
      return RV_CANCEL;
    }
    NSDictionary* dnrDecision = DeclarativeNetRequestDecision(request);
    NSString* dnrType = [dnrDecision[@"type"] isKindOfClass:NSString.class]
        ? dnrDecision[@"type"]
        : @"none";
    if ([dnrType isEqualToString:@"block"]) {
      DispatchWebRequestEvent(@"webRequest.onErrorOccurred", details,
                              @"net::ERR_BLOCKED_BY_CLIENT");
      return RV_CANCEL;
    }
    if ([dnrType isEqualToString:@"redirect"]) {
      NSString* redirectURL =
          [dnrDecision[@"redirectUrl"] isKindOfClass:NSString.class]
              ? dnrDecision[@"redirectUrl"]
              : nil;
      if (redirectURL.length > 0) {
        NSMutableDictionary* redirectDetails = [details mutableCopy];
        redirectDetails[@"redirectUrl"] = redirectURL;
        DispatchWebRequestEvent(@"webRequest.onBeforeRedirect", redirectDetails);
        request->SetURL(CefString(redirectURL.UTF8String));
      }
    }
    ApplyDNRRequestHeaderModifications(request);
    NSMutableDictionary* headerDetails = [details mutableCopy];
    headerDetails[@"requestHeaders"] = RequestHeaders(request);
    DispatchWebRequestEvent(@"webRequest.onBeforeSendHeaders", headerDetails);
    return RV_CONTINUE;
  }
}

bool BrowserClient::OnResourceResponse(CefRefPtr<CefBrowser> browser,
                                       CefRefPtr<CefFrame> frame,
                                       CefRefPtr<CefRequest> request,
                                       CefRefPtr<CefResponse> response) {
  CEF_REQUIRE_IO_THREAD();
  @autoreleasepool {
    ApplyExtensionCORSHeaders(frame, request, response);
    int tabID = extension_tab_id_.load();
    NSMutableDictionary* details =
        [WebRequestDetails(frame, request, tabID) mutableCopy];
    NSNumber* statusCode = response ? @(response->GetStatus()) : nil;
    if (statusCode) details[@"statusCode"] = statusCode;
    if (response) {
      NSString* statusText = @(response->GetStatusText().ToString().c_str());
      details[@"statusLine"] =
          [NSString stringWithFormat:@"HTTP %ld %@",
                                     (long)response->GetStatus(),
                                     statusText ?: @""];
    }
    details[@"responseHeaders"] = ResponseHeaders(response);
    DispatchWebRequestEvent(@"webRequest.onHeadersReceived", details);
  }
  return false;
}

void BrowserClient::OnResourceLoadComplete(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    CefRefPtr<CefRequest> request,
    CefRefPtr<CefResponse> response,
    URLRequestStatus status,
    int64_t received_content_length) {
  CEF_REQUIRE_IO_THREAD();
  @autoreleasepool {
    int tabID = extension_tab_id_.load();
    NSDictionary* details = WebRequestDetails(frame, request, tabID);
    NSNumber* statusCode = response ? @(response->GetStatus()) : nil;
    ForgetExtensionRequestInitiator(request);
    if (status == UR_SUCCESS) {
      DispatchWebRequestEvent(@"webRequest.onCompleted", details, nil,
                              statusCode);
      return;
    }
    NSString* error = status == UR_CANCELED ? @"net::ERR_ABORTED"
                                            : @"net::ERR_FAILED";
    DispatchWebRequestEvent(@"webRequest.onErrorOccurred", details, error,
                            statusCode);
  }
}

void BrowserClient::OnLoadStart(CefRefPtr<CefBrowser> browser,
                                CefRefPtr<CefFrame> frame,
                                TransitionType transition_type) {
  CEF_REQUIRE_UI_THREAD();
  if (!frame) {
    return;
  }
  if (delegate_ && frame->IsMain()) {
    delegate_->OnLoadStart(frame->GetURL().ToString());
  }
  // Install the passkey shim before the page's own scripts run, so our
  // navigator.credentials override is the one relying parties see.
  frame->ExecuteJavaScript(kMoriPasskeyAgent, frame->GetURL(), 0);
  frame->ExecuteJavaScript(kMoriWebNavigationAgent, frame->GetURL(), 0);
  InjectExtensionPageRuntime(frame);
  InjectExtensionContentScripts(frame, @"document_start");
}

void BrowserClient::OnLoadEnd(CefRefPtr<CefBrowser> browser,
                              CefRefPtr<CefFrame> frame,
                              int httpStatusCode) {
  CEF_REQUIRE_UI_THREAD();
  if (!frame) {
    return;
  }
  if (delegate_ && frame->IsMain()) {
    delegate_->OnLoadEnd(frame->GetURL().ToString(), httpStatusCode);
  }
  // Seed the auto-PiP flag, then install the media agent in this frame.
  std::string js =
      std::string("window.__moriAutoPiP=") +
      (MoriAutoPiPEnabled() ? "true" : "false") + ";" + kMoriMediaAgent;
  frame->ExecuteJavaScript(js, frame->GetURL(), 0);
  InjectExtensionContentScripts(frame, @"document_end");
  CefRefPtr<CefFrame> idleFrame = frame;
  dispatch_after(dispatch_time(DISPATCH_TIME_NOW,
                               (int64_t)(0.2 * NSEC_PER_SEC)),
                 dispatch_get_main_queue(), ^{
    if (idleFrame && idleFrame->IsValid()) {
      InjectExtensionContentScripts(idleFrame, @"document_idle");
    }
  });
}

bool BrowserClient::OnConsoleMessage(CefRefPtr<CefBrowser> browser,
                                     cef_log_severity_t level,
                                     const CefString& message,
                                     const CefString& source,
                                     int line) {
  const std::string msg = message.ToString();

  static const std::string kExtensionSmokePrefix =
      "__MORI_EXTENSION_SMOKE__";
  if (msg.rfind(kExtensionSmokePrefix, 0) == 0) {
    NSString* payload = [NSString stringWithUTF8String:msg.c_str()] ?: @"";
    NSString* resultPath =
        NSProcessInfo.processInfo.environment[@"MORI_EXTENSION_SMOKE_RESULT_PATH"];
    if (resultPath.length > 0) {
      [payload writeToFile:resultPath
                atomically:YES
                  encoding:NSUTF8StringEncoding
                     error:nil];
    }
    NSLog(@"%{public}s", payload.UTF8String);
    return true;
  }

  // WebAuthn/passkey channel: run the native ceremony, then resolve in-page.
  static const std::string kWebAuthnPrefix = "__MORI_WEBAUTHN__";
  if (msg.rfind(kWebAuthnPrefix, 0) == 0) {
    const std::string request = msg.substr(kWebAuthnPrefix.size());
    NSString* requestJSON = [NSString stringWithUTF8String:request.c_str()];
    if (requestJSON && browser) {
      // Completion is delivered on the main thread == CEF UI thread, so it is
      // safe to call back into the browser directly. OnConsoleMessage doesn't
      // tell us which frame sent the request, so we resolve in every frame; the
      // global resolver no-ops wherever the request id isn't pending.
      CefRefPtr<CefBrowser> b = browser;
      [MoriPasskeys handle:requestJSON
                  completion:^(NSString* response) {
                    NSString* js = [NSString
                        stringWithFormat:@"if(window.__moriWAResolve)"
                                          "window.__moriWAResolve(%@);",
                                         JSStringLiteral(response)];
                    CefString code(js.UTF8String);
                    std::vector<CefString> ids;
                    b->GetFrameIdentifiers(ids);
                    for (const auto& id : ids) {
                      CefRefPtr<CefFrame> f = b->GetFrameByIdentifier(id);
                      if (f) {
                        f->ExecuteJavaScript(code, f->GetURL(), 0);
                      }
                    }
                  }];
    }
    return true;  // Swallow our channel.
  }

	  static const std::string kExtensionPrefix = "__MORI_EXTENSION__";
  if (msg.rfind(kExtensionPrefix, 0) == 0) {
    const std::string request = msg.substr(kExtensionPrefix.size());
    NSString* requestJSON = [NSString stringWithUTF8String:request.c_str()];
    NSDictionary* response =
        HandleExtensionBridgeRequest(requestJSON, extension_tab_id_.load());
    if (response && browser) {
      NSString* js = [NSString
          stringWithFormat:@"if(window.__moriExtResolve)"
                            "window.__moriExtResolve(%@);",
                           JSONStringLiteral(response)];
      CefString code(js.UTF8String);
      std::vector<CefString> ids;
      browser->GetFrameIdentifiers(ids);
      for (const auto& id : ids) {
        CefRefPtr<CefFrame> f = browser->GetFrameByIdentifier(id);
        if (f) {
          f->ExecuteJavaScript(code, f->GetURL(), 0);
        }
      }
    }
    return true;
  }

  static const std::string kScriptingResultPrefix =
      "__MORI_SCRIPTING_RESULT__";
  if (msg.rfind(kScriptingResultPrefix, 0) == 0) {
    const std::string responseJSON = msg.substr(kScriptingResultPrefix.size());
    NSString* raw = [NSString stringWithUTF8String:responseJSON.c_str()];
    NSData* data = [raw dataUsingEncoding:NSUTF8StringEncoding];
    id parsed = data ? [NSJSONSerialization JSONObjectWithData:data
                                                       options:0
                                                         error:nil]
                     : nil;
    if ([parsed isKindOfClass:[NSDictionary class]]) {
      NSDictionary* response = (NSDictionary*)parsed;
      NSString* requestId =
          [response[@"requestId"] isKindOfClass:[NSString class]]
              ? response[@"requestId"]
              : @"";
      NSString* extensionId =
          [response[@"extensionId"] isKindOfClass:[NSString class]]
              ? response[@"extensionId"]
              : @"";
      NSString* error = [response[@"error"] isKindOfClass:[NSString class]]
          ? response[@"error"]
          : nil;
      id result = response[@"result"] ?: [NSNull null];
      ResolveExtensionBridge(requestId, extensionId, result, error);
    }
    return true;
  }

  static const std::string kWebNavigationPrefix = "__MORI_WEBNAV__";
  if (msg.rfind(kWebNavigationPrefix, 0) == 0) {
    const std::string payload = msg.substr(kWebNavigationPrefix.size());
    NSString* payloadJSON = [NSString stringWithUTF8String:payload.c_str()];
    DispatchWebNavigationConsoleEvent(payloadJSON, extension_tab_id_.load());
    return true;
  }

  static const std::string kPrefix = "__MORI_MEDIA__";
  if (msg.rfind(kPrefix, 0) == 0) {
    const std::string json = msg.substr(kPrefix.size());
    const int browser_id = browser ? browser->GetIdentifier() : 0;
    NSString* j = [NSString stringWithUTF8String:json.c_str()];
    [NSNotificationCenter.defaultCenter
        postNotificationName:kMoriMediaUpdated
                      object:nil
                    userInfo:@{@"browserId" : @(browser_id), @"json" : j ?: @""}];
    return true;  // Swallow our channel so it never reaches the page console.
  }

  NSString* sourceString = @(source.ToString().c_str());
  if ([sourceString hasPrefix:@(mori::kExtensionScheme)] ||
      msg.find("Extension::Error") != std::string::npos ||
      msg.find("Unsupported extension method") != std::string::npos) {
    NSString* messageString = [NSString stringWithUTF8String:msg.c_str()] ?: @"";
    NSLog(@"Mori extension console [%d] %@:%d %@", static_cast<int>(level),
          sourceString ?: @"", line, messageString);
  }
  return false;
}
