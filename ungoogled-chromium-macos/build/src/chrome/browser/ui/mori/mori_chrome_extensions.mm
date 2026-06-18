// Swift-facing surface over Chrome's REAL extension system. Mori draws the
// UI; everything else — install state, content scripts, service workers,
// chrome.* APIs, action dispatch, popups, side panels, badges, commands —
// is Chrome's own machinery, so every Chromium extension behaves exactly as
// it does in Chrome.

#import <Cocoa/Cocoa.h>

#include <map>
#include <memory>
#include <optional>
#include <string>
#include <utility>

#include "base/files/file_path.h"
#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/strings/sys_string_conversions.h"
#include "base/values.h"
#include "chrome/browser/extensions/api/side_panel/side_panel_service.h"
#include "chrome/browser/extensions/commands/command_service.h"
#include "chrome/browser/extensions/extension_action_dispatcher.h"
#include "chrome/browser/extensions/extension_action_runner.h"
#include "chrome/browser/extensions/extension_tab_util.h"
#include "chrome/browser/extensions/extension_view.h"
#include "chrome/browser/extensions/extension_view_host.h"
#include "chrome/browser/extensions/extension_view_host_factory.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/browser.h"
#include "chrome/browser/ui/browser_navigator.h"
#include "chrome/browser/ui/browser_navigator_params.h"
#import "chrome/browser/ui/mori/mori_bridge.h"
#include "chrome/browser/ui/mori/mori_chrome_hooks.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/toolbar/toolbar_actions_model.h"
#include "chrome/common/extensions/api/side_panel.h"
#include "chrome/common/extensions/api/tabs.h"
#include "components/sessions/content/session_tab_helper.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/render_frame_host.h"
#include "content/public/browser/render_widget_host_view.h"
#include "content/public/browser/web_contents.h"
#include "extensions/browser/crx_installer.h"
#include "extensions/browser/disable_reason.h"
#include "extensions/browser/event_router.h"
#include "extensions/browser/extension_action.h"
#include "extensions/browser/extension_action_manager.h"
#include "extensions/browser/extension_event_histogram_value.h"
#include "extensions/browser/extension_host.h"
#include "extensions/browser/extension_host_observer.h"
#include "extensions/browser/extension_icon_image.h"
#include "extensions/browser/extension_registrar.h"
#include "extensions/browser/extension_registry.h"
#include "extensions/browser/extension_registry_observer.h"
#include "extensions/browser/extension_system.h"
#include "extensions/browser/install/crx_install_error.h"
#include "extensions/browser/management_policy.h"
#include "extensions/browser/permissions/active_tab_permission_granter.h"
#include "extensions/browser/uninstall_reason.h"
#include "extensions/browser/unpacked_installer.h"
#include "extensions/common/api/extension_action/action_info.h"
#include "extensions/common/command.h"
#include "extensions/common/extension.h"
#include "extensions/common/extension_set.h"
#include "extensions/common/manifest.h"
#include "extensions/common/manifest_handlers/icons_handler.h"
#include "extensions/common/manifest_handlers/options_page_info.h"
#include "extensions/common/manifest_url_handlers.h"
#include "extensions/common/mojom/context_type.mojom.h"
#include "third_party/skia/include/core/SkColor.h"
#include "ui/base/page_transition_types.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "ui/gfx/geometry/size.h"
#include "ui/gfx/image/image.h"
#include "ui/gfx/image/image_skia.h"
#include "url/gurl.h"

