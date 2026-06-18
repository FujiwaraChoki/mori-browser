// Mori BrowserWindow: mostly inert (SwiftUI owns the chrome); window-level
// queries answer against the shared Mori NSWindow.

#include "chrome/browser/ui/mori/mori_browser_window.h"

#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>

#include "base/strings/sys_string_conversions.h"
#include "chrome/browser/ui/browser.h"
#include "chrome/browser/ui/mori/mori_chrome_hooks.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/exclusive_access/exclusive_access_bubble_type.h"
#include "chrome/browser/ui/views/bubble_anchor_util_views.h"
#include "components/input/native_web_keyboard_event.h"
#include "components/sharing_message/sharing_dialog_data.h"
#include "components/web_modal/modal_dialog_host.h"
#include "content/public/browser/keyboard_event_processing_result.h"
#include "ui/base/mojom/window_show_state.mojom.h"
#include "ui/gfx/range/range.h"

// Swift-exported surface of MoriRoot (see MoriRoot.swift); declared locally so
// this window can route web-content key events through the same shortcut
// registry the app-level NSEvent monitor uses, without pulling in the bridge.
@interface MoriRoot : NSObject
+ (BOOL)handleShortcutEvent:(NSEvent*)event;
@end

namespace {

// Mori's chrome shortcuts (⌘S toggle sidebar, ⌘T toggle omnibox, …) belong to
// the SwiftUI registry. Claim them here — in the browser's keyboard pre-handler
// — so they win *before* the focused web page or Chromium's own commands
// (Save Page As on ⌘S, New Tab on ⌘T) can act. This is what makes the
// shortcuts fire reliably while web content has focus, instead of racing the
// app-level NSEvent monitor and Chromium's native accelerators (the
// intermittent "needs two presses" behavior).
bool HandleMoriShortcut(const input::NativeWebKeyboardEvent& event) {
  // Only fire on the raw key-down; ignore synthesized char and key-up events.
  if (event.GetType() != input::NativeWebKeyboardEvent::Type::kRawKeyDown) {
    return false;
  }
  NSEvent* ns_event = event.os_event.Get();
  if (!ns_event || ns_event.type != NSEventTypeKeyDown) {
    return false;
  }
  return [MoriRoot handleShortcutEvent:ns_event] == YES;
}

NSString* OriginDisclosureLabel(const url::Origin& origin) {
  const std::string serialized_origin = origin.Serialize();
  if (serialized_origin.empty() || serialized_origin == "null") {
    return @"Mori Browser";
  }
  return base::SysUTF8ToNSString(serialized_origin);
}

void ConfigureDisclosureLabel(NSTextField* label,
                              NSFont* font,
                              NSColor* color) {
  label.font = font;
  label.textColor = color;
  label.backgroundColor = NSColor.clearColor;
  label.bordered = NO;
  label.editable = NO;
  label.selectable = NO;
  label.lineBreakMode = NSLineBreakByTruncatingMiddle;
}

}  // namespace

MoriBrowserWindow::MoriBrowserWindow(Browser* browser)
    : browser_(browser), exclusive_access_context_(browser) {
  mori::OnBrowserWindowCreated(browser);
}

// --- MoriFindBar --------------------------------------------------------------

FindBarController* MoriFindBar::GetFindBarController() const {
  return nullptr;
}

void MoriFindBar::SetFindBarController(FindBarController* find_bar_controller) {}

void MoriFindBar::Show(bool animate, bool focus) {}

void MoriFindBar::Hide(bool animate) {}

void MoriFindBar::SetFocusAndSelection() {}

void MoriFindBar::ClearResults( const find_in_page::FindNotificationDetails& results) {}

void MoriFindBar::StopAnimation() {}

void MoriFindBar::MoveWindowIfNecessary() {}

void MoriFindBar::SetFindTextAndSelectedRange( const std::u16string& find_text, const gfx::Range& selected_range) {}

std::u16string_view MoriFindBar::GetFindText() const {
  return {};
}

gfx::Range MoriFindBar::GetSelectedRange() const {
  return {};
}

void MoriFindBar::UpdateUIForFindResult( const find_in_page::FindNotificationDetails& result, const std::u16string& find_text) {}

void MoriFindBar::AudibleAlert() {}

bool MoriFindBar::IsFindBarVisible() const {
  return false;
}

