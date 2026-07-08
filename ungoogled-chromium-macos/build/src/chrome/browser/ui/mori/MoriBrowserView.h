// Swift-facing wrapper around a single CEF browser, presented as an NSView.
//
// This header is intentionally free of any CEF/C++ types so it can be imported
// from Swift through the bridging header. All Chromium interaction lives in the
// .mm implementation.
#import <Cocoa/Cocoa.h>

NS_ASSUME_NONNULL_BEGIN

@class MoriBrowserView;

typedef void (^MoriJavaScriptResultHandler)(id _Nullable result,
                                              NSString *_Nullable errorMessage);

/// Navigation/display state callbacks. All methods are delivered on the main
/// thread, so delegates may update UI directly.
@protocol MoriBrowserViewDelegate <NSObject>
@optional
- (void)browserView:(MoriBrowserView *)view didChangeTitle:(NSString *)title;
- (void)browserView:(MoriBrowserView *)view didChangeURL:(NSString *)url;
- (void)browserView:(MoriBrowserView *)view
    didChangeLoading:(BOOL)isLoading
           canGoBack:(BOOL)canGoBack
        canGoForward:(BOOL)canGoForward;
- (void)browserView:(MoriBrowserView *)view
    didChangeFaviconURLs:(NSArray<NSString *> *)urls;
/// The page's favicon, downloaded and decoded by Chromium. `image` is nil when
/// the download failed, so the delegate should fall back to its own rendering.
- (void)browserView:(MoriBrowserView *)view
    didLoadFaviconImage:(nullable NSImage *)image;
- (void)browserView:(MoriBrowserView *)view
    didStartNavigationToURL:(NSString *)url
                 isRedirect:(BOOL)isRedirect
                userGesture:(BOOL)userGesture;
- (void)browserView:(MoriBrowserView *)view
    didCommitNavigationToURL:(NSString *)url;
- (void)browserView:(MoriBrowserView *)view
    didFinishNavigationToURL:(NSString *)url
              httpStatusCode:(NSInteger)httpStatusCode;
- (void)browserView:(MoriBrowserView *)view
       didFailLoad:(NSString *)errorText
         failedURL:(NSString *)failedURL;
/// A popup / target=_blank navigation that should open in a brand new tab.
- (void)browserView:(MoriBrowserView *)view
    requestsNewTabWithURL:(NSString *)url;
/// Find-in-page match results: 1-based index of the active match and the total
/// number of matches for the current query (0 when there are none).
- (void)browserView:(MoriBrowserView *)view
    didUpdateFindMatchOrdinal:(int)ordinal
                     ofMatches:(int)count;
/// The page started or stopped producing audible sound.
- (void)browserView:(MoriBrowserView *)view
    didChangeAudioState:(BOOL)audible;
@end

@interface MoriBrowserView : NSView

@property(nonatomic, weak, nullable) id<MoriBrowserViewDelegate> navDelegate;

/// Live state, kept in sync for convenience (also pushed via the delegate).
@property(nonatomic, copy, readonly) NSString *currentURL;
@property(nonatomic, copy, readonly) NSString *currentTitle;
@property(nonatomic, readonly) BOOL isLoading;
@property(nonatomic, readonly) BOOL canGoBack;
@property(nonatomic, readonly) BOOL canGoForward;

/// Designated initializer. The CEF browser is created lazily once the view is
/// installed in a window with a non-empty size.
- (instancetype)initWithURL:(NSString *)url;

- (void)loadURL:(NSString *)url;
- (void)goBack;
- (void)goForward;
- (void)reload;
- (void)reloadIgnoringCache;
- (void)stopLoading;

/// Back/forward navigation entries for the history popover. Each entry is a
/// dictionary with keys "title" (NSString), "url" (NSString) and "offset"
/// (NSNumber: negative = back, 0 = current page, positive = forward), ordered
/// oldest → newest. Empty when there's no live browser.
- (NSArray<NSDictionary<NSString *, id> *> *)backForwardEntries;

/// Navigate to a relative history offset (negative = back, positive = forward).
/// A no-op when the offset is out of range.
- (void)goToHistoryOffset:(NSInteger)offset;

