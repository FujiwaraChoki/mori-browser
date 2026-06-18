// MoriBrowserView implemented on chrome's Browser/TabStripModel/WebContents —
// the same pure-ObjC contract the Mori Swift UI compiles against, now backed
// by the full //chrome layer so the REAL extension system (service workers,
// content scripts, chrome.* APIs) sees Mori's tabs natively. No shims.

#import "chrome/browser/ui/mori/MoriBrowserView.h"
#import "chrome/browser/ui/mori/MoriPrivacy.h"
#include "chrome/browser/ui/mori/mori_chrome_hooks.h"

#include <algorithm>
#include <cmath>
#include <map>
#include <string>

#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/scoped_observation.h"
#include "base/strings/stringprintf.h"
#include "base/strings/sys_string_conversions.h"
#include "base/task/sequenced_task_runner.h"
#include "base/values.h"
#include "chrome/browser/devtools/devtools_window.h"
#include "chrome/browser/download/download_core_service_factory.h"
#include "chrome/browser/shell_integration.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/browser.h"
#include "chrome/browser/ui/browser_navigator.h"
#include "chrome/browser/ui/browser_navigator_params.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/tabs/tab_strip_model_observer.h"
#include "chrome/common/chrome_isolated_world_ids.h"
#include "components/favicon/content/content_favicon_driver.h"
#include "components/favicon/core/favicon_driver.h"
#include "components/favicon/core/favicon_driver_observer.h"
#include "components/find_in_page/find_notification_details.h"
#include "components/find_in_page/find_result_observer.h"
#include "components/find_in_page/find_tab_helper.h"
#include "components/find_in_page/find_types.h"
#include "content/public/browser/host_zoom_map.h"
#include "content/public/browser/navigation_controller.h"
#include "content/public/browser/navigation_entry.h"
#include "content/public/browser/navigation_handle.h"
#include "content/public/browser/render_frame_host.h"
#include "content/public/browser/render_widget_host_view.h"
#include "content/public/browser/storage_partition.h"
#include "content/public/browser/web_contents.h"
#include "content/public/common/referrer.h"
#include "content/public/browser/browser_context.h"
#include "content/public/browser/download_manager.h"
#include "components/download/public/common/download_item.h"
#include "content/public/browser/web_contents_observer.h"
#include "base/task/thread_pool.h"
#include "services/network/public/mojom/clear_data_filter.mojom.h"
#include "services/network/public/mojom/cookie_manager.mojom.h"
#include "services/network/public/mojom/network_context.mojom.h"
#include "ui/base/page_transition_types.h"
#include "ui/gfx/image/image.h"
#include "url/gurl.h"

// Swift-exported surface of MoriRoot (MoriRoot.swift).
@interface MoriRoot : NSObject
+ (NSViewController*)makeRootViewController;
+ (void)prepareForTermination;
+ (BOOL)shouldAutoFocusWebContent;
+ (BOOL)handleShortcutEvent:(NSEvent*)event;
+ (void)releaseShortcutEvent:(NSEvent*)event;
+ (BOOL)isReservedShortcutKeyEquivalent:(NSString*)keyEquivalent
                           modifierMask:(NSUInteger)modifierMask;
+ (void)newTab;
+ (void)openNewTabWithURL:(NSString*)url;
+ (void)goBack;
+ (void)goForward;
+ (void)toggleSidebar;
@end

@class MoriBrowserView;

@interface MoriBrowserView (MoriFocusPrivate)
- (BOOL)canReceiveBrowserFocus;
- (BOOL)containsEventLocation:(NSEvent*)event;
@end

@interface NSView (MoriRendererKeyForwarding)
- (NSInteger)keyEvent:(NSEvent*)event;
@end

// ---------------------------------------------------------------------------
// Globals