namespace {

constexpr int kIconSizeDIP = 32;

NSString* const kExtensionsChanged = @"MoriChromeExtensionsChanged";
NSString* const kSidePanelChanged = @"MoriChromeExtensionSidePanelChanged";
NSString* const kInstallFinished = @"MoriChromeExtensionInstallFinished";

Profile* MoriProfile() {
  Browser* browser = mori::MoriBrowser();
  return browser ? browser->profile() : nullptr;
}

content::WebContents* ActiveWebContents() {
  Browser* browser = mori::MoriBrowser();
  return browser ? browser->tab_strip_model()->GetActiveWebContents()
                 : nullptr;
}

int ActiveTabId() {
  content::WebContents* contents = ActiveWebContents();
  if (!contents) {
    return extensions::ExtensionAction::kDefaultTabId;
  }
  const int id = sessions::SessionTabHelper::IdForTab(contents).id();
  return id >= 0 ? id : extensions::ExtensionAction::kDefaultTabId;
}

void PostNotification(NSString* name, NSDictionary* user_info = nil) {
  [[NSNotificationCenter defaultCenter] postNotificationName:name
                                                      object:nil
                                                    userInfo:user_info];
}

// Coalesces change notifications: many registry/action events can land in one
// runloop turn (e.g. during install), and the Swift side rebuilds its whole
// model per notification.
void PostExtensionsChanged() {
  static BOOL pending = NO;
  if (pending) {
    return;
  }
  pending = YES;
  dispatch_async(dispatch_get_main_queue(), ^{
    pending = NO;
    PostNotification(kExtensionsChanged);
  });
}

NSArray<NSNumber*>* ColorArray(SkColor color) {
  return @[
    @(SkColorGetR(color)), @(SkColorGetG(color)), @(SkColorGetB(color)),
    @(SkColorGetA(color))
  ];
}

NSString* CRXInstallErrorMessage(
    const std::optional<extensions::CrxInstallError>& error) {
  if (!error) {
    return @"";
  }
  NSString* message = base::SysUTF16ToNSString(error->message());
  if (message.length) {
    return message;
  }
  return [NSString stringWithFormat:@"type=%d detail=%d",
                                    static_cast<int>(error->type()),
                                    static_cast<int>(error->detail())];
}

// MARK: - Change observation

// Watches everything that can invalidate the Swift-side extension model:
// install/uninstall/enable/disable (ExtensionRegistry), badge/icon/title
// changes (ExtensionActionDispatcher), pin changes (ToolbarActionsModel),
// and async manifest-icon loads (IconImage).
class MoriExtensionsStateObserver
    : public extensions::ExtensionRegistryObserver,
      public extensions::ExtensionActionDispatcher::Observer,
      public ToolbarActionsModel::Observer,
      public extensions::IconImage::Observer {
 public:
  static MoriExtensionsStateObserver* Ensure(Profile* profile) {
    static MoriExtensionsStateObserver* instance = nullptr;
    if (!instance) {
      instance = new MoriExtensionsStateObserver(profile);
    }
    return instance;
  }

  // The best currently-loaded icon for an extension, preferring the action's
  // dynamic (chrome.action.setIcon) and declarative icons for `tab_id`, then
  // the manifest icon, then the action's default icon.
  NSImage* IconFor(const extensions::Extension& extension,
                   extensions::ExtensionAction* action,
                   int tab_id) {
    if (action) {
      gfx::Image explicit_icon = action->GetExplicitlySetIcon(tab_id);
      if (!explicit_icon.IsEmpty()) {
        return explicit_icon.ToNSImage();
      }
      gfx::Image declarative_icon = action->GetDeclarativeIcon(tab_id);
      if (!declarative_icon.IsEmpty()) {
        return declarative_icon.ToNSImage();
      }
    }
    auto it = icons_.find(extension.id());
    if (it != icons_.end()) {
      gfx::Image manifest_icon = it->second->image();
      if (!manifest_icon.IsEmpty()) {
        return manifest_icon.ToNSImage();
      }
    }
    if (action) {
      gfx::Image default_icon = action->GetDefaultIconImage();
      if (!default_icon.IsEmpty()) {
        return default_icon.ToNSImage();
      }
    }
    return nil;
  }

  // extensions::ExtensionRegistryObserver:
  void OnExtensionLoaded(content::BrowserContext* browser_context,
                         const extensions::Extension* extension) override {
    RebuildIcons();
    PostExtensionsChanged();
  }
  void OnExtensionUnloaded(content::BrowserContext* browser_context,
                           const extensions::Extension* extension,
                           extensions::UnloadedExtensionReason reason) override {
    RebuildIcons();
    PostExtensionsChanged();
  }
  void OnExtensionInstalled(content::BrowserContext* browser_context,
                            const extensions::Extension* extension,
                            bool is_update) override {
    RebuildIcons();
    PostExtensionsChanged();
  }
  void OnExtensionUninstalled(content::BrowserContext* browser_context,
                              const extensions::Extension* extension,
                              extensions::UninstallReason reason) override {
    RebuildIcons();
    PostExtensionsChanged();
  }

  // extensions::ExtensionActionDispatcher::Observer:
  void OnExtensionActionUpdated(
      extensions::ExtensionAction* extension_action,
      content::WebContents* web_contents,
      content::BrowserContext* browser_context) override {
    PostExtensionsChanged();
  }
  void OnShuttingDown() override { dispatcher_observed_ = false; }

  // ToolbarActionsModel::Observer:
  void OnToolbarActionAdded(const ToolbarActionsModel::ActionId& id) override {
    PostExtensionsChanged();
  }
  void OnToolbarActionRemoved(
      const ToolbarActionsModel::ActionId& id) override {
    PostExtensionsChanged();
  }
  void OnToolbarActionUpdated(
      const ToolbarActionsModel::ActionId& id) override {
    PostExtensionsChanged();
  }
  void OnToolbarModelInitialized() override { PostExtensionsChanged(); }
  void OnToolbarPinnedActionsChanged() override { PostExtensionsChanged(); }

  // extensions::IconImage::Observer:
  void OnExtensionIconImageChanged(extensions::IconImage* image) override {
    PostExtensionsChanged();
  }

 private:
  explicit MoriExtensionsStateObserver(Profile* profile) : profile_(profile) {
    extensions::ExtensionRegistry::Get(profile_)->AddObserver(this);
    extensions::ExtensionActionDispatcher::Get(profile_)->AddObserver(this);
    dispatcher_observed_ = true;
    if (ToolbarActionsModel* model = ToolbarActionsModel::Get(profile_)) {
      model->AddObserver(this);
    }
    RebuildIcons();
  }

  ~MoriExtensionsStateObserver() override = default;

  // Keeps one async-loading manifest icon per installed extension. IconImage
  // self-invalidates when its extension unloads, so the map is rebuilt on
  // every registry change.
  void RebuildIcons() {
    auto* registry = extensions::ExtensionRegistry::Get(profile_);
    std::map<std::string, std::unique_ptr<extensions::IconImage>> next;
    for (const extensions::ExtensionSet* set :
         {&registry->enabled_extensions(), &registry->disabled_extensions()}) {
      for (const auto& extension : *set) {
        if (!extension->is_extension()) {
          continue;
        }
        auto existing = icons_.find(extension->id());
        if (existing != icons_.end() &&
            existing->second->is_valid()) {
          next[extension->id()] = std::move(existing->second);
          continue;
        }
        next[extension->id()] = std::make_unique<extensions::IconImage>(
            profile_, extension.get(),
            extensions::IconsInfo::GetIcons(extension.get()), kIconSizeDIP,
            gfx::ImageSkia(), this);
      }
    }
    icons_ = std::move(next);
  }

  raw_ptr<Profile> profile_;
  bool dispatcher_observed_ = false;
  std::map<std::string, std::unique_ptr<extensions::IconImage>> icons_;
};

// MARK: - Popup hosting

constexpr gfx::Size kPopupMinSize = {25, 25};
constexpr gfx::Size kPopupMaxSize = {800, 600};

// Hosts an ExtensionViewHost popup in a floating NSPanel, playing the role
// views' ExtensionPopup does in stock Chrome: starts the renderer, enables
// content-driven auto-resize, shows the panel once loaded, and closes on
// Escape / focus loss / host teardown (window.close()).
class MoriExtensionPopup : public extensions::ExtensionView,
                           public extensions::ExtensionHostObserver {
 public:
  static MoriExtensionPopup* Shared() {
    static MoriExtensionPopup* instance = new MoriExtensionPopup();
    return instance;
  }

  void Show(std::unique_ptr<extensions::ExtensionViewHost> host,
            NSRect anchor) {
    Close();
    host_ = std::move(host);
    anchor_ = anchor;
    host_->AddObserver(this);
    host_->set_view(this);
    // window.close() / Escape land here via ExtensionHost::Close(). Hop the
    // runloop so the host isn't destroyed re-entrantly from its own method.
    host_->SetCloseHandler(base::BindOnce(^(extensions::ExtensionHost* h) {
      dispatch_async(dispatch_get_main_queue(), ^{
        MoriExtensionPopup::Shared()->Close();
      });
    }));

    NSView* contents_view =
        host_->host_contents()->GetNativeView().GetNativeNSView();
    NSPanel* panel = [[NSPanel alloc]
        initWithContentRect:NSMakeRect(0, 0, 360, 320)
                  styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                            NSWindowStyleMaskNonactivatingPanel |
                            NSWindowStyleMaskFullSizeContentView
                    backing:NSBackingStoreBuffered
                      defer:NO];
    panel.titlebarAppearsTransparent = YES;
    panel.titleVisibility = NSWindowTitleHidden;
    panel.releasedWhenClosed = NO;
    panel.floatingPanel = YES;
    contents_view.frame = panel.contentView.bounds;
    contents_view.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    [panel.contentView addSubview:contents_view];
    panel_ = panel;
    Reposition();

    // Dismiss when the panel loses key (click elsewhere), like Chrome popups.
    resign_observer_ = [[NSNotificationCenter defaultCenter]
        addObserverForName:NSWindowDidResignKeyNotification
                    object:panel
                     queue:[NSOperationQueue mainQueue]
                usingBlock:^(NSNotification* note) {
                  MoriExtensionPopup::Shared()->Close();
                }];

    host_->CreateRendererSoon();
  }

  void Close() {
    if (resign_observer_) {
      [[NSNotificationCenter defaultCenter] removeObserver:resign_observer_];
      resign_observer_ = nil;
    }
    if (panel_) {
      [panel_ orderOut:nil];
      panel_ = nil;
    }
    if (host_) {
      host_->RemoveObserver(this);
      host_.reset();
    }
  }

  // extensions::ExtensionView:
  gfx::NativeView GetNativeView() override {
    return host_->host_contents()->GetNativeView();
  }
  void ResizeDueToAutoResize(content::WebContents* web_contents,
                             const gfx::Size& new_size) override {
    if (!panel_) {
      return;
    }
    gfx::Size size = new_size;
    size.SetToMax(kPopupMinSize);
    size.SetToMin(kPopupMaxSize);
    [panel_ setContentSize:NSMakeSize(size.width(), size.height())];
    Reposition();
  }
  void RenderFrameCreated(content::RenderFrameHost* render_frame_host) override {
    if (auto* view = render_frame_host->GetView()) {
      view->EnableAutoResize(kPopupMinSize, kPopupMaxSize);
    }
  }
  bool HandleKeyboardEvent(content::WebContents* source,
                           const input::NativeWebKeyboardEvent& event) override {
    return false;
  }
  void OnLoaded() override {
    if (panel_) {
      [panel_ makeKeyAndOrderFront:nil];
    }
  }

  // extensions::ExtensionHostObserver:
  void OnExtensionHostDestroyed(extensions::ExtensionHost* host) override {
    // The host tears itself down (e.g. the popup called window.close(), or
    // the extension was unloaded); drop the panel without re-entering reset.
    if (resign_observer_) {
      [[NSNotificationCenter defaultCenter] removeObserver:resign_observer_];
      resign_observer_ = nil;
    }
    if (panel_) {
      [panel_ orderOut:nil];
      panel_ = nil;
    }
    if (host_ && host_.get() == host) {
      host_.release();  // Owner is destroying it; don't double-delete.
    }
  }

 private:
  MoriExtensionPopup() = default;
  ~MoriExtensionPopup() override = default;

  void Reposition() {
    if (!panel_) {
      return;
    }
    const NSSize size = [panel_ contentRectForFrameRect:panel_.frame].size;
    NSPoint top_left;
    if (NSIsEmptyRect(anchor_)) {
      NSWindow* window = mori::MoriMainWindow();
      const NSRect wf = window ? window.frame : NSScreen.mainScreen.visibleFrame;
      top_left = NSMakePoint(NSMaxX(wf) - size.width - 24, NSMaxY(wf) - 96);
    } else {
      // Below the anchor, right-aligned to it (Chrome aligns popups to the
      // trailing edge of the action button).
      top_left = NSMakePoint(NSMaxX(anchor_) - size.width, NSMinY(anchor_) - 6);
    }
    if (NSScreen* screen = NSScreen.mainScreen) {
      const NSRect vf = screen.visibleFrame;
      top_left.x = MAX(NSMinX(vf) + 8, MIN(top_left.x, NSMaxX(vf) - size.width - 8));
      top_left.y = MAX(NSMinY(vf) + size.height + 8, MIN(top_left.y, NSMaxY(vf) - 8));
    }
    [panel_ setFrameTopLeftPoint:top_left];
  }

  std::unique_ptr<extensions::ExtensionViewHost> host_;
  NSPanel* __strong panel_ = nil;
  id __strong resign_observer_ = nil;
  NSRect anchor_ = NSZeroRect;
};

// MARK: - Side panel hosting

// Hosts an extension side panel's ExtensionViewHost. Mori's SwiftUI side
// panel chrome embeds `view()`.
class MoriExtensionSidePanel : public extensions::ExtensionView,
                               public extensions::ExtensionHostObserver {
 public:
  static MoriExtensionSidePanel* Shared() {
    static MoriExtensionSidePanel* instance = new MoriExtensionSidePanel();
    return instance;
  }

  void Show(std::unique_ptr<extensions::ExtensionViewHost> host,
            const std::string& extension_id,
            const std::string& title) {
    Close();
    host_ = std::move(host);
    extension_id_ = extension_id;
    title_ = title;
    host_->AddObserver(this);
    host_->set_view(this);
    host_->SetCloseHandler(base::BindOnce(^(extensions::ExtensionHost* h) {
      dispatch_async(dispatch_get_main_queue(), ^{
        MoriExtensionSidePanel::Shared()->Close();
      });
    }));
    host_->CreateRendererSoon();
    PostNotification(kSidePanelChanged);
  }

  void Close() {
    if (!host_) {
      return;
    }
    host_->RemoveObserver(this);
    host_.reset();
    extension_id_.clear();
    title_.clear();
    PostNotification(kSidePanelChanged);
  }

  bool IsOpenFor(const std::string& extension_id) const {
    return host_ && extension_id_ == extension_id;
  }
  NSView* view() const {
    return host_ ? host_->host_contents()->GetNativeView().GetNativeNSView()
                 : nil;
  }
  NSString* extension_id() const {
    return host_ ? base::SysUTF8ToNSString(extension_id_) : nil;
  }
  NSString* title() const {
    return host_ ? base::SysUTF8ToNSString(title_) : nil;
  }

  // extensions::ExtensionView:
  gfx::NativeView GetNativeView() override {
    return host_->host_contents()->GetNativeView();
  }
  void ResizeDueToAutoResize(content::WebContents* web_contents,
                             const gfx::Size& new_size) override {}
  void RenderFrameCreated(content::RenderFrameHost* render_frame_host) override {}
  bool HandleKeyboardEvent(content::WebContents* source,
                           const input::NativeWebKeyboardEvent& event) override {
    return false;
  }
  void OnLoaded() override {}

  // extensions::ExtensionHostObserver:
  void OnExtensionHostDestroyed(extensions::ExtensionHost* host) override {
    if (host_ && host_.get() == host) {
      host_.release();  // Owner is destroying it; don't double-delete.
      extension_id_.clear();
      title_.clear();
      PostNotification(kSidePanelChanged);
    }
  }

 private:
  MoriExtensionSidePanel() = default;
  ~MoriExtensionSidePanel() override = default;

  std::unique_ptr<extensions::ExtensionViewHost> host_;
  std::string extension_id_;
  std::string title_;
};

// Inserted unowned → the Mori tab-strip observer adopts it as a new tab.
void OpenInNewTab(const GURL& url) {
  Browser* browser = mori::MoriBrowser();
  if (!browser) {
    return;
  }
  NavigateParams params(browser, url, ui::PAGE_TRANSITION_TYPED);
  params.disposition = WindowOpenDisposition::NEW_BACKGROUND_TAB;
  params.window_action = NavigateParams::WindowAction::kNoAction;
  Navigate(&params);
}

NSString* InstallTypeString(const extensions::Extension& extension) {
  if (extensions::Manifest::IsUnpackedLocation(extension.location())) {
    return @"development";
  }
  if (extension.location() == extensions::mojom::ManifestLocation::kInternal) {
    return @"normal";
  }
  return @"other";
}

}  // namespace

