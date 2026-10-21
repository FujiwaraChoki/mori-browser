#include "chrome/browser/ui/views/mori/mori_peek.h"

#include <algorithm>
#include <memory>
#include <string>
#include <string_view>
#include <utility>

#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/scoped_observation.h"
#include "base/task/sequenced_task_runner.h"
#include "cc/paint/paint_flags.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/navigator/browser_navigator.h"
#include "chrome/browser/ui/navigator/browser_navigator_params.h"
#include "chrome/browser/ui/tab_helpers.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/tabs/tab_strip_model_observer.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_window_events.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_overlay.h"
#include "chrome/browser/ui/views/mori/mori_tab_list.h"
#include "components/input/native_web_keyboard_event.h"
#include "content/public/browser/navigation_controller.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/web_contents_delegate.h"
#include "third_party/blink/public/common/input/web_input_event.h"
#include "ui/base/accelerators/accelerator.h"
#include "ui/base/class_property.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/base/models/image_model.h"
#include "ui/color/color_id.h"
#include "ui/color/color_provider.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/gfx/geometry/rounded_corners_f.h"
#include "ui/gfx/paint_vector_icon.h"
#include "ui/gfx/shadow_value.h"
#include "ui/gfx/skia_paint_util.h"
#include "ui/views/controls/button/button.h"
#include "ui/views/controls/native/native_view_host.h"
#include "ui/views/controls/webview/unhandled_keyboard_event_handler.h"
#include "ui/views/controls/webview/webview.h"
#include "ui/views/view.h"
#include "ui/views/view_observer.h"
#include "ui/events/event.h"
#include "ui/events/event_observer.h"
#include "ui/views/event_monitor.h"
#include "ui/views/widget/widget.h"
#include "url/gurl.h"

// Link Preview's name in tab_helpers.h, which lets a previewed page be
// adopted as a tab-to-be. Peek is the same thing.
class PreviewTab {
 public:
  static void Adopt(content::WebContents* contents) {
    TabHelpers::AttachTabHelpers(contents);
  }
};

namespace mori {
namespace {

// Mori 3's Peek (crates/mori/src/ui/peek.rs).
constexpr float kCardWidthRatio = 0.8f;
constexpr int kCardMaxWidth = 1200;
// Room kept beside the card for the buttons.
constexpr int kControlsGutter = 56;
constexpr int kControlSize = 32;
constexpr int kControlShadowRoom = 6;
constexpr int kControlGap = 12;
constexpr float kCardRadius = 10;
constexpr float kBackdropRadius = 6;

// One of Zen's Glance buttons: a light grey disc with a slate glyph, in
// both appearances.
class ControlButton : public views::Button {
  METADATA_HEADER(ControlButton, views::Button)

 public:
  ControlButton(PressedCallback callback,
                std::string_view icon,
                std::u16string tooltip)
      : views::Button(std::move(callback)), icon_(LucideIcon(icon)) {
    // No tooltip: a child widget's tooltip lands in the wrong place on
    // Wayland and stays there.
    SetAccessibleName(tooltip);
    SetPreferredSize(gfx::Size(kControlSize + 2 * kControlShadowRoom,
                               kControlSize + 2 * kControlShadowRoom));
  }

  void PaintButtonContents(gfx::Canvas* canvas) override {
    const gfx::RectF disc(kControlShadowRoom, kControlShadowRoom, kControlSize,
                          kControlSize);
    const bool hovered =
        GetState() == STATE_HOVERED || GetState() == STATE_PRESSED;
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    flags.setColor(hovered ? SkColorSetRGB(0xCF, 0xD1, 0xD3)
                           : SkColorSetRGB(0xDF, 0xE0, 0xE1));
    flags.setLooper(gfx::CreateShadowDrawLooper(
        {gfx::ShadowValue(gfx::Vector2d(0, 2), 8,
                          SkColorSetA(SK_ColorBLACK, 0x40))}));
    canvas->DrawCircle(disc.CenterPoint(), kControlSize / 2.f, flags);
    constexpr int kGlyph = 15;
    canvas->Save();
    canvas->Translate(gfx::Vector2d(
        static_cast<int>(disc.CenterPoint().x()) - kGlyph / 2,
        static_cast<int>(disc.CenterPoint().y()) - kGlyph / 2));
    gfx::PaintVectorIcon(canvas, *icon_, kGlyph,
                         SkColorSetRGB(0x50, 0x62, 0x78));
    canvas->Restore();
  }