void MoriFindBar::RestoreSavedFocus() {}

bool MoriFindBar::HasGlobalFindPasteboard() const {
  return false;
}

void MoriFindBar::UpdateFindBarForChangedWebContents() {}

bool MoriFindBar::CanPopulateFromSelectedText() {
  return false;
}

const FindBarTesting* MoriFindBar::GetFindBarTesting() const {
  return nullptr;
}

bool MoriFindBar::HasFocus() const {
  return false;
}

void MoriFindBar::CloseOverlappingBubbles() {}

views::Widget* MoriFindBar::GetHostWidget() {
  return nullptr;
}

// --- MoriLocationBar ---------------------------------------------------------

void MoriLocationBar::FocusLocation(bool is_user_initiated, bool clear_focus_if_failed) {}

void MoriLocationBar::FocusSearch() {}

void MoriLocationBar::UpdateFocusBehavior(bool toolbar_visible) {}

void MoriLocationBar::UpdateContentSettingsIcons() {}

void MoriLocationBar::SaveStateToContents(content::WebContents* contents) {}

void MoriLocationBar::Revert() {}

OmniboxView* MoriLocationBar::GetOmniboxView() {
  return nullptr;
}

OmniboxController* MoriLocationBar::GetOmniboxController() {
  return nullptr;
}

bool MoriLocationBar::ShouldCloseOmniboxPopup(ui::MouseEvent* event) {
  return false;
}

content::WebContents* MoriLocationBar::GetWebContents() {
  return nullptr;
}

LocationBarModel* MoriLocationBar::GetLocationBarModel() {
  return nullptr;
}

std::optional<bubble_anchor_util::AnchorConfiguration> MoriLocationBar::GetChipAnchor() {
  return {};
}

ChipController* MoriLocationBar::GetChipController() {
  return nullptr;
}

void MoriLocationBar::OnChanged() {}

void MoriLocationBar::UpdateWithoutTabRestore() {}

ui::TrackedElement* MoriLocationBar::GetAnchorOrNull() {
  return nullptr;
}

Browser* MoriLocationBar::GetBrowser() {
  return nullptr;
}

Profile* MoriLocationBar::GetProfile() {
  return nullptr;
}

bool MoriLocationBar::IsInitialized() const {
  return false;
}

bool MoriLocationBar::IsVisible() const {
  return false;
}

bool MoriLocationBar::IsDrawn() const {
  return false;
}

bool MoriLocationBar::IsFullscreen() const {
  return false;
}

bool MoriLocationBar::IsEditingOrEmpty() const {
  return false;
}

void MoriLocationBar::InvalidateLayout() {}

gfx::Rect MoriLocationBar::Bounds() const {
  return {};
}

gfx::Rect MoriLocationBar::BoundsInScreen() const {
  return {};
}

gfx::Size MoriLocationBar::MinimumSize() const {
  return {};
}

gfx::Size MoriLocationBar::PreferredSize() const {
  return {};
}

void MoriLocationBar::Update(content::WebContents* contents) {}

void MoriLocationBar::ResetTabState(content::WebContents* contents) {}

bool MoriLocationBar::HasSecurityStateChanged() {
  return false;
}

LocationBarTesting* MoriLocationBar::GetLocationBarForTesting() {
  return nullptr;
}

// --- MoriExclusiveAccessContext ---------------------------------------------

MoriExclusiveAccessContext::MoriExclusiveAccessContext(Browser* browser)
    : browser_(browser) {}

MoriExclusiveAccessContext::~MoriExclusiveAccessContext() {
  HideFullscreenDisclosure(ExclusiveAccessBubbleHideReason::kInterrupted);
}

Profile* MoriExclusiveAccessContext::GetProfile() {
  return browser_->profile();
}

bool MoriExclusiveAccessContext::IsFullscreen() const {
  NSWindow* window = mori::MoriMainWindow();
  return window && (window.styleMask & NSWindowStyleMaskFullScreen);
}

void MoriExclusiveAccessContext::EnterFullscreen(
    const url::Origin& origin,
    ExclusiveAccessBubbleType bubble_type,
    FullscreenTabParams fullscreen_tab_params) {
  NSWindow* window = mori::MoriMainWindow();
  if (window && !(window.styleMask & NSWindowStyleMaskFullScreen)) {
    [window toggleFullScreen:nil];
  }
  ShowFullscreenDisclosure(origin);
}