@implementation MoriChromeExtensions

+ (NSArray<NSDictionary*>*)installedExtensions {
  Profile* profile = MoriProfile();
  if (!profile) {
    return @[];
  }
  auto* observer = MoriExtensionsStateObserver::Ensure(profile);
  auto* registry = extensions::ExtensionRegistry::Get(profile);
  auto* action_manager = extensions::ExtensionActionManager::Get(profile);
  auto* policy = extensions::ExtensionSystem::Get(profile)->management_policy();
  ToolbarActionsModel* toolbar_model = ToolbarActionsModel::Get(profile);
  const int tab_id = ActiveTabId();

  NSMutableArray<NSDictionary*>* result = [NSMutableArray array];
  struct Entry {
    const extensions::Extension* extension;
    bool enabled;
  };
  std::map<std::string, Entry> by_id;
  for (const auto& extension : registry->enabled_extensions()) {
    by_id[extension->id()] = {extension.get(), true};
  }
  for (const auto& extension : registry->disabled_extensions()) {
    by_id[extension->id()] = {extension.get(), false};
  }

  for (const auto& [extension_id, entry] : by_id) {
    const extensions::Extension& extension = *entry.extension;
    if (!extension.is_extension() ||
        extensions::Manifest::IsComponentLocation(extension.location())) {
      continue;
    }

    extensions::ExtensionAction* action =
        action_manager->GetExtensionAction(extension);
    const bool may_disable =
        !policy || policy->UserMayModifySettings(&extension, nullptr);
    const bool pinned =
        toolbar_model && toolbar_model->IsActionPinned(extension.id());
    const GURL homepage = extensions::ManifestURL::GetHomepageURL(&extension);

    NSMutableDictionary* info = [@{
      @"id" : base::SysUTF8ToNSString(extension.id()),
      @"name" : base::SysUTF8ToNSString(extension.name()),
      @"shortName" : base::SysUTF8ToNSString(extension.short_name()),
      @"description" : base::SysUTF8ToNSString(extension.description()),
      @"version" : base::SysUTF8ToNSString(extension.VersionString()),
      @"enabled" : @(entry.enabled),
      @"pinned" : @(pinned),
      @"mayDisable" : @(may_disable),
      @"hasOptionsPage" :
          @(extensions::OptionsPageInfo::HasOptionsPage(&extension)),
      @"hasPopup" : @(action && action->HasPopup(tab_id)),
      @"badgeText" : action ? base::SysUTF8ToNSString(
                                  action->GetDisplayBadgeText(tab_id))
                            : @"",
      @"actionTitle" :
          action ? base::SysUTF8ToNSString(action->GetTitle(tab_id)) : @"",
      @"badgeBackgroundColor" :
          ColorArray(action ? action->GetBadgeBackgroundColor(tab_id)
                            : SkColorSetRGB(0xd9, 0x30, 0x25)),
      @"badgeTextColor" : ColorArray(
          action ? action->GetBadgeTextColor(tab_id) : SK_ColorWHITE),
      @"homepageURL" : base::SysUTF8ToNSString(homepage.spec()),
      @"webStoreURL" : [NSString
          stringWithFormat:@"https://chromewebstore.google.com/detail/%s",
                           extension.id().c_str()],
      @"installType" : InstallTypeString(extension),
    } mutableCopy];
    if (NSImage* icon = observer->IconFor(extension, action, tab_id)) {
      info[@"icon"] = icon;
    }
    [result addObject:info];
  }

  [result sortUsingComparator:^NSComparisonResult(NSDictionary* a,
                                                  NSDictionary* b) {
    return [a[@"name"] localizedCaseInsensitiveCompare:b[@"name"]];
  }];
  return result;
}