/// Page zoom. Steps are relative to the browser's current zoom; reset returns
/// to 100%.
- (void)zoomIn;
- (void)zoomOut;
- (void)resetZoom;
- (void)setZoomFactor:(double)factor;

/// Find-in-page. `findText:` highlights matches and scrolls to the next/prev
/// one; results arrive via the delegate. `stopFinding:` clears the highlights.
- (void)findText:(NSString *)text forward:(BOOL)forward;
- (void)stopFinding:(BOOL)clearSelection;

/// Developer tools for the underlying browser.
- (void)showDevTools;
- (void)closeDevTools;
- (void)toggleDevTools;

/// Open the system print dialog for the current page.
- (void)printPage;

/// Evaluate JavaScript in the main frame and return a JSON-serializable result
/// via Chromium's DevTools protocol. Used by Mori's local AI tools.
- (BOOL)evaluateJavaScript:(NSString *)source
                completion:(MoriJavaScriptResultHandler)completion;

/// Evaluate JavaScript in Mori's internal media-agent isolated world. Page
/// scripts cannot read or replace globals created in this world.
- (BOOL)evaluateMediaJavaScript:(NSString *)source
                      completion:(MoriJavaScriptResultHandler)completion;

/// Copy the image under a window-space point to the pasteboard using
/// Chromium's native image pipeline. Operates on the already-decoded bitmap, so
/// it works for cross-origin (CORS-restricted) images. Returns NO when there is
/// no live browser/view or no image at that point.
- (BOOL)copyImageAtWindowPoint:(NSPoint)point;

/// Save the image under a window-space point. http(s)/blob/file images download
/// by URL through Chromium's download UI; canvas and data-URL images route
/// through the renderer at the given point. Returns NO when unavailable.
- (BOOL)saveImageURL:(NSString *)url atWindowPoint:(NSPoint)point;

/// Make this browser the first responder / give it keyboard focus.
- (void)focusBrowser;

/// Mirror Mori's pinned-tab state into Chrome's real TabStripModel so Chrome
/// APIs, extension APIs, and tab ordering agree with Mori's sidebar.
- (void)setTabPinned:(BOOL)pinned;

/// CEF browser identifier (0 until the browser exists). Used to attribute
/// media-player updates broadcast from the engine to this tab.
@property(nonatomic, readonly) int browserIdentifier;

/// Drive the injected media agent (play/pause/seek/skip/mute/pip).
- (void)sendMediaCommand:(NSString *)action value:(double)value;

/// Mute or unmute all audio from this tab's page.
- (void)setAudioMuted:(BOOL)muted;
/// Whether this tab's audio is currently muted.
@property(nonatomic, readonly) BOOL isAudioMuted;

/// Tell Chromium this page is (un)occluded — flips `document.hidden`, which
/// drives throttling and the auto-PiP-on-tab-switch behavior.
- (void)setPageHidden:(BOOL)hidden;

/// Explicitly show/hide the native Chromium child view for this tab. This is
/// separate from NSView.hidden because CEF keeps its own visibility state.
- (void)setWebWindowVisible:(BOOL)visible;

/// Let chrome-owned auxiliary views (extension action popovers, etc.) remain
/// drawable while Mori hides normal tab web content behind full-window UI.
- (void)setIgnoresGlobalWebContentSuppression:(BOOL)ignores;

/// Push the auto-PiP preference into this (already-loaded) page.
- (void)applyAutoPiP:(BOOL)enabled;

/// Set the process-wide auto-PiP default applied to newly loaded pages.
+ (void)setAutoPiPEnabled:(BOOL)enabled;

/// Cancel an active Chromium-owned download by id.
+ (BOOL)cancelDownloadWithID:(uint32_t)downloadID;

/// Hide every live web view at once while a full-window SwiftUI overlay (e.g.
/// the new-tab launcher or Settings) is presented.
+ (void)setWebContentSuppressed:(BOOL)suppressed;

/// Close the underlying CEF browser. Safe to call multiple times.
- (void)closeBrowser;

@end

NS_ASSUME_NONNULL_END