void MoriExclusiveAccessContext::ExitFullscreen() {
  HideFullscreenDisclosure(ExclusiveAccessBubbleHideReason::kInterrupted);
  NSWindow* window = mori::MoriMainWindow();
  if (window && (window.styleMask & NSWindowStyleMaskFullScreen)) {
    [window toggleFullScreen:nil];
  }
}

void MoriExclusiveAccessContext::UpdateExclusiveAccessBubble(
    const ExclusiveAccessBubbleParams& params,
    ExclusiveAccessBubbleHideCallback first_hide_callback) {
  const bool should_close_bubble =
      !params.has_download &&
      params.type == EXCLUSIVE_ACCESS_BUBBLE_TYPE_NONE;
  if (should_close_bubble) {
    if (first_hide_callback) {
      std::move(first_hide_callback)
          .Run(ExclusiveAccessBubbleHideReason::kNotShown);
    }
    HideFullscreenDisclosure(ExclusiveAccessBubbleHideReason::kInterrupted);
    return;
  }
  ShowFullscreenDisclosure(params.origin, std::move(first_hide_callback));
}

bool MoriExclusiveAccessContext::IsExclusiveAccessBubbleDisplayed() const {
  return exclusive_access_bubble_visible_;
}

void MoriExclusiveAccessContext::OnExclusiveAccessUserInput() {
  NSPanel* panel = (__bridge NSPanel*)fullscreen_disclosure_;
  if (panel) {
    [panel orderFront:nil];
  }
}

content::WebContents* MoriExclusiveAccessContext::GetWebContentsForExclusiveAccess() {
  return browser_->tab_strip_model()->GetActiveWebContents();
}

bool MoriExclusiveAccessContext::CanUserEnterFullscreen() const {
  return true;
}

bool MoriExclusiveAccessContext::CanUserExitFullscreen() const {
  return true;
}

void MoriExclusiveAccessContext::ShowFullscreenDisclosure(
    const url::Origin& origin,
    ExclusiveAccessBubbleHideCallback first_hide_callback) {
  NSWindow* parent = mori::MoriMainWindow();
  if (!parent) {
    if (first_hide_callback) {
      std::move(first_hide_callback)
          .Run(ExclusiveAccessBubbleHideReason::kNotShown);
    }
    return;
  }

  HideFullscreenDisclosure(ExclusiveAccessBubbleHideReason::kInterrupted);

  const NSRect parent_frame = parent.frame;
  const CGFloat width = std::min<CGFloat>(
      520.0, std::max<CGFloat>(320.0, parent_frame.size.width - 48.0));
  const CGFloat height = 72.0;
  const NSRect frame = NSMakeRect(NSMidX(parent_frame) - width / 2.0,
                                  NSMaxY(parent_frame) - height - 28.0,
                                  width, height);

  NSPanel* panel =
      [[NSPanel alloc] initWithContentRect:frame
                                 styleMask:NSWindowStyleMaskBorderless |
                                           NSWindowStyleMaskNonactivatingPanel
                                   backing:NSBackingStoreBuffered
                                     defer:NO];
  panel.opaque = NO;
  panel.backgroundColor = NSColor.clearColor;
  panel.hasShadow = YES;
  panel.ignoresMouseEvents = YES;
  panel.level = parent.level + 1;
  panel.collectionBehavior = NSWindowCollectionBehaviorFullScreenAuxiliary |
                             NSWindowCollectionBehaviorCanJoinAllSpaces |
                             NSWindowCollectionBehaviorTransient;

  NSView* container =
      [[NSView alloc] initWithFrame:NSMakeRect(0.0, 0.0, width, height)];
  container.wantsLayer = YES;
  container.layer.cornerRadius = 14.0;
  container.layer.masksToBounds = YES;
  container.layer.backgroundColor =
      [[NSColor colorWithCalibratedWhite:0.06 alpha:0.86] CGColor];

  NSTextField* title = [NSTextField labelWithString:@"Full screen"];
  title.frame = NSMakeRect(20.0, 45.0, width - 40.0, 18.0);
  ConfigureDisclosureLabel(
      title, [NSFont systemFontOfSize:13.0 weight:NSFontWeightSemibold],
      NSColor.whiteColor);

  NSTextField* origin_label =
      [NSTextField labelWithString:OriginDisclosureLabel(origin)];
  origin_label.frame = NSMakeRect(20.0, 26.0, width - 40.0, 16.0);
  ConfigureDisclosureLabel(origin_label,
                           [NSFont systemFontOfSize:12.0
                                             weight:NSFontWeightRegular],
                           [NSColor colorWithCalibratedWhite:1.0 alpha:0.78]);

  NSTextField* instruction =
      [NSTextField labelWithString:@"Press Esc to exit full screen"];
  instruction.frame = NSMakeRect(20.0, 10.0, width - 40.0, 15.0);
  ConfigureDisclosureLabel(instruction,
                           [NSFont systemFontOfSize:11.0
                                             weight:NSFontWeightRegular],
                           [NSColor colorWithCalibratedWhite:1.0 alpha:0.58]);

  [container addSubview:title];
  [container addSubview:origin_label];
  [container addSubview:instruction];
  panel.contentView = container;

  [parent addChildWindow:panel ordered:NSWindowAbove];
  [panel orderFront:nil];

  fullscreen_disclosure_ = (__bridge_retained void*)panel;
  fullscreen_disclosure_hide_callback_ = std::move(first_hide_callback);
  exclusive_access_bubble_visible_ = true;
}