+ (void)runActionWithId:(NSString*)extensionId anchorRect:(NSRect)anchor {
  Browser* browser = mori::MoriBrowser();
  Profile* profile = MoriProfile();
  if (!browser || !profile) {
    return;
  }
  const extensions::Extension* extension =
      extensions::ExtensionRegistry::Get(profile)
          ->enabled_extensions()
          .GetByID(base::SysNSStringToUTF8(extensionId));
  if (!extension) {
    return;
  }
  content::WebContents* contents = ActiveWebContents();
  if (!contents) {
    return;
  }
  auto* runner = extensions::ExtensionActionRunner::GetForWebContents(contents);
  if (!runner) {
    return;
  }

  switch (runner->RunAction(extension, /*grant_tab_permissions=*/true)) {
    case extensions::ExtensionAction::ShowAction::kNone:
      break;
    case extensions::ExtensionAction::ShowAction::kShowPopup: {
      extensions::ExtensionAction* action =
          extensions::ExtensionActionManager::Get(profile)->GetExtensionAction(
              *extension);
      if (!action) {
        break;
      }
      const GURL popup_url = action->GetPopupUrl(ActiveTabId());
      if (!popup_url.is_valid()) {
        break;
      }
      auto host = extensions::ExtensionViewHostFactory::CreatePopupHost(
          *extension, popup_url, browser);
      if (host) {
        MoriExtensionPopup::Shared()->Show(std::move(host), anchor);
      }
      break;
    }
    case extensions::ExtensionAction::ShowAction::kToggleSidePanel:
      [self toggleSidePanelForId:extensionId];
      break;
  }
}