  void StateChanged(ButtonState old_state) override { SchedulePaint(); }

 private:
  raw_ref<const gfx::VectorIcon> icon_;
};

BEGIN_METADATA(ControlButton)
END_METADATA

// The dimmed page, the card holding the peeked page, and its buttons.
class PeekView : public views::View,
                 public content::WebContentsDelegate,
                 public TabStripModelObserver,
                 public ui::EventObserver {
  METADATA_HEADER(PeekView, views::View)

 public:
  PeekView(BrowserView* browser_view, base::RepeatingClosure hide)
      : browser_view_(browser_view), hide_(std::move(hide)) {
    web_view_ = AddChildView(std::make_unique<views::WebView>(
        browser_view->browser()->GetProfile()));
    close_ = AddChildView(std::make_unique<ControlButton>(
        base::BindRepeating(&PeekView::Close, base::Unretained(this)), "x",
        u"Close"));
    open_ = AddChildView(std::make_unique<ControlButton>(
        base::BindRepeating(&PeekView::Promote, base::Unretained(this),
                            WindowOpenDisposition::NEW_FOREGROUND_TAB),
        "maximize-2", u"Open as Tab"));
    split_ = AddChildView(std::make_unique<ControlButton>(
        base::BindRepeating(&PeekView::Promote, base::Unretained(this),
                            WindowOpenDisposition::NEW_SPLIT_VIEW),
        "columns-2", u"Open in Split View"));
    AddAccelerator(ui::Accelerator(ui::VKEY_ESCAPE, ui::EF_NONE));
    browser_view->browser()->GetTabStripModel()->AddObserver(this);
  }

  ~PeekView() override {
    web_view_->SetWebContents(nullptr);
    if (contents_) {
      contents_->SetDelegate(nullptr);
    }
  }

  bool is_open() const { return !!contents_; }

  void Open(const GURL& url) {
    DropContents();
    content::WebContents::CreateParams params(
        browser_view_->browser()->GetProfile());
    contents_ = content::WebContents::Create(params);
    PreviewTab::Adopt(contents_.get());
    contents_->SetDelegate(this);
    web_view_->SetWebContents(contents_.get());
    content::NavigationController::LoadURLParams load(url);
    load.transition_type = ui::PAGE_TRANSITION_LINK;
    contents_->GetController().LoadURLWithParams(load);
    // Escape closes Peek wherever the keyboard focus is.
    if (GetWidget()) {
      escape_monitor_ =
          MonitorWindow(this, GetWidget(), {ui::EventType::kKeyPressed});
    }
    InvalidateLayout();
  }

  // ui::EventObserver:
  using views::View::OnEvent;
  void OnEvent(const ui::Event& event) override {
    if (!contents_ || !event.IsKeyEvent() ||
        event.AsKeyEvent()->key_code() != ui::VKEY_ESCAPE ||
        !IsEventInWindow(event, GetWidget()) || contents_->IsFullscreen()) {
      return;
    }
    base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE,
        base::BindOnce(&PeekView::Close, weak_factory_.GetWeakPtr()));
  }

  void FocusPage() { web_view_->RequestFocus(); }

  // The page goes away; focus returns to the tab underneath.
  void Close() {
    if (!contents_) {
      return;
    }
    hide_.Run();
    DropContents();
    if (content::WebContents* active =
            browser_view_->browser()->GetTabStripModel()
                ->GetActiveWebContents()) {
      active->Focus();
    }
  }

  // views::View:
  void Layout(PassKey) override {
    const gfx::Rect card = CardBounds();
    web_view_->SetBoundsRect(card);
    web_view_->holder()->SetNativeViewCornerRadii(
        gfx::RoundedCornersF(kCardRadius));
    int y = card.y() + 27 - kControlShadowRoom;
    for (views::View* control : {close_.get(), open_.get(), split_.get()}) {
      const gfx::Size size = control->GetPreferredSize();
      control->SetBounds(card.right() + 14 - kControlShadowRoom, y,
                         size.width(), size.height());
      y += kControlSize + kControlGap;
    }
  }