void MoriExclusiveAccessContext::HideFullscreenDisclosure(
    ExclusiveAccessBubbleHideReason reason) {
  NSPanel* panel = (__bridge_transfer NSPanel*)fullscreen_disclosure_;
  fullscreen_disclosure_ = nullptr;
  exclusive_access_bubble_visible_ = false;

  if (panel) {
    [panel.parentWindow removeChildWindow:panel];
    [panel orderOut:nil];
    [panel close];
  }

  if (fullscreen_disclosure_hide_callback_) {
    std::move(fullscreen_disclosure_hide_callback_).Run(reason);
  }
}

MoriBrowserWindow::~MoriBrowserWindow() {
  mori::OnBrowserWindowDestroyed(browser_);
}

// --- MoriModalDialogHost ----------------------------------------------------

MoriModalDialogHost::~MoriModalDialogHost() {
  for (auto& observer : observers_) {
    observer.OnHostDestroying();
  }
}

gfx::NativeView MoriModalDialogHost::GetHostView() const {
  return gfx::NativeView(mori::MoriMainWindow().contentView);
}

gfx::Point MoriModalDialogHost::GetDialogPosition(const gfx::Size& size) {
  NSView* content = mori::MoriMainWindow().contentView;
  const int width = content ? NSWidth(content.bounds) : 1280;
  return gfx::Point(std::max(0, (width - size.width()) / 2), 64);
}

gfx::Size MoriModalDialogHost::GetMaximumDialogSize() {
  NSView* content = mori::MoriMainWindow().contentView;
  if (!content) {
    return gfx::Size(1200, 760);
  }
  return gfx::Size(NSWidth(content.bounds), NSHeight(content.bounds));
}

void MoriModalDialogHost::AddObserver(
    web_modal::ModalDialogHostObserver* observer) {
  observers_.AddObserver(observer);
}

void MoriModalDialogHost::RemoveObserver(
    web_modal::ModalDialogHostObserver* observer) {
  observers_.RemoveObserver(observer);
}

bool MoriBrowserWindow::IsMaximized() const {
  return false;
}

bool MoriBrowserWindow::IsMinimized() const {
  return false;
}

bool MoriBrowserWindow::IsFullscreen() const {
  return false;
}

void MoriBrowserWindow::Hide() {}

void MoriBrowserWindow::ShowInactive() {}

void MoriBrowserWindow::Deactivate() {}

void MoriBrowserWindow::Maximize() {}

void MoriBrowserWindow::Minimize() {}

void MoriBrowserWindow::Restore() {}

void MoriBrowserWindow::FlashFrame(bool flash) {}

ui::ZOrderLevel MoriBrowserWindow::GetZOrderLevel() const {
  return ui::ZOrderLevel::kNormal;
}

void MoriBrowserWindow::SetZOrderLevel(ui::ZOrderLevel order) {}

bool MoriBrowserWindow::IsOnCurrentWorkspace() const {
  return false;
}

bool MoriBrowserWindow::IsVisibleOnScreen() const {
  return false;
}

void MoriBrowserWindow::SetTopControlsShownRatio(content::WebContents* web_contents, float ratio) {}