+ (void)setExtensionWithId:(NSString*)extensionId enabled:(BOOL)enabled {
  Profile* profile = MoriProfile();
  if (!profile) {
    return;
  }
  auto* registrar = extensions::ExtensionRegistrar::Get(profile);
  const std::string extension_id = base::SysNSStringToUTF8(extensionId);
  if (enabled) {
    registrar->EnableExtension(extension_id);
  } else {
    registrar->DisableExtension(
        extension_id, {extensions::disable_reason::DISABLE_USER_ACTION});
  }
}

+ (BOOL)uninstallExtensionWithId:(NSString*)extensionId {
  Profile* profile = MoriProfile();
  if (!profile) {
    return NO;
  }
  std::u16string error;
  const bool ok =
      extensions::ExtensionRegistrar::Get(profile)->UninstallExtension(
          base::SysNSStringToUTF8(extensionId),
          extensions::UNINSTALL_REASON_USER_INITIATED, &error);
  if (!ok) {
    NSLog(@"MORI extension uninstall failed: %@",
          base::SysUTF16ToNSString(error));
  }
  return ok;
}

+ (void)setExtensionWithId:(NSString*)extensionId pinned:(BOOL)pinned {
  Profile* profile = MoriProfile();
  if (!profile) {
    return;
  }
  ToolbarActionsModel* model = ToolbarActionsModel::Get(profile);
  const std::string action_id = base::SysNSStringToUTF8(extensionId);
  if (model && model->HasAction(action_id)) {
    model->SetActionVisibility(action_id, pinned);
  }
}