  void OnPaint(gfx::Canvas* canvas) override {
    const SkColor frame =
        GetColorProvider()->GetColor(ui::kColorFrameActive);
    const bool dark = color_utils::IsDark(frame);
    cc::PaintFlags dim;
    dim.setAntiAlias(true);
    dim.setColor(SkColorSetA(SK_ColorBLACK, dark ? 0x99 : 0x73));
    canvas->DrawRoundRect(gfx::RectF(GetLocalBounds()), kBackdropRadius, dim);
    // The card's shadow; the page covers the card itself.
    cc::PaintFlags card;
    card.setAntiAlias(true);
    card.setColor(dark ? SkColorSetRGB(0x2B, 0x2B, 0x2B) : SK_ColorWHITE);
    card.setLooper(gfx::CreateShadowDrawLooper(
        {gfx::ShadowValue(gfx::Vector2d(0, 12), 48,
                          SkColorSetA(SK_ColorBLACK, dark ? 0x80 : 0x40))}));
    canvas->DrawRoundRect(gfx::RectF(CardBounds()), kCardRadius, card);
  }

  // A click on the dimmed page closes Peek.
  bool OnMousePressed(const ui::MouseEvent& event) override {
    Close();
    return true;
  }

  bool AcceleratorPressed(const ui::Accelerator& accelerator) override {
    Close();
    return true;
  }

  // content::WebContentsDelegate:
  content::WebContents* OpenURLFromTab(
      content::WebContents* source,
      const content::OpenURLParams& params,
      base::OnceCallback<void(content::NavigationHandle&)>
          navigation_handle_callback) override {
    // Links stay in Peek; new tabs and windows open in the browser.
    if (params.disposition == WindowOpenDisposition::CURRENT_TAB ||
        params.disposition == WindowOpenDisposition::NEW_PICTURE_IN_PICTURE) {
      source->GetController().LoadURLWithParams(
          content::NavigationController::LoadURLParams(params));
      return source;
    }
    NavigateParams nav(browser_view_->browser(), params.url,
                       params.transition);
    nav.FillNavigateParamsFromOpenURLParams(params);
    Navigate(&nav);
    return nav.navigated_or_inserted_contents;
  }

  content::WebContents* AddNewContents(
      content::WebContents* source,
      std::unique_ptr<content::WebContents> new_contents,
      const GURL& target_url,
      WindowOpenDisposition disposition,
      const blink::mojom::WindowFeatures& window_features,
      bool user_gesture,
      bool* was_blocked) override {
    NavigateParams nav(browser_view_->browser(), std::move(new_contents));
    nav.disposition = WindowOpenDisposition::NEW_FOREGROUND_TAB;
    Navigate(&nav);
    return nullptr;
  }

  content::KeyboardEventProcessingResult PreHandleKeyboardEvent(
      content::WebContents* source,
      const input::NativeWebKeyboardEvent& event) override {
    // Escape closes Peek before the page sees it, as Zen's Glance.
    if (event.GetType() == blink::WebInputEvent::Type::kRawKeyDown &&
        event.windows_key_code == ui::VKEY_ESCAPE &&
        !source->IsFullscreen()) {
      base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
          FROM_HERE, base::BindOnce(&PeekView::Close,
                                    weak_factory_.GetWeakPtr()));
      return content::KeyboardEventProcessingResult::HANDLED;
    }
    return content::KeyboardEventProcessingResult::NOT_HANDLED;
  }

  bool HandleKeyboardEvent(
      content::WebContents* source,
      const input::NativeWebKeyboardEvent& event) override {
    // The window's shortcuts work from inside Peek too.
    return unhandled_keyboard_event_handler_.HandleKeyboardEvent(
        event, GetFocusManager());
  }