bool MoriBrowserWindow::DoBrowserControlsShrinkRendererSize( const content::WebContents* contents) const {
  return false;
}

ui::NativeTheme* MoriBrowserWindow::GetNativeTheme() {
  return nullptr;
}

const ui::ThemeProvider* MoriBrowserWindow::GetThemeProvider() const {
  return nullptr;
}

const ui::ColorProvider* MoriBrowserWindow::GetColorProvider() const {
  return nullptr;
}

int MoriBrowserWindow::GetTopControlsHeight() const {
  return {};
}

void MoriBrowserWindow::SetTopControlsGestureScrollInProgress(bool in_progress) {}

std::vector<StatusBubble*> MoriBrowserWindow::GetStatusBubbles() {
  return {};
}

void MoriBrowserWindow::UpdateTitleBar() {}

void MoriBrowserWindow::BookmarkBarStateChanged( BookmarkBar::AnimateChangeType change_type) {}

void MoriBrowserWindow::TemporarilyShowBookmarkBar(base::TimeDelta duration) {}

void MoriBrowserWindow::UpdateDevTools(content::WebContents* inspected_web_contents) {}

bool MoriBrowserWindow::CanDockDevTools() const {
  return false;
}

void MoriBrowserWindow::UpdateLoadingAnimations(bool is_visible) {}

void MoriBrowserWindow::SetStarredState(bool is_starred) {}

bool MoriBrowserWindow::IsTabModalPopupDeprecated() const {
  return false;
}

void MoriBrowserWindow::SetIsTabModalPopupDeprecated( bool is_tab_modal_popup_deprecated) {}

void MoriBrowserWindow::OnActiveTabChanged(content::WebContents* old_contents, content::WebContents* new_contents, int index, int reason) {}

void MoriBrowserWindow::OnTabDetached(content::WebContents* contents, bool was_active) {}

void MoriBrowserWindow::ZoomChangedForActiveTab(bool can_show_bubble) {}

bool MoriBrowserWindow::ShouldHideUIForFullscreen() const {
  return false;
}

bool MoriBrowserWindow::IsFullscreenBubbleVisible() const {
  return false;
}

bool MoriBrowserWindow::IsForceFullscreen() const {
  return false;
}

void MoriBrowserWindow::SetForceFullscreen(bool force_fullscreen) {}

gfx::Size MoriBrowserWindow::GetContentsSize() const {
  return {};
}

void MoriBrowserWindow::SetContentsSize(const gfx::Size& size) {}

void MoriBrowserWindow::UpdatePageActionIcon(PageActionIconType type) {}

autofill::AutofillBubbleHandler* MoriBrowserWindow::GetAutofillBubbleHandler() {
  return nullptr;
}

void MoriBrowserWindow::ExecutePageActionIconForTesting(PageActionIconType type) {}

LocationBar* MoriBrowserWindow::GetLocationBar() const {
  return const_cast<MoriLocationBar*>(&location_bar_);
}

void MoriBrowserWindow::SetFocusToLocationBar(bool is_user_initiated) {}

void MoriBrowserWindow::UpdateReloadStopState(bool is_loading, bool force) {}

void MoriBrowserWindow::UpdateToolbar(content::WebContents* contents) {}

bool MoriBrowserWindow::UpdateToolbarSecurityState() {
  return false;
}

void MoriBrowserWindow::UpdateCustomTabBarVisibility(bool visible, bool animate) {}

void MoriBrowserWindow::SetDevToolsScrimVisibility(bool visible) {}

void MoriBrowserWindow::ResetToolbarTabState(content::WebContents* contents) {}

void MoriBrowserWindow::FocusToolbar() {}

void MoriBrowserWindow::ToolbarSizeChanged(bool is_animating) {}

void MoriBrowserWindow::TabDraggingStatusChanged(bool is_dragging) {}

void MoriBrowserWindow::LinkOpeningFromGesture(WindowOpenDisposition disposition) {}

void MoriBrowserWindow::FocusAppMenu() {}

void MoriBrowserWindow::FocusBookmarksToolbar() {}

void MoriBrowserWindow::FocusInactivePopupForAccessibility() {}

void MoriBrowserWindow::RotatePaneFocus(bool forwards) {}