+ (BOOL)openOptionsPageForId:(NSString*)extensionId {
  Browser* browser = mori::MoriBrowser();
  Profile* profile = MoriProfile();
  if (!browser || !profile) {
    return NO;
  }
  const extensions::Extension* extension =
      extensions::ExtensionRegistry::Get(profile)
          ->GetInstalledExtension(base::SysNSStringToUTF8(extensionId));
  if (!extension) {
    return NO;
  }
  return extensions::ExtensionTabUtil::OpenOptionsPage(extension, browser);
}

+ (void)openExtensionsPage {
  OpenInNewTab(GURL("chrome://extensions"));
}

+ (void)openExtensionsPageForId:(NSString*)extensionId {
  OpenInNewTab(
      GURL("chrome://extensions/?id=" + base::SysNSStringToUTF8(extensionId)));
}

+ (BOOL)installCRXAtPath:(NSString*)path expectedId:(NSString*)extensionId {
  Profile* profile = MoriProfile();
  if (!profile || path.length == 0) {
    return NO;
  }

  auto installer = extensions::CrxInstaller::CreateSilent(profile);
  // Mori obtains explicit install consent in Swift before staging the CRX.
  // Keep Chrome's host/API permission grants withheld until the user grants
  // them through Chromium's extension UI.
  installer->set_allow_silent_install(false);
  installer->set_grant_permissions(false);
  installer->set_is_gallery_install(true);
  installer->set_delete_source(true);
  installer->set_off_store_install_allow_reason(
      extensions::CrxInstaller::OffStoreInstallAllowedFromSettingsPage);
  if (extensionId.length > 0) {
    installer->set_expected_id(base::SysNSStringToUTF8(extensionId));
  }
  installer->AddInstallerCallback(base::BindOnce(
      ^(const std::optional<extensions::CrxInstallError>& error) {
        NSString* message = CRXInstallErrorMessage(error);
        if (error) {
          NSLog(@"MORI extension CRX install failed: %@", message);
        }
        PostNotification(kInstallFinished, @{
          @"ok" : @(!error),
          @"error" : message,
        });
        PostExtensionsChanged();
      }));
  installer->InstallCrx(base::FilePath(base::SysNSStringToUTF8(path)));
  return YES;
}