namespace {

constexpr int kMoriMediaWorldId = ISOLATED_WORLD_ID_CHROME_INTERNAL;

Browser* g_mori_browser = nullptr;
// True while a MoriBrowserView is synchronously creating its own tab via
// Navigate() — the TabStripModel insert observer must not treat that insert
// as an engine-created orphan (it fires before the view can register).
bool g_self_insert_in_progress = false;
NSWindow* __strong g_main_window = nil;
BOOL g_web_content_suppressed = NO;
int g_next_browser_identifier = 1;
// When YES, hiding a tab with a playing video pops it out to Picture-in-Picture
// (driven from -setPageHidden:). Mirrors BrowserSettings.autoPiP.
BOOL g_mori_auto_pip = YES;

std::map<content::WebContents*, __weak MoriBrowserView*>& ViewMap() {
  static std::map<content::WebContents*, __weak MoriBrowserView*> map;
  return map;
}

// Tabs created by the engine (window.open, chrome.tabs.create) waiting for a
// MoriBrowserView to adopt them, keyed by their URL spec.
std::multimap<std::string, content::WebContents*>& OrphanMap() {
  static std::multimap<std::string, content::WebContents*> map;
  return map;
}

NSMutableArray<MoriBrowserView*>* AllViews() {
  static NSMutableArray* views = [NSMutableArray array];
  return views;
}

void DetachMoriWebContentsDelegates(Browser* browser) {
  if (!browser) {
    return;
  }
  for (auto& entry : ViewMap()) {
    content::WebContents* contents = entry.first;
    if (contents && contents->GetDelegate() == browser) {
      contents->SetDelegate(nullptr);
    }
  }
  for (auto& entry : OrphanMap()) {
    content::WebContents* contents = entry.second;
    if (contents && contents->GetDelegate() == browser) {
      contents->SetDelegate(nullptr);
    }
  }
}

MoriBrowserView* ActiveMoriBrowserView() {
  if (!g_mori_browser) {
    return nil;
  }
  content::WebContents* contents =
      g_mori_browser->tab_strip_model()->GetActiveWebContents();
  if (!contents) {
    return nil;
  }
  auto it = ViewMap().find(contents);
  return it == ViewMap().end() ? nil : it->second;
}

MoriBrowserView* FirstFocusableMoriBrowserView() {
  MoriBrowserView* active = ActiveMoriBrowserView();
  if ([active canReceiveBrowserFocus]) {
    return active;
  }
  for (MoriBrowserView* view in [AllViews() reverseObjectEnumerator]) {
    if ([view canReceiveBrowserFocus]) {
      return view;
    }
  }
  return nil;
}

MoriBrowserView* MoriBrowserViewForEvent(NSEvent* event) {
  for (MoriBrowserView* view in [AllViews() reverseObjectEnumerator]) {
    if ([view containsEventLocation:event]) {
      return view;
    }
  }
  return nil;
}

bool HandleNavigationMouseButton(NSEvent* event, bool performNavigation) {
  if (event.type != NSEventTypeOtherMouseDown &&
      event.type != NSEventTypeOtherMouseUp) {
    return false;
  }

  MoriBrowserView* view = MoriBrowserViewForEvent(event);
  switch (event.buttonNumber) {
    case 3:
      if (performNavigation) {
        if (view) {
          [view goBack];
        } else {
          [MoriRoot goBack];
        }
      }
      return true;
    case 4:
      if (performNavigation) {
        if (view) {
          [view goForward];
        } else {
          [MoriRoot goForward];
        }
      }
      return true;
    default:
      return false;
  }
}

bool IsNativeTextInputFirstResponder(NSResponder* responder) {
  return [responder isKindOfClass:[NSTextView class]] ||
         [responder isKindOfClass:[NSTextField class]];
}

NSMenu* FindTopLevelMenu(NSString* title) {
  NSMenu* mainMenu = NSApp.mainMenu;
  if (!mainMenu) {
    return nil;
  }
  for (NSMenuItem* item in mainMenu.itemArray) {
    if ([item.title isEqualToString:title] && item.submenu) {
      return item.submenu;
    }
  }
  return nil;
}

NSMenu* EnsureTopLevelMenu(NSString* title, NSInteger preferredIndex) {
  NSMenu* mainMenu = NSApp.mainMenu;
  if (!mainMenu) {
    return nil;
  }
  if (NSMenu* existing = FindTopLevelMenu(title)) {
    return existing;
  }

  NSMenuItem* item =
      [[NSMenuItem alloc] initWithTitle:title action:nil keyEquivalent:@""];
  NSMenu* submenu = [[NSMenu alloc] initWithTitle:title];
  item.submenu = submenu;
  NSInteger index = std::max<NSInteger>(
      0, std::min<NSInteger>(preferredIndex, mainMenu.numberOfItems));
  [mainMenu insertItem:item atIndex:index];
  return submenu;
}

void EnsureMenuAction(NSMenu* menu,
                      NSString* title,
                      SEL action,
                      NSString* keyEquivalent,
                      NSEventModifierFlags modifiers) {
  if (!menu) {
    return;
  }

  NSMenuItem* item = nil;
  for (NSMenuItem* candidate in menu.itemArray) {
    if (candidate.action == action) {
      item = candidate;
      break;
    }
  }
  if (!item) {
    item = [[NSMenuItem alloc] initWithTitle:title
                                      action:action
                               keyEquivalent:keyEquivalent];
    [menu addItem:item];
  }

  item.title = title;
  item.target = nil;
  item.action = action;
  item.keyEquivalent = keyEquivalent;
  item.keyEquivalentModifierMask = modifiers;
}

void InstallStandardEditMenuShortcuts() {
  NSMenu* editMenu = EnsureTopLevelMenu(@"Edit", 1);
  if (!editMenu) {
    return;
  }

  EnsureMenuAction(editMenu, @"Undo", @selector(undo:), @"z",
                   NSEventModifierFlagCommand);
  EnsureMenuAction(editMenu, @"Redo", @selector(redo:), @"z",
                   NSEventModifierFlagCommand | NSEventModifierFlagShift);
  EnsureMenuAction(editMenu, @"Cut", @selector(cut:), @"x",
                   NSEventModifierFlagCommand);
  EnsureMenuAction(editMenu, @"Copy", @selector(copy:), @"c",
                   NSEventModifierFlagCommand);
  EnsureMenuAction(editMenu, @"Paste", @selector(paste:), @"v",
                   NSEventModifierFlagCommand);
  EnsureMenuAction(editMenu, @"Select All", @selector(selectAll:), @"a",
                   NSEventModifierFlagCommand);
}

void InstallSidebarMenuShortcut() {
  NSMenu* mainMenu = NSApp.mainMenu;
  if (!mainMenu) {
    return;
  }

  NSMenuItem* existingToggle = nil;
  NSMenu* viewMenu = nil;
  NSMutableArray<NSMenu*>* pendingMenus =
      [NSMutableArray arrayWithObject:mainMenu];
  while (pendingMenus.count > 0) {
    NSMenu* menu = pendingMenus.lastObject;
    [pendingMenus removeLastObject];
    for (NSMenuItem* item in menu.itemArray) {
      NSString* key = item.keyEquivalent.lowercaseString ?: @"";
      NSEventModifierFlags modifiers =
          item.keyEquivalentModifierMask &
          (NSEventModifierFlagCommand | NSEventModifierFlagShift |
           NSEventModifierFlagOption | NSEventModifierFlagControl);
      // Strip key equivalents Mori owns so Chromium menu accelerators never
      // intercept them before the Swift shortcut registry. The reservations
      // live beside the shortcut declarations in ShortcutRegistry.swift.
      if ([MoriRoot isReservedShortcutKeyEquivalent:key
                                      modifierMask:modifiers]) {
        item.keyEquivalent = @"";
      }
      if ([item.title isEqualToString:@"Toggle Sidebar"]) {
        existingToggle = item;
      }
      if ([item.title isEqualToString:@"View"] && item.submenu) {
        viewMenu = item.submenu;
      }
      if (item.submenu) {
        [pendingMenus addObject:item.submenu];
      }
    }
  }

  if (!viewMenu) {
    NSMenuItem* viewItem =
        [[NSMenuItem alloc] initWithTitle:@"View" action:nil keyEquivalent:@""];
    viewMenu = [[NSMenu alloc] initWithTitle:@"View"];
    viewItem.submenu = viewMenu;
    [mainMenu addItem:viewItem];
  }

  NSMenuItem* item = existingToggle;
  if (!item) {
    item = [[NSMenuItem alloc] initWithTitle:@"Toggle Sidebar"
                                      action:@selector(toggleSidebar)
                               keyEquivalent:@"s"];
    [viewMenu insertItem:item atIndex:0];
  }
  item.target = (id)[MoriRoot class];
  item.action = @selector(toggleSidebar);
  item.keyEquivalent = @"";
  item.keyEquivalentModifierMask = 0;
}

id NSObjectFromValue(const base::Value& value) {
  switch (value.type()) {
    case base::Value::Type::NONE:
      return [NSNull null];
    case base::Value::Type::BOOLEAN:
      return @(value.GetBool());
    case base::Value::Type::INTEGER:
      return @(value.GetInt());
    case base::Value::Type::DOUBLE:
      return @(value.GetDouble());
    case base::Value::Type::STRING:
      return base::SysUTF8ToNSString(value.GetString());
    case base::Value::Type::LIST: {
      NSMutableArray* array = [NSMutableArray array];
      for (const base::Value& item : value.GetList()) {
        [array addObject:NSObjectFromValue(item)];
      }
      return array;
    }
    case base::Value::Type::DICT: {
      NSMutableDictionary* dict = [NSMutableDictionary dictionary];
      for (auto pair : value.GetDict()) {
        dict[base::SysUTF8ToNSString(pair.first)] =
            NSObjectFromValue(pair.second);
      }
      return dict;
    }
    case base::Value::Type::BINARY:
      return [NSNull null];
  }
  return [NSNull null];
}

}  // namespace

// Engine-facing surface of MoriBrowserView (called from the C++ observers).
@interface MoriBrowserView ()
- (void)engineAttachWebContents:(content::WebContents*)webContents;
- (void)engineWebContentsGone;
- (void)engineSetTitle:(NSString*)title;
- (void)engineSetURL:(NSString*)url;
- (void)engineSetFaviconImage:(NSImage*)image iconURL:(NSString*)iconURL;
- (void)engineSetLoading:(BOOL)loading;
- (void)engineNavStateChanged;
- (void)engineFindReplyOrdinal:(int)ordinal count:(int)count;
- (void)engineAudioStateChanged:(BOOL)audible;
- (void)engineRequestsNewTabWithURL:(NSString*)url;
- (void)engineMaybeRefocus;
- (BOOL)canReceiveBrowserFocus;
- (BOOL)focusRendererAndForwardKeyEventIfNeeded:(NSEvent*)event;
- (BOOL)ensureRendererFirstResponderForKeyEvent:(NSEvent*)event;
- (BOOL)forwardRendererEditShortcutIfNeeded:(NSEvent*)event;
- (BOOL)containsEventLocation:(NSEvent*)event;
- (void)applySuppressionState;
@end

// Menu target for the "Set as Default Browser…" item in the app menu.
@interface MoriMenuActions : NSObject
- (void)setAsDefaultBrowser:(id)sender;
@end

@implementation MoriMenuActions
- (void)setAsDefaultBrowser:(id)sender {
  // May block on the LaunchServices registration; keep it off the UI thread.
  base::ThreadPool::PostTask(
      FROM_HERE, {base::MayBlock()},
      base::BindOnce([] { shell_integration::SetAsDefaultBrowser(); }));
}
@end

// ---------------------------------------------------------------------------
// Per-view WebContents observer → delegate events