void MoriBrowserWindow::FocusWebContentsPane() {
  if (content::WebContents* contents =
          browser_->tab_strip_model()->GetActiveWebContents()) {
    contents->Focus();
  }
}

bool MoriBrowserWindow::IsBookmarkBarVisible() const {
  return false;
}

bool MoriBrowserWindow::IsBookmarkBarAnimating() const {
  return false;
}

bool MoriBrowserWindow::IsTabStripEditable() const {
  return true;
}

void MoriBrowserWindow::DisableTabStripEditingForTesting() {}

bool MoriBrowserWindow::IsToolbarVisible() const {
  return false;
}

bool MoriBrowserWindow::IsToolbarShowing() const {
  return false;
}

bool MoriBrowserWindow::IsLocationBarVisible() const {
  return false;
}

SharingDialog* MoriBrowserWindow::ShowSharingDialog(content::WebContents* contents, SharingDialogData data) {
  return nullptr;
}

void MoriBrowserWindow::ShowUpdateChromeDialog() {}

void MoriBrowserWindow::ShowIntentPickerBubble( std::vector<apps::IntentPickerAppInfo> app_info, bool show_stay_in_chrome, bool show_remember_selection, apps::IntentPickerBubbleType bubble_type, const std::optional<url::Origin>& initiating_origin, IntentPickerResponse callback) {}

void MoriBrowserWindow::ShowBookmarkBubble(const GURL& url, bool already_bookmarked) {}

sharing_hub::ScreenshotCapturedBubble* MoriBrowserWindow::ShowScreenshotCapturedBubble( content::WebContents* contents, const gfx::Image& image) {
  return nullptr;
}

qrcode_generator::QRCodeGeneratorBubbleView* MoriBrowserWindow::ShowQRCodeGeneratorBubble(content::WebContents* contents, const GURL& url, bool show_back_button) {
  return nullptr;
}

send_tab_to_self::SendTabToSelfBubbleView* MoriBrowserWindow::ShowSendTabToSelfDevicePickerBubble(content::WebContents* contents) {
  return nullptr;
}

send_tab_to_self::SendTabToSelfBubbleView* MoriBrowserWindow::ShowSendTabToSelfPromoBubble(content::WebContents* contents, bool show_signin_button) {
  return nullptr;
}

sharing_hub::SharingHubBubbleView* MoriBrowserWindow::ShowSharingHubBubble( share::ShareAttempt attempt) {
  return nullptr;
}

ShowTranslateBubbleResult MoriBrowserWindow::ShowTranslateBubble( content::WebContents* contents, translate::TranslateStep step, const std::string& source_language, const std::string& target_language, translate::TranslateErrors error_type, bool is_user_gesture) {
  return {};
}

void MoriBrowserWindow::StartPartialTranslate(const std::string& source_language, const std::string& target_language, const std::u16string& text_selection) {}

DownloadBubbleUIController* MoriBrowserWindow::GetDownloadBubbleUIController() {
  return nullptr;
}

void MoriBrowserWindow::ConfirmBrowserCloseWithPendingDownloads( int download_count, Browser::DownloadCloseType dialog_type, base::OnceCallback<void(bool)> callback) {}

void MoriBrowserWindow::ShowAppMenu() {}

void MoriBrowserWindow::PreHandleDragUpdate(const content::DropData& drop_data, const gfx::PointF& point) {}

void MoriBrowserWindow::PreHandleDragExit() {}

void MoriBrowserWindow::HandleDragEnded() {}

content::KeyboardEventProcessingResult MoriBrowserWindow::PreHandleKeyboardEvent( const input::NativeWebKeyboardEvent& event) {
  if (HandleMoriShortcut(event)) {
    return content::KeyboardEventProcessingResult::HANDLED;
  }
  return content::KeyboardEventProcessingResult::NOT_HANDLED;
}

bool MoriBrowserWindow::HandleKeyboardEvent( const input::NativeWebKeyboardEvent& event) {
  return false;
}

std::unique_ptr<FindBar> MoriBrowserWindow::CreateFindBar() {
  return std::make_unique<MoriFindBar>();
}

web_modal::WebContentsModalDialogHost*
MoriBrowserWindow::GetWebContentsModalDialogHost() {
  return &modal_dialog_host_;
}

web_modal::WebContentsModalDialogHost*
MoriBrowserWindow::GetWebContentsModalDialogHostFor(
    content::WebContents* web_contents) {
  return &modal_dialog_host_;
}