+ (BOOL)loadUnpackedExtensionAtPath:(NSString*)path {
  Profile* profile = MoriProfile();
  if (!profile || path.length == 0) {
    return NO;
  }

  auto installer = extensions::UnpackedInstaller::Create(profile);
  installer->set_be_noisy_on_failure(true);
  installer->set_completion_callback(base::BindOnce(
      ^(const extensions::Extension* extension,
        const base::FilePath& extension_path, const std::u16string& error) {
        if (!extension) {
          NSLog(@"MORI unpacked extension load failed: %@",
                base::SysUTF16ToNSString(error));
        }
        PostNotification(kInstallFinished, @{
          @"ok" : @(extension != nullptr),
          @"error" : base::SysUTF16ToNSString(error),
        });
        PostExtensionsChanged();
      }));
  installer->Load(base::FilePath(base::SysNSStringToUTF8(path)));
  return YES;
}

+ (NSArray<NSDictionary*>*)commands {
  Profile* profile = MoriProfile();
  if (!profile) {
    return @[];
  }
  auto* registry = extensions::ExtensionRegistry::Get(profile);
  auto* command_service = extensions::CommandService::Get(profile);

  NSMutableArray<NSDictionary*>* result = [NSMutableArray array];
  auto append = ^(const extensions::Extension& extension,
                  const ui::Command& command, bool is_action) {
    if (command.accelerator().key_code() == ui::VKEY_UNKNOWN) {
      return;
    }
    [result addObject:@{
      @"extensionId" : base::SysUTF8ToNSString(extension.id()),
      @"extensionName" : base::SysUTF8ToNSString(extension.name()),
      @"commandName" : base::SysUTF8ToNSString(command.command_name()),
      @"description" : base::SysUTF16ToNSString(command.description()),
      @"shortcut" : base::SysUTF8ToNSString(
          ui::Command::AcceleratorToString(command.accelerator())),
      @"isAction" : @(is_action),
    }];
  };

  for (const auto& extension : registry->enabled_extensions()) {
    if (!extension->is_extension()) {
      continue;
    }
    ui::CommandMap named;
    if (command_service->GetNamedCommands(
            extension->id(), extensions::CommandService::ACTIVE,
            extensions::CommandService::ANY_SCOPE, &named)) {
      for (const auto& [name, command] : named) {
        append(*extension, command, false);
      }
    }
    if (const extensions::ActionInfo* action_info =
            extensions::ActionInfo::GetExtensionActionInfo(extension.get())) {
      extensions::Command action_command;
      if (command_service->GetExtensionActionCommand(
              extension->id(), action_info->type,
              extensions::CommandService::ACTIVE, &action_command,
              /*active=*/nullptr)) {
        append(*extension, action_command, true);
      }
    }
  }
  return result;
}