namespace mori {

class TabBridge : public content::WebContentsObserver,
                  public find_in_page::FindResultObserver,
                  public favicon::FaviconDriverObserver {
 public:
  TabBridge(content::WebContents* contents, MoriBrowserView* view)
      : content::WebContentsObserver(contents), view_(view) {
    if (auto* helper = find_in_page::FindTabHelper::FromWebContents(contents)) {
      find_observation_.Observe(helper);
    }
    // Chromium downloads and decodes the page's real favicon (any format) and
    // notifies us here; Swift-side favicon rendering never performs its own
    // network fetch and falls back to a local brand glyph or monogram.
    if (auto* favicon_driver =
            favicon::ContentFaviconDriver::FromWebContents(contents)) {
      favicon_observation_.Observe(favicon_driver);
    }
  }
  ~TabBridge() override = default;

  // content::WebContentsObserver:
  void TitleWasSet(content::NavigationEntry* entry) override {
    [view_ engineSetTitle:base::SysUTF16ToNSString(
                              web_contents()->GetTitle())];
  }

  void PrimaryPageChanged(content::Page& page) override {
    [view_ engineSetURL:base::SysUTF8ToNSString(
                             web_contents()->GetLastCommittedURL().spec())];
    [view_ engineNavStateChanged];
    [view_ engineMaybeRefocus];
  }

  void DidFinishNavigation(
      content::NavigationHandle* navigation_handle) override {
    if (!navigation_handle->HasCommitted() ||
        !navigation_handle->IsInPrimaryMainFrame() ||
        !navigation_handle->IsSameDocument()) {
      return;
    }
    [view_ engineSetURL:base::SysUTF8ToNSString(
                             navigation_handle->GetURL().spec())];
    [view_ engineNavStateChanged];
  }

  void DidStartLoading() override {
    [view_ engineSetLoading:YES];
  }

  void DidStopLoading() override {
    [view_ engineSetLoading:NO];
    [view_ engineNavStateChanged];
    [view_ engineMaybeRefocus];
  }

  void OnAudioStateChanged(bool audible) override {
    [view_ engineAudioStateChanged:audible];
  }

  void WebContentsDestroyed() override {
    [view_ engineWebContentsGone];
  }

  // find_in_page::FindResultObserver:
  void OnFindResultAvailable(content::WebContents* web_contents) override {
    auto* helper = find_in_page::FindTabHelper::FromWebContents(web_contents);
    if (!helper) {
      return;
    }
    const find_in_page::FindNotificationDetails& result = helper->find_result();
    [view_ engineFindReplyOrdinal:result.active_match_ordinal()
                            count:result.number_of_matches()];
  }

  void OnFindTabHelperDestroyed(find_in_page::FindTabHelper* helper) override {
    find_observation_.Reset();
  }

  // favicon::FaviconDriverObserver:
  void OnFaviconUpdated(favicon::FaviconDriver* favicon_driver,
                        NotificationIconType notification_icon_type,
                        const GURL& icon_url,
                        bool icon_url_changed,
                        const gfx::Image& image) override {
    // Only the standard 16-DIP page favicon drives the sidebar glyph; ignore
    // the larger touch-icon notifications so a big apple-touch-icon doesn't
    // displace the crisp favicon. `AsNSImage` is nil for an empty image (icon
    // removed), which the Swift side treats as "fall back to monogram".
    if (notification_icon_type !=
        favicon::FaviconDriverObserver::NON_TOUCH_16_DIP) {
      return;
    }
    [view_ engineSetFaviconImage:image.AsNSImage()
                         iconURL:base::SysUTF8ToNSString(icon_url.spec())];
  }

 private:
  __weak MoriBrowserView* view_;
  base::ScopedObservation<find_in_page::FindTabHelper,
                          find_in_page::FindResultObserver>
      find_observation_{this};
  base::ScopedObservation<favicon::FaviconDriver,
                          favicon::FaviconDriverObserver>
      favicon_observation_{this};
};

// ---------------------------------------------------------------------------
// TabStripModel observer: engine-created tabs (popups, chrome.tabs.create)

class MoriTabStripBridge : public TabStripModelObserver {
 public:
  MoriTabStripBridge() = default;

  void OnTabStripModelChanged(
      TabStripModel* tab_strip_model,
      const TabStripModelChange& change,
      const TabStripSelectionChange& selection) override {
    if (change.type() != TabStripModelChange::kInserted) {
      return;
    }
    if (g_self_insert_in_progress) {
      return;  // A MoriBrowserView is inserting its own tab.
    }
    for (const auto& contents : change.GetInsert()->contents) {
      content::WebContents* wc = contents.contents.get();
      if (g_mori_browser) {
        wc->SetDelegate(g_mori_browser);
      }
      if (ViewMap().count(wc)) {
        // A Mori-created tab arrived: the startup blank (if any) can go now
        // without emptying the strip (which would tear the Browser down).
        MaybeCloseStartupBlank();
        continue;
      }
      const GURL url = wc->GetVisibleURL();
      const bool startup_blank =
          ViewMap().empty() &&
          (url.is_empty() || url.spec() == "about:blank" ||
           url.host() == "newtab" || url.host() == "new-tab-page");
      if (startup_blank) {
        // Mori restores its own session; this engine-created NTP is closed as
        // soon as a real tab exists (closing it now would empty the strip and
        // destroy the Browser). Navigate it to about:blank first: the NTP
        // WebUI renderer can't fully connect without Chrome's views window
        // and self-terminates after 15s, which would tear down the tab from
        // under us.
        startup_blank_ = wc;
        wc->GetController().LoadURL(GURL("about:blank"), content::Referrer(),
                                    ui::PAGE_TRANSITION_AUTO_TOPLEVEL,
                                    std::string());
        continue;
      }
      // Engine-created tab (window.open, chrome.tabs.create from extension
      // popups, etc.): stash as orphan and ask the Mori UI to open a tab at
      // that URL; the resulting MoriBrowserView adopts the orphan. Deliver to
      // a view that actually has a navDelegate (background runners may not).
      const std::string spec = url.is_valid() && !url.spec().empty()
                                   ? url.spec()
                                   : std::string("about:blank");
      OrphanMap().emplace(spec, wc);
      NSLog(@"MORI adopt-orphan url=%s", spec.c_str());
      [MoriRoot openNewTabWithURL:base::SysUTF8ToNSString(spec)];
    }
  }

 private:
  void MaybeCloseStartupBlank() {
    content::WebContents* blank = startup_blank_;
    if (!blank) {
      return;
    }
    startup_blank_ = nullptr;
    base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce([](content::WebContents* contents) {
          if (!g_mori_browser) {
            return;
          }
          TabStripModel* model = g_mori_browser->tab_strip_model();
          const int index = model->GetIndexOfWebContents(contents);
          if (index != TabStripModel::kNoTab && model->count() > 1 &&
              !ViewMap().count(contents)) {
            model->CloseWebContentsAt(index, TabCloseTypes::CLOSE_NONE);
          }
        }, blank));
  }

  raw_ptr<content::WebContents> startup_blank_ = nullptr;
};

// Feeds Mori's DownloadStore (which listens for the CEF-era
// "MoriDownloadUpdated" NSNotification) from Chrome's real DownloadManager.
class MoriDownloadBridge : public content::DownloadManager::Observer,
                           public download::DownloadItem::Observer {
 public:
  explicit MoriDownloadBridge(content::DownloadManager* manager) {
    manager->AddObserver(this);
  }

  void OnDownloadCreated(content::DownloadManager* manager,
                         download::DownloadItem* item) override {
    item->AddObserver(this);
    Broadcast(item);
  }

  void OnDownloadUpdated(download::DownloadItem* item) override {
    Broadcast(item);
  }

  void OnDownloadDestroyed(download::DownloadItem* item) override {
    item->RemoveObserver(this);
  }

 private:
  void Broadcast(download::DownloadItem* item) {
    const auto state = item->GetState();
    NSDictionary* info = @{
      @"id" : @(item->GetId()),
      @"url" : base::SysUTF8ToNSString(item->GetURL().spec()),
      @"filename" : base::SysUTF8ToNSString(
          item->GetFileNameToReportUser().AsUTF8Unsafe()),
      @"path" : base::SysUTF8ToNSString(
          item->GetTargetFilePath().AsUTF8Unsafe()),
      @"percent" : @(item->PercentComplete()),
      @"received" : @(item->GetReceivedBytes()),
      @"total" : @(item->GetTotalBytes()),
      @"speed" : @(item->CurrentSpeed()),
      @"inProgress" : @(state == download::DownloadItem::IN_PROGRESS),
      @"complete" : @(state == download::DownloadItem::COMPLETE),
      @"canceled" : @(state == download::DownloadItem::CANCELLED ||
                      state == download::DownloadItem::INTERRUPTED),
    };
    [[NSNotificationCenter defaultCenter]
        postNotificationName:@"MoriDownloadUpdated"
                      object:nil
                    userInfo:info];
  }
};