void MoriBrowserWindow::ShowAvatarBubbleFromAvatarButton(bool is_source_accelerator) {}

void MoriBrowserWindow::MaybeShowProfileSwitchIPH() {}

void MoriBrowserWindow::MaybeShowSupervisedUserProfileSignInIPH() {}

void MoriBrowserWindow::ShowHatsDialog( const std::string& site_id, const std::optional<std::string>& hats_histogram_name, const std::optional<uint64_t> hats_survey_ukm_id, base::OnceClosure success_callback, base::OnceClosure failure_callback, const SurveyBitsData& product_specific_bits_data, const SurveyStringData& product_specific_string_data) {}

ExclusiveAccessContext* MoriBrowserWindow::GetExclusiveAccessContext() {
  return &exclusive_access_context_;
}

std::string MoriBrowserWindow::GetWorkspace() const {
  return {};
}

bool MoriBrowserWindow::IsVisibleOnAllWorkspaces() const {
  return false;
}

void MoriBrowserWindow::ShowEmojiPanel() {}

std::unique_ptr<content::EyeDropper> MoriBrowserWindow::OpenEyeDropper( content::RenderFrameHost* frame, content::EyeDropperListener* listener) {
  return {};
}

void MoriBrowserWindow::ShowCaretBrowsingDialog() {}

void MoriBrowserWindow::CreateTabSearchBubble() {}

void MoriBrowserWindow::CloseTabSearchBubble() {}

void MoriBrowserWindow::ShowIncognitoClearBrowsingDataDialog() {}

void MoriBrowserWindow::ShowIncognitoHistoryDisclaimerDialog() {}

bool MoriBrowserWindow::IsUnframedModeEnabled() const {
  return false;
}

bool MoriBrowserWindow::GetCanResize() {
  return false;
}

ui::mojom::WindowShowState MoriBrowserWindow::GetWindowShowState() const {
  return {};
}

void MoriBrowserWindow::ShowChromeLabs() {}

BrowserView* MoriBrowserWindow::AsBrowserView() {
  return nullptr;
}

void MoriBrowserWindow::DeleteBrowserWindow() {
  delete this;
}

// --- Real implementations against the shared Mori window -------------------

namespace {
NSWindow* MoriWindow() {
  return mori::MoriMainWindow();
}
}  // namespace

void MoriBrowserWindow::Show() {
  mori::EnsureMoriUIStarted(browser_);
}

void MoriBrowserWindow::Close() {
  // The BrowserView/WebUIBrowserWindow close protocol, minus the OS window:
  // beforeunload gets a veto, then tabs close (TabStripEmpty() re-enters
  // Close()), and an empty browser is destroyed synchronously.
  if (!browser_->HandleBeforeClose()) {
    return;
  }
  browser_->OnWindowClosing();
  if (!browser_->tab_strip_model()->empty()) {
    browser_->tab_strip_model()->CloseAllTabs();
    return;
  }
  browser_->SynchronouslyDestroyBrowser();
  // `this` is deleted.
}

bool MoriBrowserWindow::IsActive() const {
  return MoriWindow().isKeyWindow;
}

void MoriBrowserWindow::Activate() {
  [MoriWindow() makeKeyAndOrderFront:nil];
}

gfx::NativeWindow MoriBrowserWindow::GetNativeWindow() const {
  return gfx::NativeWindow(MoriWindow());
}

gfx::Rect MoriBrowserWindow::GetBounds() const {
  NSWindow* window = MoriWindow();
  if (!window) {
    return gfx::Rect(0, 0, 1280, 820);
  }
  NSRect f = window.frame;
  NSScreen* screen = window.screen ?: NSScreen.screens.firstObject;
  const CGFloat flipped_y = NSMaxY(screen.frame) - NSMaxY(f);
  return gfx::Rect(NSMinX(f), flipped_y, NSWidth(f), NSHeight(f));
}

gfx::Rect MoriBrowserWindow::GetRestoredBounds() const {
  return GetBounds();
}

ui::mojom::WindowShowState MoriBrowserWindow::GetRestoredState() const {
  return ui::mojom::WindowShowState::kNormal;
}

bool MoriBrowserWindow::IsVisible() const {
  return MoriWindow().isVisible;
}

void MoriBrowserWindow::SetBounds(const gfx::Rect& bounds) {}