+ (void)dispatchCommand:(NSString*)commandName
         forExtensionId:(NSString*)extensionId {
  Profile* profile = MoriProfile();
  if (!profile) {
    return;
  }
  const extensions::Extension* extension =
      extensions::ExtensionRegistry::Get(profile)
          ->enabled_extensions()
          .GetByID(base::SysNSStringToUTF8(extensionId));
  if (!extension) {
    return;
  }

  // Mirrors ExtensionKeybindingRegistry::CommandExecuted: grant activeTab,
  // then fire commands.onCommand with the active tab attached.
  content::WebContents* contents = ActiveWebContents();
  if (contents) {
    if (auto* granter =
            extensions::ActiveTabPermissionGranter::FromWebContents(contents)) {
      granter->GrantIfRequested(extension);
    }
  }

  base::ListValue args;
  args.Append(base::SysNSStringToUTF8(commandName));
  base::Value tab_value;
  if (contents) {
    constexpr extensions::mojom::ContextType context_type =
        extensions::mojom::ContextType::kPrivilegedExtension;
    extensions::ExtensionTabUtil::ScrubTabBehavior scrub_tab_behavior =
        extensions::ExtensionTabUtil::GetScrubTabBehavior(
            extension, context_type, contents);
    tab_value = base::Value(extensions::ExtensionTabUtil::CreateTabObject(
                                contents, scrub_tab_behavior, extension)
                                .ToValue());
  }
  args.Append(std::move(tab_value));

  auto event = std::make_unique<extensions::Event>(
      extensions::events::COMMANDS_ON_COMMAND, "commands.onCommand",
      std::move(args), profile);
  event->user_gesture = extensions::EventRouter::UserGestureState::kEnabled;
  extensions::EventRouter::Get(profile)->DispatchEventToExtension(
      extension->id(), std::move(event));
}

+ (NSView*)sidePanelView {
  return MoriExtensionSidePanel::Shared()->view();
}

+ (NSString*)sidePanelExtensionId {
  return MoriExtensionSidePanel::Shared()->extension_id();
}

+ (NSString*)sidePanelTitle {
  return MoriExtensionSidePanel::Shared()->title();
}

+ (BOOL)toggleSidePanelForId:(NSString*)extensionId {
  Browser* browser = mori::MoriBrowser();
  Profile* profile = MoriProfile();
  if (!browser || !profile) {
    return NO;
  }
  const std::string extension_id = base::SysNSStringToUTF8(extensionId);
  auto* side_panel = MoriExtensionSidePanel::Shared();
  if (side_panel->IsOpenFor(extension_id)) {
    side_panel->Close();
    return YES;
  }

  const extensions::Extension* extension =
      extensions::ExtensionRegistry::Get(profile)
          ->enabled_extensions()
          .GetByID(extension_id);
  if (!extension) {
    return NO;
  }
  content::WebContents* contents = ActiveWebContents();
  if (!contents) {
    return NO;
  }
  auto* service = extensions::SidePanelService::Get(profile);
  if (!service) {
    return NO;
  }
  const int tab_id = sessions::SessionTabHelper::IdForTab(contents).id();
  extensions::api::side_panel::PanelOptions options = service->GetOptions(
      *extension,
      tab_id >= 0 ? std::optional<int>(tab_id) : std::nullopt);
  if (!options.path || (options.enabled && !*options.enabled)) {
    return NO;
  }
  tabs::TabInterface* tab = tabs::TabInterface::MaybeGetFromContents(contents);
  if (!tab) {
    return NO;
  }
  auto host = extensions::ExtensionViewHostFactory::CreateSidePanelHost(
      *extension, extension->ResolveExtensionURL(*options.path), browser, tab);
  if (!host) {
    return NO;
  }
  side_panel->Show(std::move(host), extension->id(), extension->name());
  return YES;
}

+ (void)closeSidePanel {
  MoriExtensionSidePanel::Shared()->Close();
}

+ (void)closePopup {
  MoriExtensionPopup::Shared()->Close();
}

@end