content::DownloadManager* MoriDownloadManager() {
  return g_mori_browser
             ? g_mori_browser->profile()->GetDownloadManager()
             : nullptr;
}

void OnBrowserWindowCreated(Browser* browser) {
  NSLog(@"MORI OnBrowserWindowCreated type=%d existing=%p", (int)browser->type(),
        g_mori_browser);
  if (g_mori_browser || browser->type() != Browser::TYPE_NORMAL) {
    return;
  }
  g_mori_browser = browser;
  static MoriTabStripBridge* bridge = new MoriTabStripBridge();
  browser->tab_strip_model()->AddObserver(bridge);
  NSLog(@"MORI adopted browser %p", browser);
}

void OnBrowserWindowDestroyed(Browser* browser) {
  if (g_mori_browser == browser) {
    DetachMoriWebContentsDelegates(browser);
    g_mori_browser = nullptr;
  }
}

void EnsureMoriUIStarted(Browser* browser) {
  NSLog(@"MORI EnsureMoriUIStarted type=%d window=%p", (int)browser->type(),
        g_main_window);
  if (g_main_window || browser->type() != Browser::TYPE_NORMAL) {
    return;
  }

  NSWindow* window = [[NSWindow alloc]
      initWithContentRect:NSMakeRect(0, 0, 1280, 820)
                styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                          NSWindowStyleMaskMiniaturizable |
                          NSWindowStyleMaskResizable |
                          NSWindowStyleMaskFullSizeContentView
                  backing:NSBackingStoreBuffered
                    defer:NO];
  window.title = @"Mori";
  window.titlebarAppearsTransparent = YES;
  window.titleVisibility = NSWindowTitleHidden;
  NSButton* closeButton = [window standardWindowButton:NSWindowCloseButton];
  NSButton* miniaturizeButton =
      [window standardWindowButton:NSWindowMiniaturizeButton];
  NSButton* zoomButton = [window standardWindowButton:NSWindowZoomButton];
  NSButton* titlebarButtons[3] = {closeButton, miniaturizeButton, zoomButton};
  for (NSButton* button : titlebarButtons) {
    if (!button) {
      continue;
    }
    button.hidden = YES;
    button.enabled = NO;
    button.alphaValue = 0;
  }
  window.releasedWhenClosed = NO;
  window.collectionBehavior |= NSWindowCollectionBehaviorFullScreenPrimary;
  window.contentMinSize = NSMakeSize(720, 480);
  window.contentViewController = [MoriRoot makeRootViewController];
  [window center];
  [window makeKeyAndOrderFront:nil];
  [NSApp activateIgnoringOtherApps:YES];
  g_main_window = window;
  NSLog(@"MORI main window up visible=%d", window.isVisible ? 1 : 0);

  // MoriApplication.sendEvent equivalent: Mori's shortcut registry gets first
  // crack at key events; consume the handled ones.
  [NSEvent
      addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown |
                                           NSEventMaskKeyUp |
                                           NSEventMaskFlagsChanged |
                                           NSEventMaskLeftMouseDown |
                                           NSEventMaskRightMouseDown |
                                           NSEventMaskOtherMouseDown |
                                           NSEventMaskOtherMouseUp
                                   handler:^NSEvent*(NSEvent* event) {
                                     if (event.type == NSEventTypeKeyDown) {
                                       // ⌘S (toggle sidebar) and ⌘T (toggle
                                       // omnibox) flow through the shared
                                       // shortcut registry below like every
                                       // other shortcut, so they behave
                                       // identically whether chrome or web
                                       // content has focus.
                                       if ([MoriRoot
                                               handleShortcutEvent:event]) {
                                         return nil;
                                       }
                                       if (MoriBrowserView* view =
                                               FirstFocusableMoriBrowserView()) {
                                         if ([view forwardRendererEditShortcutIfNeeded:
                                                   event]) {
                                           return nil;
                                         }
                                         if ([view focusRendererAndForwardKeyEventIfNeeded:
                                                   event]) {
                                           return nil;
                                         }
                                         [view ensureRendererFirstResponderForKeyEvent:
                                                   event];
                                       }
                                     } else if (event.type ==
                                                NSEventTypeKeyUp) {
                                       [MoriRoot releaseShortcutEvent:event];
                                       if (MoriBrowserView* view =
                                               FirstFocusableMoriBrowserView()) {
                                         [view ensureRendererFirstResponderForKeyEvent:
                                                   event];
                                       }
                                     } else if (event.type ==
                                                NSEventTypeFlagsChanged) {
                                       [MoriRoot releaseShortcutEvent:event];
                                     } else if (event.type ==
                                                    NSEventTypeLeftMouseDown ||
                                                event.type ==
                                                    NSEventTypeRightMouseDown ||
                                                event.type ==
                                                    NSEventTypeOtherMouseDown) {
                                       if (MoriBrowserView* view =
                                               MoriBrowserViewForEvent(event)) {
                                         [view focusBrowser];
                                       }
                                       if (HandleNavigationMouseButton(
                                               event, false)) {
                                         return nil;
                                       }
                                     } else if (event.type ==
                                                NSEventTypeOtherMouseUp) {
                                       if (HandleNavigationMouseButton(
                                               event, true)) {
                                         return nil;
                                       }
                                     }
                                     return event;
                                   }];

  [[NSNotificationCenter defaultCenter]
      addObserverForName:NSApplicationWillTerminateNotification
                  object:nil
                   queue:[NSOperationQueue mainQueue]
              usingBlock:^(NSNotification* note) {
                DetachMoriWebContentsDelegates(g_mori_browser);
                [MoriRoot prepareForTermination];
              }];

  // Chrome's real downloads → Mori's DownloadStore.
  if (content::DownloadManager* manager = MoriDownloadManager()) {
    static MoriDownloadBridge* downloads = new MoriDownloadBridge(manager);
    (void)downloads;
  }

  // "Mori ▸ Set as Default Browser…" in the application menu.
  static MoriMenuActions* menuActions = [[MoriMenuActions alloc] init];
  NSMenu* appMenu = [[NSApp.mainMenu itemAtIndex:0] submenu];
  if (appMenu) {
    NSMenuItem* item =
        [[NSMenuItem alloc] initWithTitle:@"Set as Default Browser…"
                                   action:@selector(setAsDefaultBrowser:)
                            keyEquivalent:@""];
    item.target = menuActions;
    [appMenu insertItem:item atIndex:1];
    [appMenu insertItem:[NSMenuItem separatorItem] atIndex:2];
  }
  InstallStandardEditMenuShortcuts();
  InstallSidebarMenuShortcut();
}

NSWindow* MoriMainWindow() {
  return g_main_window;
}

Browser* MoriBrowser() {
  return g_mori_browser;
}

}  // namespace mori

// ---------------------------------------------------------------------------
// MoriBrowserView