  void CloseContents(content::WebContents* source) override {
    base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE,
        base::BindOnce(&PeekView::Close, weak_factory_.GetWeakPtr()));
  }

  // TabStripModelObserver:
  void OnTabStripModelChanged(
      TabStripModel* tab_strip_model,
      const TabStripModelChange& change,
      const TabStripSelectionChange& selection) override {
    // Peek belongs to the tab it was opened over.
    if (selection.active_tab_changed() && contents_ && !promoting_) {
      base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
          FROM_HERE,
          base::BindOnce(&PeekView::Close, weak_factory_.GetWeakPtr()));
    }
  }

 private:
  // Mori 3's card: 80% of the page's width (at most 1200), centered, as
  // tall as the page.
  gfx::Rect CardBounds() const {
    const gfx::Rect area = GetLocalBounds();
    const int room = std::max(200, area.width() - 2 * kControlsGutter);
    const int width = std::min(
        {static_cast<int>(area.width() * kCardWidthRatio), kCardMaxWidth,
         room});
    return gfx::Rect(area.x() + (area.width() - width) / 2, area.y(), width,
                     area.height());
  }

  // The page becomes a tab of its own, or half of a split with the tab
  // underneath.
  void Promote(WindowOpenDisposition disposition) {
    if (!contents_) {
      return;
    }
    hide_.Run();
    escape_monitor_.reset();
    web_view_->SetWebContents(nullptr);
    contents_->SetDelegate(nullptr);
    BrowserWindowInterface* browser = browser_view_->browser();
    NavigateParams nav(browser, std::move(contents_));
    nav.disposition = disposition;
    if (disposition == WindowOpenDisposition::NEW_FOREGROUND_TAB) {
      nav.tabstrip_index = IndexForNewTab(browser);
    }
    promoting_ = true;
    Navigate(&nav);
    promoting_ = false;
  }

  void DropContents() {
    escape_monitor_.reset();
    if (!contents_) {
      return;
    }
    web_view_->SetWebContents(nullptr);
    contents_->SetDelegate(nullptr);
    // Not from inside one of its own callbacks.
    base::SequencedTaskRunner::GetCurrentDefault()->DeleteSoon(
        FROM_HERE, std::move(contents_));
  }

  raw_ptr<BrowserView> browser_view_;
  base::RepeatingClosure hide_;
  raw_ptr<views::WebView> web_view_ = nullptr;
  raw_ptr<ControlButton> close_ = nullptr;
  raw_ptr<ControlButton> open_ = nullptr;
  raw_ptr<ControlButton> split_ = nullptr;
  std::unique_ptr<content::WebContents> contents_;
  bool promoting_ = false;
  std::unique_ptr<views::EventMonitor> escape_monitor_;
  views::UnhandledKeyboardEventHandler unhandled_keyboard_event_handler_;
  base::WeakPtrFactory<PeekView> weak_factory_{this};
};

BEGIN_METADATA(PeekView)
END_METADATA

// Owns Peek's widget; stored on the BrowserView. Keeps the widget over the
// page area as the window changes.
class Peek : public views::ViewObserver {
 public:
  explicit Peek(BrowserView* browser_view) : browser_view_(browser_view) {
    overlay_ = std::make_unique<Overlay>(
        browser_view,
        std::make_unique<PeekView>(
            browser_view,
            base::BindRepeating(&Peek::Hide, base::Unretained(this))),
        "MoriPeek");
  }

  void Show(const GURL& url) {
    auto* view = static_cast<PeekView*>(overlay_->contents());
    views::View* area = browser_view_->contents_container();
    if (!view || !area) {
      return;
    }
    view->Open(url);
    if (!observation_.IsObserving()) {
      observation_.Observe(area);
    }
    Reposition();
    view->FocusPage();
  }

  void Hide() {
    observation_.Reset();
    overlay_->Hide();
  }

  // views::ViewObserver:
  void OnViewBoundsChanged(views::View* observed_view) override {
    Reposition();
  }
  void OnViewIsDeleting(views::View* observed_view) override {
    observation_.Reset();
  }

 private:
  void Reposition() {
    views::View* area = browser_view_->contents_container();
    if (!area) {
      return;
    }
    overlay_->Show(views::View::ConvertRectToTarget(
        area, browser_view_, area->GetLocalBounds()));
  }

  raw_ptr<BrowserView> browser_view_;
  std::unique_ptr<Overlay> overlay_;
  base::ScopedObservation<views::View, views::ViewObserver> observation_{this};
};

}  // namespace
}  // namespace mori

DEFINE_UI_CLASS_PROPERTY_TYPE(mori::Peek*)

namespace mori {
namespace {
DEFINE_OWNED_UI_CLASS_PROPERTY_KEY(Peek, kPeekKey)
}  // namespace

bool ShowPeek(BrowserWindowInterface* browser, const GURL& url) {
  if (!browser || browser->GetType() != BrowserWindowInterface::TYPE_NORMAL) {
    return false;
  }
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view || !browser_view->GetWidget()) {
    return false;
  }
  Peek* peek = browser_view->GetProperty(kPeekKey);
  if (!peek) {
    peek = browser_view->SetProperty(kPeekKey,
                                     std::make_unique<Peek>(browser_view));
  }
  peek->Show(url);
  return true;
}

}  // namespace mori