@implementation MoriBrowserView {
  content::WebContents* _webContents;  // Owned by the TabStripModel.
  std::unique_ptr<mori::TabBridge> _bridge;
  NSString* _pendingURL;
  NSView* __strong _webView;
  double _zoomLevel;
  BOOL _webWindowVisible;
  BOOL _ignoresGlobalWebContentSuppression;
  int _browserIdentifier;
}

@synthesize navDelegate = _navDelegate;
@synthesize currentURL = _currentURL;
@synthesize currentTitle = _currentTitle;
@synthesize isLoading = _isLoading;
@synthesize canGoBack = _canGoBack;
@synthesize canGoForward = _canGoForward;

- (instancetype)initWithURL:(NSString*)url {
  if ((self = [super initWithFrame:NSZeroRect])) {
    _pendingURL = [url copy] ?: @"about:blank";
    _currentURL = [url copy] ?: @"";
    _currentTitle = @"";
    _webWindowVisible = YES;
    _browserIdentifier = g_next_browser_identifier++;
    [AllViews() addObject:self];
  }
  return self;
}

- (void)dealloc {
  [AllViews() removeObject:self];
}

- (void)viewDidMoveToWindow {
  [super viewDidMoveToWindow];
  [self maybeCreateTab];
}

- (void)layout {
  [super layout];
  [self maybeCreateTab];
}

// Creates (or adopts) the engine tab once the view lives in a window — the
// same lazy contract as the CEF-backed implementation.
- (void)maybeCreateTab {
  if (_webContents || !self.window || NSIsEmptyRect(self.bounds) ||
      !g_mori_browser) {
    return;
  }

  GURL url(base::SysNSStringToUTF8(_pendingURL));
  if (!url.is_valid()) {
    url = GURL("about:blank");
  }

  // Adopt an engine-created orphan (popup / chrome.tabs.create) if one is
  // waiting at this URL.
  auto orphan = OrphanMap().find(url.spec());
  if (orphan == OrphanMap().end() && url.spec() != "about:blank") {
    orphan = OrphanMap().find("about:blank");
  }
  if (orphan != OrphanMap().end()) {
    content::WebContents* wc = orphan->second;
    OrphanMap().erase(orphan);
    [self engineAttachWebContents:wc];
    return;
  }

  NavigateParams params(g_mori_browser, url,
                        ui::PAGE_TRANSITION_AUTO_TOPLEVEL);
  params.disposition = WindowOpenDisposition::NEW_BACKGROUND_TAB;
  params.window_action = NavigateParams::WindowAction::kNoAction;
  g_self_insert_in_progress = true;
  Navigate(&params);
  g_self_insert_in_progress = false;
  if (content::WebContents* wc = params.navigated_or_inserted_contents) {
    [self engineAttachWebContents:wc];
  }
}

- (void)engineAttachWebContents:(content::WebContents*)webContents {
  _webContents = webContents;
  if (g_mori_browser) {
    webContents->SetDelegate(g_mori_browser);
  }
  ViewMap()[webContents] = self;
  _bridge = std::make_unique<mori::TabBridge>(webContents, self);

  NSView* webView = webContents->GetNativeView().GetNativeNSView();
  _webView = webView;
  webView.frame = self.bounds;
  webView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
  [self addSubview:webView];
  [self applySuppressionState];
  if (!self.isHidden && _webWindowVisible) {
    [self focusBrowser];
  }
}

- (void)engineWebContentsGone {
  if (_webContents) {
    ViewMap().erase(_webContents);
    _webContents = nullptr;
  }
  _bridge.reset();
  [_webView removeFromSuperview];
  _webView = nil;
}

// MARK: engine state → delegate

- (void)engineSetTitle:(NSString*)title {
  _currentTitle = [title copy] ?: @"";
  if ([_navDelegate respondsToSelector:@selector(browserView:
                                           didChangeTitle:)]) {
    [_navDelegate browserView:self didChangeTitle:_currentTitle];
  }
}

- (void)engineSetURL:(NSString*)url {
  _currentURL = [url copy] ?: @"";
  if ([_navDelegate respondsToSelector:@selector(browserView:didChangeURL:)]) {
    [_navDelegate browserView:self didChangeURL:_currentURL];
  }
  if ([_navDelegate respondsToSelector:@selector(browserView:
                                           didCommitNavigationToURL:)]) {
    [_navDelegate browserView:self didCommitNavigationToURL:_currentURL];
  }
}

- (void)engineSetFaviconImage:(NSImage*)image iconURL:(NSString*)iconURL {
  if (iconURL.length &&
      [_navDelegate respondsToSelector:@selector(browserView:
                                           didChangeFaviconURLs:)]) {
    [_navDelegate browserView:self didChangeFaviconURLs:@[ iconURL ]];
  }
  if ([_navDelegate respondsToSelector:@selector(browserView:
                                           didLoadFaviconImage:)]) {
    [_navDelegate browserView:self didLoadFaviconImage:image];
  }
}

- (void)engineSetLoading:(BOOL)loading {
  const BOOL wasLoading = _isLoading;
  _isLoading = loading;
  [self engineNavStateChanged];
  if (!wasLoading && loading) {
    if ([_navDelegate respondsToSelector:@selector
                      (browserView:didStartNavigationToURL:isRedirect:
                                      userGesture:)]) {
      [_navDelegate browserView:self
          didStartNavigationToURL:_currentURL
                       isRedirect:NO
                      userGesture:NO];
    }
  } else if (wasLoading && !loading) {
    if ([_navDelegate respondsToSelector:@selector
                      (browserView:didFinishNavigationToURL:httpStatusCode:)]) {
      [_navDelegate browserView:self
          didFinishNavigationToURL:_currentURL
                    httpStatusCode:200];
    }
  }
}

- (void)engineNavStateChanged {
  if (_webContents) {
    _canGoBack = _webContents->GetController().CanGoBack();
    _canGoForward = _webContents->GetController().CanGoForward();
  }
  if ([_navDelegate respondsToSelector:@selector
                    (browserView:didChangeLoading:canGoBack:canGoForward:)]) {
    [_navDelegate browserView:self
             didChangeLoading:_isLoading
                    canGoBack:_canGoBack
                 canGoForward:_canGoForward];
  }
}

- (void)engineFindReplyOrdinal:(int)ordinal count:(int)count {
  if ([_navDelegate respondsToSelector:@selector
                    (browserView:didUpdateFindMatchOrdinal:ofMatches:)]) {
    [_navDelegate browserView:self
        didUpdateFindMatchOrdinal:ordinal
                        ofMatches:count];
  }
}

- (void)engineRequestsNewTabWithURL:(NSString*)url {
  if ([_navDelegate respondsToSelector:@selector(browserView:
                                           requestsNewTabWithURL:)]) {
    [_navDelegate browserView:self requestsNewTabWithURL:url];
  }
}

- (void)engineAudioStateChanged:(BOOL)audible {
  if ([_navDelegate respondsToSelector:@selector(browserView:
                                           didChangeAudioState:)]) {
    [_navDelegate browserView:self didChangeAudioState:audible];
  }
}

- (void)setAudioMuted:(BOOL)muted {
  if (_webContents) {
    _webContents->SetAudioMuted(muted);
  }
}

- (BOOL)isAudioMuted {
  return _webContents ? _webContents->IsAudioMuted() : NO;
}

// MARK: commands

- (void)loadURL:(NSString*)url {
  _pendingURL = [url copy];
  if (!_webContents) {
    [self maybeCreateTab];
    return;
  }
  GURL gurl(base::SysNSStringToUTF8(url));
  if (!gurl.is_valid()) {
    return;
  }
  content::NavigationController::LoadURLParams params(gurl);
  params.transition_type = ui::PAGE_TRANSITION_TYPED;
  _webContents->GetController().LoadURLWithParams(params);
}

- (void)goBack {
  if (_webContents && _webContents->GetController().CanGoBack()) {
    _webContents->GetController().GoBack();
  }
}

- (void)goForward {
  if (_webContents && _webContents->GetController().CanGoForward()) {
    _webContents->GetController().GoForward();
  }
}

- (void)reload {
  if (_webContents) {
    _webContents->GetController().Reload(content::ReloadType::NORMAL, false);
  }
}

- (void)reloadIgnoringCache {
  if (_webContents) {
    _webContents->GetController().Reload(content::ReloadType::BYPASSING_CACHE,
                                         false);
  }
}

- (void)stopLoading {
  if (_webContents) {
    _webContents->Stop();
  }
}

// MARK: zoom

- (void)zoomIn {
  [self adjustZoomBy:0.5];
}

- (void)zoomOut {
  [self adjustZoomBy:-0.5];
}

- (void)resetZoom {
  _zoomLevel = 0;
  [self applyZoom];
}

- (void)setZoomFactor:(double)factor {
  if (factor <= 0) {
    return;
  }
  _zoomLevel = std::log(factor) / std::log(1.2);
  [self applyZoom];
}

- (void)adjustZoomBy:(double)delta {
  _zoomLevel += delta;
  [self applyZoom];
}

- (void)applyZoom {
  if (_webContents) {
    content::HostZoomMap::SetZoomLevel(_webContents, _zoomLevel);
  }
}

// MARK: find in page (real FindTabHelper — highlights, tickmarks, ordinals)

- (void)findText:(NSString*)text forward:(BOOL)forward {
  if (!_webContents) {
    return;
  }
  auto* helper = find_in_page::FindTabHelper::FromWebContents(_webContents);
  if (!helper) {
    return;
  }
  helper->StartFinding(base::SysNSStringToUTF16(text), forward,
                       /*case_sensitive=*/false, /*find_match=*/true);
}

- (void)stopFinding:(BOOL)clearSelection {
  if (!_webContents) {
    return;
  }
  auto* helper = find_in_page::FindTabHelper::FromWebContents(_webContents);
  if (!helper) {
    return;
  }
  helper->StopFinding(clearSelection
                          ? find_in_page::SelectionAction::kClear
                          : find_in_page::SelectionAction::kKeep);
}

// MARK: devtools / print

- (void)showDevTools {
  if (_webContents) {
    DevToolsWindow::OpenDevToolsWindow(
        _webContents, DevToolsOpenedByAction::kUnknown);
  }
}

- (void)closeDevTools {
  if (_webContents) {
    if (DevToolsWindow::GetInstanceForInspectedWebContents(_webContents) &&
        g_mori_browser) {
      // Acts on the browser's active tab — Mori keeps that in lockstep with
      // its own selection (focusBrowser).
      DevToolsWindow::ToggleDevToolsWindow(g_mori_browser,
                                           DevToolsToggleAction::Toggle());
    }
  }
}

- (void)toggleDevTools {
  if (!_webContents) {
    return;
  }
  if (DevToolsWindow::GetInstanceForInspectedWebContents(_webContents)) {
    [self closeDevTools];
  } else {
    [self showDevTools];
  }
}

- (void)printPage {
  // TODO(mori): wire chrome printing (printing::StartPrint).
}

// MARK: scripting

- (BOOL)evaluateJavaScript:(NSString*)source
                   worldID:(int)worldID
                completion:(MoriJavaScriptResultHandler)completion {
  if (![NSThread isMainThread]) {
    NSString* sourceCopy = [source copy];
    MoriJavaScriptResultHandler handler = [completion copy];
    dispatch_async(dispatch_get_main_queue(), ^{
      if (![self evaluateJavaScript:sourceCopy worldID:worldID completion:handler]) {
        handler(nil, @"Browser unavailable");
      }
    });
    return YES;
  }
  if (!_webContents) {
    return NO;
  }
  content::RenderFrameHost* frame = _webContents->GetPrimaryMainFrame();
  if (!frame) {
    return NO;
  }
  MoriJavaScriptResultHandler handler = [completion copy];
  frame->ExecuteJavaScriptForTests(
      base::SysNSStringToUTF16(source),
      base::BindOnce(^(base::Value value) {
        handler(NSObjectFromValue(value), nil);
      }),
      worldID);
  return YES;
}

- (BOOL)evaluateJavaScript:(NSString*)source
                completion:(MoriJavaScriptResultHandler)completion {
  return [self evaluateJavaScript:source
                          worldID:content::ISOLATED_WORLD_ID_GLOBAL
                       completion:completion];
}

- (BOOL)evaluateMediaJavaScript:(NSString*)source
                      completion:(MoriJavaScriptResultHandler)completion {
  return [self evaluateJavaScript:source
                          worldID:kMoriMediaWorldId
                       completion:completion];
}

// MARK: image context-menu actions (native copy/save)

// Convert a window-space point (AppKit, bottom-left origin) into the render
// widget's viewport coordinates (top-left origin, DIP) that CopyImageAt /
// SaveImageAt expect. Returns NO if there's no live render view.
- (BOOL)viewportPointForWindowPoint:(NSPoint)windowPoint
                               outX:(int*)outX
                               outY:(int*)outY {
  content::RenderWidgetHostView* rv =
      _webContents ? _webContents->GetRenderWidgetHostView() : nullptr;
  NSView* nsview = rv ? rv->GetNativeView().GetNativeNSView() : nil;
  if (!nsview) {
    return NO;
  }
  NSPoint local = [nsview convertPoint:windowPoint fromView:nil];
  CGFloat y = nsview.isFlipped ? local.y : nsview.bounds.size.height - local.y;
  *outX = static_cast<int>(std::lround(local.x));
  *outY = static_cast<int>(std::lround(y));
  return YES;
}

- (BOOL)copyImageAtWindowPoint:(NSPoint)windowPoint {
  if (!_webContents) {
    return NO;
  }
  content::RenderFrameHost* frame = _webContents->GetPrimaryMainFrame();
  int x = 0, y = 0;
  if (!frame ||
      ![self viewportPointForWindowPoint:windowPoint outX:&x outY:&y]) {
    return NO;
  }
  // Copies the already-decoded bitmap — no network fetch, so cross-origin
  // (CORS-restricted) images copy fine.
  frame->CopyImageAt(x, y);
  return YES;
}

- (BOOL)saveImageURL:(NSString*)url atWindowPoint:(NSPoint)windowPoint {
  if (!_webContents) {
    return NO;
  }
  content::RenderFrameHost* frame = _webContents->GetPrimaryMainFrame();
  if (!frame) {
    return NO;
  }
  GURL gurl(base::SysNSStringToUTF8(url ?: @""));
  // Canvas and large data-URL images have no fetchable URL; let the renderer
  // post back the download (mirrors Chromium's own RenderViewContextMenu).
  if (!gurl.is_valid() || gurl.SchemeIs("data")) {
    int x = 0, y = 0;
    if (![self viewportPointForWindowPoint:windowPoint outX:&x outY:&y]) {
      return NO;
    }
    frame->SaveImageAt(x, y);
    return YES;
  }
  // http(s)/blob/file images download by URL through Chromium's download UI,
  // using the frame's isolation info (correct referrer, cookies, etc.).
  _webContents->SaveFrame(gurl, content::Referrer(), frame);
  return YES;
}

// MARK: focus / visibility / lifetime

- (void)engineMaybeRefocus {
  if (!_webContents || self.isHidden || !_webWindowVisible || _webView.hidden ||
      ![MoriRoot shouldAutoFocusWebContent]) {
    return;
  }
  dispatch_async(dispatch_get_main_queue(), ^{
    if (!self->_webContents || self.isHidden || !self->_webWindowVisible ||
        self->_webView.hidden || ![MoriRoot shouldAutoFocusWebContent]) {
      return;
    }
    NSWindow* window = self.window ?: self->_webView.window;
    if (IsNativeTextInputFirstResponder(window.firstResponder)) {
      return;
    }
    [self focusBrowser];
  });
}

- (BOOL)canReceiveBrowserFocus {
  return _webContents && !self.isHidden && _webWindowVisible &&
         !_webView.hidden && self.window;
}

- (BOOL)focusRendererAndForwardKeyEventIfNeeded:(NSEvent*)event {
  if (event.type != NSEventTypeKeyDown || ![self canReceiveBrowserFocus]) {
    return NO;
  }
  NSEventModifierFlags modifiers =
      event.modifierFlags &
      (NSEventModifierFlagCommand | NSEventModifierFlagOption |
       NSEventModifierFlagControl);
  if (modifiers != 0) {
    return NO;
  }

  content::RenderWidgetHostView* renderView =
      _webContents->GetRenderWidgetHostView();
  NSView* rendererNativeView =
      renderView ? renderView->GetNativeView().GetNativeNSView() : nil;
  NSWindow* window = self.window ?: rendererNativeView.window ?: _webView.window;
  if (!window || (event.window && event.window != window) ||
      !rendererNativeView.window) {
    return NO;
  }
  if (IsNativeTextInputFirstResponder(window.firstResponder)) {
    return NO;
  }
  if (window.firstResponder == rendererNativeView) {
    return NO;
  }

  if (!window.isKeyWindow) {
    [window makeKeyWindow];
  }
  [window makeFirstResponder:rendererNativeView];
  if (renderView) {
    renderView->Focus();
  }
  _webContents->Focus();

  if (window.firstResponder != rendererNativeView) {
    return NO;
  }

  if ([rendererNativeView respondsToSelector:@selector(keyEvent:)]) {
    [rendererNativeView keyEvent:event];
  } else {
    [rendererNativeView keyDown:event];
  }
  return YES;
}

- (BOOL)ensureRendererFirstResponderForKeyEvent:(NSEvent*)event {
  if (![self canReceiveBrowserFocus]) {
    return NO;
  }
  NSEventModifierFlags modifiers =
      event.modifierFlags &
      (NSEventModifierFlagCommand | NSEventModifierFlagOption |
       NSEventModifierFlagControl);
  if (modifiers != 0) {
    return NO;
  }

  content::RenderWidgetHostView* renderView =
      _webContents->GetRenderWidgetHostView();
  NSView* rendererNativeView =
      renderView ? renderView->GetNativeView().GetNativeNSView() : nil;
  NSWindow* window = self.window ?: rendererNativeView.window ?: _webView.window;
  if (!window || (event.window && event.window != window) ||
      !rendererNativeView.window) {
    return NO;
  }
  if (IsNativeTextInputFirstResponder(window.firstResponder)) {
    return NO;
  }
  if (window.firstResponder == rendererNativeView) {
    if (renderView) {
      renderView->Focus();
    }
    _webContents->Focus();
    return YES;
  }

  if (!window.isKeyWindow) {
    [window makeKeyWindow];
  }
  [window makeFirstResponder:rendererNativeView];
  if (renderView) {
    renderView->Focus();
  }
  _webContents->Focus();
  return window.firstResponder == rendererNativeView;
}

- (BOOL)forwardRendererEditShortcutIfNeeded:(NSEvent*)event {
  if (event.type != NSEventTypeKeyDown || ![self canReceiveBrowserFocus]) {
    return NO;
  }

  NSEventModifierFlags modifiers =
      event.modifierFlags &
      (NSEventModifierFlagCommand | NSEventModifierFlagShift |
       NSEventModifierFlagOption | NSEventModifierFlagControl);
  const bool commandOnly = modifiers == NSEventModifierFlagCommand;
  const bool commandShift =
      modifiers == (NSEventModifierFlagCommand | NSEventModifierFlagShift);
  const unsigned short keyCode = event.keyCode;
  const bool standardEditShortcut =
      commandOnly &&
      (keyCode == 0 ||   // A
       keyCode == 6 ||   // Z
       keyCode == 7 ||   // X
       keyCode == 8 ||   // C
       keyCode == 9);    // V
  const bool redoShortcut = commandShift && keyCode == 6;  // Z
  if (!standardEditShortcut && !redoShortcut) {
    return NO;
  }

  content::RenderWidgetHostView* renderView =
      _webContents->GetRenderWidgetHostView();
  NSView* rendererNativeView =
      renderView ? renderView->GetNativeView().GetNativeNSView() : nil;
  NSWindow* window = self.window ?: rendererNativeView.window ?: _webView.window;
  if (!window || (event.window && event.window != window) ||
      !rendererNativeView.window) {
    return NO;
  }
  if (IsNativeTextInputFirstResponder(window.firstResponder)) {
    return NO;
  }
  if (!window.isKeyWindow) {
    [window makeKeyWindow];
  }
  [window makeFirstResponder:rendererNativeView];
  if (renderView) {
    renderView->Focus();
  }
  _webContents->Focus();
  if (window.firstResponder != rendererNativeView) {
    return NO;
  }

  SEL action = nil;
  if (commandOnly) {
    switch (keyCode) {
      case 0:
        action = @selector(selectAll:);
        break;
      case 6:
        action = @selector(undo:);
        break;
      case 7:
        action = @selector(cut:);
        break;
      case 8:
        action = @selector(copy:);
        break;
      case 9:
        action = @selector(paste:);
        break;
      default:
        break;
    }
  } else if (redoShortcut) {
    action = @selector(redo:);
  }
  if (action &&
      [NSApp sendAction:action to:rendererNativeView from:self]) {
    return YES;
  }
  if (action && [NSApp sendAction:action to:nil from:self]) {
    return YES;
  }
  return [rendererNativeView performKeyEquivalent:event];
}

- (BOOL)containsEventLocation:(NSEvent*)event {
  if (![self canReceiveBrowserFocus]) {
    return NO;
  }
  NSWindow* window = self.window ?: _webView.window;
  if (!window || event.window != window) {
    return NO;
  }
  NSPoint localPoint = [self convertPoint:event.locationInWindow fromView:nil];
  return NSPointInRect(localPoint, self.bounds);
}

- (void)focusBrowser {
  if (!_webContents || !g_mori_browser) {
    return;
  }
  // Keep chrome's "active tab" (what chrome.tabs and extension actions see)
  // in lockstep with Mori's selection.
  TabStripModel* model = g_mori_browser->tab_strip_model();
  const int index = model->GetIndexOfWebContents(_webContents);
  if (index != TabStripModel::kNoTab && model->active_index() != index) {
    model->ActivateTabAt(index);
  }
  content::RenderWidgetHostView* renderView =
      _webContents->GetRenderWidgetHostView();
  NSView* rendererNativeView =
      renderView ? renderView->GetNativeView().GetNativeNSView() : nil;
  NSWindow* window = self.window ?: rendererNativeView.window ?: _webView.window;
  if (window && !window.isKeyWindow) {
    [window makeKeyWindow];
  }
  if (rendererNativeView.window) {
    [rendererNativeView.window makeFirstResponder:rendererNativeView];
  }
  if (renderView) {
    renderView->Focus();
  }
  _webContents->Focus();
}

- (void)setTabPinned:(BOOL)pinned {
  if (!_webContents || !g_mori_browser) {
    return;
  }
  TabStripModel* model = g_mori_browser->tab_strip_model();
  const int index = model->GetIndexOfWebContents(_webContents);
  if (index != TabStripModel::kNoTab && model->IsTabPinned(index) != pinned) {
    model->SetTabPinned(index, pinned);
  }
}

- (int)browserIdentifier {
  return _browserIdentifier;
}

static BOOL MoriIsAllowedMediaAction(NSString* action) {
  static NSSet<NSString*>* allowed;
  static dispatch_once_t once;
  dispatch_once(&once, ^{
    allowed = [NSSet setWithObjects:@"play", @"pause", @"toggle", @"seek",
                                   @"seekBy", @"mute", @"pip", @"pipEnter",
                                   @"pipExit", nil];
  });
  return [allowed containsObject:action];
}

static NSString* MoriMediaCommandScript(NSString* action, double value) {
  std::string script = base::StringPrintf(R"JS(
(() => {
  const action = "%s";
  const value = %.17g;
  const closestElement = (target, selector) => {
    for (let n = target; n; n = n.parentNode || (n.host || null)) {
      if (n.nodeType === 1 && n.matches && n.matches(selector)) return n;
    }
    return null;
  };
  const hasAudibleTrack = (el) =>
    !el.muted && (typeof el.volume !== 'number' || el.volume > 0);
  const isYouTubePreview = (el) => {
    const host = location.hostname.replace(/^www\./, '');
    if (host !== 'youtube.com' && host !== 'm.youtube.com') return false;
    if (!el.muted) return false;
    return !closestElement(el, '#movie_player, ytd-player, #shorts-player');
  };
  const eligible = (el) => {
    if (!el || isYouTubePreview(el)) return false;
    // These markers are written by Mori's media agent in the same isolated
    // world; page scripts cannot spoof them with main-world expandos.
    if (el.__moriMediaEligible || el.__moriMediaUserSelected) return true;
    return !el.paused && hasAudibleTrack(el);
  };
  const pick = () => {
    const els = Array.from(document.querySelectorAll('video,audio')).filter((m) =>
      (m.currentSrc || m.src) && eligible(m));
    if (!els.length) return null;
    els.sort((a, b) => {
      const ap = a.paused ? 0 : 1;
      const bp = b.paused ? 0 : 1;
      if (ap !== bp) return bp - ap;
      const aa = (a.videoWidth || 0) * (a.videoHeight || 0);
      const ba = (b.videoWidth || 0) * (b.videoHeight || 0);
      return ba - aa;
    });
    return els[0];
  };
  const pickVideo = () => {
    const el = pick();
    return el && el.tagName === 'VIDEO' ? el : null;
  };
  const pipEnter = () => {
    try {
      if (document.pictureInPictureElement) return;
      const el = pickVideo();
      if (el && el.requestPictureInPicture) el.requestPictureInPicture().catch(() => {});
    } catch (e) {}
  };
  const pipExit = () => {
    try {
      if (document.pictureInPictureElement) document.exitPictureInPicture().catch(() => {});
    } catch (e) {}
  };
  if (action === 'pip') {
    try {
      if (document.pictureInPictureElement) {
        document.exitPictureInPicture().catch(() => {});
      } else {
        pipEnter();
      }
    } catch (e) {}
    return;
  }
  if (action === 'pipEnter') { pipEnter(); return; }
  if (action === 'pipExit') { pipExit(); return; }
  const el = pick();
  if (!el) return;
  switch (action) {
    case 'play':
      if (el.play) el.play();
      break;
    case 'pause':
      if (el.pause) el.pause();
      break;
    case 'toggle':
      el.paused ? (el.play && el.play()) : (el.pause && el.pause());
      break;
    case 'seek':
      el.currentTime = value;
      break;
    case 'seekBy':
      el.currentTime = Math.max(0, (el.currentTime || 0) + value);
      break;
    case 'mute':
      el.muted = !el.muted;
      break;
  }
})()
)JS",
      base::SysNSStringToUTF8(action).c_str(), value);
  return base::SysUTF8ToNSString(script);
}

// Run fixed media-command JavaScript with a synthetic user activation so gated
// media APIs succeed without calling any page-overwritable function.
- (void)runMediaScriptWithUserGesture:(NSString*)source {
  if (!_webContents) {
    return;
  }
  content::RenderFrameHost* frame = _webContents->GetPrimaryMainFrame();
  if (!frame) {
    return;
  }
  frame->ExecuteJavaScriptWithUserGestureForTests(
      base::SysNSStringToUTF16(source), base::BindOnce(^(base::Value) {}),
      kMoriMediaWorldId);
}

- (void)sendMediaCommand:(NSString*)action value:(double)value {
  if (![NSThread isMainThread]) {
    NSString* actionCopy = [action copy];
    dispatch_async(dispatch_get_main_queue(), ^{
      [self sendMediaCommand:actionCopy value:value];
    });
    return;
  }
  if (action.length == 0) {
    return;
  }
  if (!MoriIsAllowedMediaAction(action)) {
    return;
  }
  [self runMediaScriptWithUserGesture:MoriMediaCommandScript(action, value)];
}

- (void)setPageHidden:(BOOL)hidden {
  if (!_webContents) {
    return;
  }
  if (hidden) {
    _webContents->WasHidden();
    // Pop a playing video out to PiP as the tab goes to the background. The
    // synthetic user gesture is what lets this clear the activation gate, so
    // no Chromium auto-PiP feature flag is required.
    if (g_mori_auto_pip) {
      [self runMediaScriptWithUserGesture:MoriMediaCommandScript(@"pipEnter", 0)];
    }
  } else {
    _webContents->WasShown();
    // Returning to the tab brings the video back inline.
    [self runMediaScriptWithUserGesture:MoriMediaCommandScript(@"pipExit", 0)];
  }
}

- (void)setWebWindowVisible:(BOOL)visible {
  _webWindowVisible = visible;
  [self applySuppressionState];
}

- (void)setIgnoresGlobalWebContentSuppression:(BOOL)ignores {
  _ignoresGlobalWebContentSuppression = ignores;
  [self applySuppressionState];
}

- (void)applySuppressionState {
  const BOOL suppressed =
      g_web_content_suppressed && !_ignoresGlobalWebContentSuppression;
  _webView.hidden = !_webWindowVisible || suppressed;
}

- (void)applyAutoPiP:(BOOL)enabled {
  g_mori_auto_pip = enabled;
}

+ (void)setAutoPiPEnabled:(BOOL)enabled {
  g_mori_auto_pip = enabled;
}

+ (void)setAdBlockerEnabled:(BOOL)enabled {
}

+ (BOOL)cancelDownloadWithID:(uint32_t)downloadID {
  content::DownloadManager* manager = mori::MoriDownloadManager();
  if (!manager) {
    return NO;
  }
  if (download::DownloadItem* item = manager->GetDownload(downloadID)) {
    item->Cancel(/*from_user=*/true);
    return YES;
  }
  return NO;
}

+ (void)setWebContentSuppressed:(BOOL)suppressed {
  g_web_content_suppressed = suppressed;
  for (MoriBrowserView* view in [AllViews() copy]) {
    [view applySuppressionState];
  }
}

- (void)closeBrowser {
  if (!_webContents || !g_mori_browser) {
    return;
  }
  TabStripModel* model = g_mori_browser->tab_strip_model();
  const int index = model->GetIndexOfWebContents(_webContents);
  if (index != TabStripModel::kNoTab) {
    model->CloseWebContentsAt(index, TabCloseTypes::CLOSE_USER_GESTURE |
                                         TabCloseTypes::CLOSE_CREATE_HISTORICAL_TAB);
  }
}

@end

// ---------------------------------------------------------------------------
// MoriPrivacy

@implementation MoriPrivacy

+ (content::StoragePartition*)defaultPartition {
  if (!g_mori_browser) {
    return nullptr;
  }
  return g_mori_browser->profile()->GetDefaultStoragePartition();
}

+ (void)clearCookies {
  if (content::StoragePartition* partition = [self defaultPartition]) {
    partition->GetCookieManagerForBrowserProcess()->DeleteCookies(
        network::mojom::CookieDeletionFilter::New(), base::DoNothing());
  }
}

+ (void)clearCache {
  if (content::StoragePartition* partition = [self defaultPartition]) {
    partition->GetNetworkContext()->ClearHttpCache(
        base::Time(), base::Time::Max(), nullptr, base::DoNothing());
  }
}

+ (void)flushCookies {
  if (content::StoragePartition* partition = [self defaultPartition]) {
    partition->GetCookieManagerForBrowserProcess()->FlushCookieStore(
        base::DoNothing());
  }
}

@end
