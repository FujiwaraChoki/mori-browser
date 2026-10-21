#include "chrome/browser/ui/views/mori/mori_new_tab.h"

#include "build/build_config.h"
#include <memory>
#include <string>

#include "base/memory/raw_ptr.h"
#include "base/scoped_multi_source_observation.h"
#include "cc/paint/paint_flags.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/color/chrome_color_id.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/tabs/tab_strip_model_observer.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_command_bar.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_layout.h"
#include "chrome/browser/ui/views/mori/mori_overlay.h"
#include "content/public/browser/navigation_handle.h"
#include "third_party/skia/include/core/SkPathBuilder.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/web_contents_observer.h"
#include "ui/base/class_property.h"
#include "ui/base/cursor/cursor.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/color/color_provider.h"
#include "ui/events/event.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/font_list.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/gfx/paint_vector_icon.h"
#include "ui/gfx/text_utils.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/controls/native/native_view_host.h"
#include "ui/views/view.h"
#include "ui/views/view_class_properties.h"
#include "ui/views/view_observer.h"
#include "ui/views/view_tracker.h"
#include "ui/views/widget/widget.h"
#include "url/gurl.h"

namespace mori {
namespace {

constexpr int kLogoSize = 56;
constexpr int kFieldMaxWidth = 420;
constexpr int kFieldHeight = 44;
constexpr int kGap = 16;

gfx::FontList Font(int size) {
  const gfx::FontList base;
  return base.DeriveWithSizeDelta(size - base.GetFontSize());
}

// The page: the logo (two trees on the ground, logo.svg) and the field.
class NewTabView : public views::View {
  METADATA_HEADER(NewTabView, views::View)

 public:
  explicit NewTabView(BrowserWindowInterface* browser) : browser_(browser) {
    GetViewAccessibility().SetRole(ax::mojom::Role::kButton);
    GetViewAccessibility().SetName(u"Search or enter address");
  }

  void OnPaint(gfx::Canvas* canvas) override {
    // The card's colour, apart from the sidebar's (which is translucent,
    // for a space's wash).
    const SkColor background =
        GetColorProvider()->GetColor(kColorNewTabPageBackground);
    const bool dark = color_utils::IsDark(background);
    const SkColor foreground =
        dark ? SkColorSetRGB(0xEE, 0xEE, 0xEF) : SkColorSetRGB(0x1D, 0x1F, 0x22);
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    flags.setColor(background);
    canvas->DrawRoundRect(gfx::RectF(GetLocalBounds()), kContentCardRadius,
                          flags);

    const int total = kLogoSize + kGap + kFieldHeight;
    const int top = (height() - total) / 2;
    PaintLogo(canvas,
              gfx::Rect((width() - kLogoSize) / 2, top, kLogoSize, kLogoSize),
              SkColorSetA(foreground, 0x2E));

    const gfx::Rect field = Field();
    gfx::RectF field_f(field);
    flags.setColor(SkColorSetA(foreground, hovered_ ? 0x14 : 0x0D));
    canvas->DrawRoundRect(field_f, 12, flags);
    flags.setStyle(cc::PaintFlags::kStroke_Style);
    flags.setStrokeWidth(1);
    flags.setColor(SkColorSetA(foreground, 0x1F));
    field_f.Inset(0.5f);
    canvas->DrawRoundRect(field_f, 11.5f, flags);

    const SkColor muted = SkColorSetA(foreground, 0x99);
    const gfx::ImageSkia search =
        gfx::CreateVectorIcon(LucideIcon("search"), 16, muted);
    canvas->DrawImageInt(search, field.x() + 16,
                         field.y() + (field.height() - search.height()) / 2);
    // The shortcut, as key caps.
    const gfx::FontList caps_font = Font(11);
    int right = field.right() - 14;
#if BUILDFLAG(IS_MAC)
    const std::u16string modifier = u"⌘";
#else
    const std::u16string modifier = u"Ctrl";
#endif
    for (const std::u16string& key : {std::u16string(u"L"), modifier}) {
      const int width = std::max(20, gfx::GetStringWidth(key, caps_font) + 10);
      const gfx::Rect cap(right - width, field.y() + (field.height() - 20) / 2,
                          width, 20);
      cc::PaintFlags cap_flags;
      cap_flags.setAntiAlias(true);
      cap_flags.setColor(SkColorSetA(foreground, 0x14));
      canvas->DrawRoundRect(gfx::RectF(cap), 4, cap_flags);
      canvas->DrawStringRectWithFlags(key, caps_font, muted, cap,
                                      gfx::Canvas::TEXT_ALIGN_CENTER);
      right = cap.x() - 4;
    }
    const int text_x = field.x() + 16 + 16 + 12;
    canvas->DrawStringRect(u"Search or enter address", Font(14), muted,
                           gfx::Rect(text_x, field.y(), right - 8 - text_x,
                                     field.height()));
  }

  bool OnMousePressed(const ui::MouseEvent& event) override {
    return Field().Contains(event.location());
  }

  void OnMouseReleased(const ui::MouseEvent& event) override {
    if (Field().Contains(event.location())) {
      ShowCommandBar(browser_, CommandBarMode::kCurrentTab);
    }
  }

  void OnMouseMoved(const ui::MouseEvent& event) override {
    SetHovered(Field().Contains(event.location()));
  }
  void OnMouseExited(const ui::MouseEvent& event) override {
    SetHovered(false);
  }

  ui::Cursor GetCursor(const ui::MouseEvent& event) override {
    return Field().Contains(event.location())
               ? ui::Cursor(ui::mojom::CursorType::kIBeam)
               : ui::Cursor();
  }

 private:
  gfx::Rect Field() const {
    const int width = std::min(kFieldMaxWidth, std::max(0, this->width() - 48));
    const int total = kLogoSize + kGap + kFieldHeight;
    const int top = (height() - total) / 2 + kLogoSize + kGap;
    return gfx::Rect((this->width() - width) / 2, top, width, kFieldHeight);
  }

  void SetHovered(bool hovered) {
    if (hovered_ != hovered) {
      hovered_ = hovered;
      SchedulePaint();
    }
  }

  // logo.svg (64x64): two trunks, two filled triangles, the ground; 4px
  // round strokes.
  static void PaintLogo(gfx::Canvas* canvas, const gfx::Rect& rect, SkColor color) {
    const float s = rect.width() / 64.0f;
    auto p = [&](float x, float y) {
      return SkPoint::Make(rect.x() + x * s, rect.y() + y * s);
    };
    cc::PaintFlags stroke;
    stroke.setAntiAlias(true);
    stroke.setColor(color);
    stroke.setStyle(cc::PaintFlags::kStroke_Style);
    stroke.setStrokeWidth(4 * s);
    stroke.setStrokeCap(cc::PaintFlags::kRound_Cap);
    stroke.setStrokeJoin(cc::PaintFlags::kRound_Join);
    cc::PaintFlags fill = stroke;
    fill.setStyle(cc::PaintFlags::kFill_Style);
    auto line = [&](SkPoint a, SkPoint b) {
      SkPathBuilder path;
      path.moveTo(a);
      path.lineTo(b);
      canvas->DrawPath(path.detach(), stroke);
    };
    auto tree = [&](SkPoint a, SkPoint b, SkPoint c, SkPoint d) {
      SkPathBuilder path;
      path.moveTo(a);
      path.lineTo(b);
      path.lineTo(c);
      path.lineTo(d);
      path.close();
      SkPath triangle = path.detach();
      // logo.svg strokes and fills the triangles.
      canvas->DrawPath(triangle, fill);
      canvas->DrawPath(triangle, stroke);
    };
    line(p(20, 54), p(20, 40));
    line(p(44, 54), p(44, 36));
    tree(p(20, 40), p(10, 40), p(20, 14), p(30, 40));
    tree(p(44, 36), p(32, 36), p(44, 8), p(56, 36));
    line(p(6, 56), p(58, 56));
  }

  raw_ptr<BrowserWindowInterface> browser_;
  bool hovered_ = false;
};

BEGIN_METADATA(NewTabView)
END_METADATA

// Shows the page over the card while the tab on screen is a new tab page.
class NewTabPages : public TabStripModelObserver,
                    public content::WebContentsObserver,
                    public views::ViewObserver {
 public:
  explicit NewTabPages(BrowserView* browser_view)
      : browser_view_(browser_view) {
    browser_view->browser()->GetTabStripModel()->AddObserver(this);
    Observe(browser_view->browser()->GetTabStripModel()->GetActiveWebContents());
    Update();
  }

  ~NewTabPages() override {
    if (browser_view_->browser()) {
      browser_view_->browser()->GetTabStripModel()->RemoveObserver(this);
    }
  }

  // TabStripModelObserver:
  void OnTabStripModelChanged(
      TabStripModel* tab_strip_model,
      const TabStripModelChange& change,
      const TabStripSelectionChange& selection) override {
    if (selection.active_tab_changed()) {
      Observe(tab_strip_model->GetActiveWebContents());
    }
    Update();
  }

  // content::WebContentsObserver:
  void DidStartNavigation(content::NavigationHandle* handle) override {
    if (handle->IsInPrimaryMainFrame()) {
      Update();
    }
  }
  void DidFinishNavigation(content::NavigationHandle* handle) override {
    if (handle->IsInPrimaryMainFrame()) {
      Update();
    }
  }

  // views::ViewObserver:
  void OnViewBoundsChanged(views::View* observed_view) override { Update(); }
  // The card goes before the BrowserView's properties (this) when the
  // window closes.
  void OnViewIsDeleting(views::View* observed_view) override {
    view_observation_.RemoveObservation(observed_view);
  }

 private:
  void Update() {
    views::View* card = browser_view_->contents_container();
    content::WebContents* contents = web_contents();
    const bool show = card && contents &&
                      IsNewTabPage(contents->GetVisibleURL()) &&
                      browser_view_->GetWidget();
    if (!show) {
      if (host_.view()) {
        host_.view()->SetVisible(false);
      }
      return;
    }
    if (!overlay_) {
      overlay_ = std::make_unique<Overlay>(
          browser_view_,
          std::make_unique<NewTabView>(browser_view_->browser()),
          "MoriNewTab");
      // A child widget on its own sits above every view in the window,
      // the floating sidebar too. Hosted in the view tree, as the page
      // itself is, it stacks just above the card and below the sidebar.
      size_t index = browser_view_->children().size();
      for (views::View* child = card; child; child = child->parent()) {
        if (child->parent() == browser_view_) {
          index = *browser_view_->GetIndexOf(child) + 1;
          break;
        }
      }
      auto* host = browser_view_->AddChildViewAt(
          std::make_unique<views::NativeViewHost>(), index);
      host->SetProperty(views::kViewIgnoredByLayoutKey, true);
      host->SetVisible(false);
      views::Widget* widget = overlay_->contents()->GetWidget();
      host->Attach(widget->GetNativeView());
      widget->Show();
      host_.SetView(host);
    }
    // The card moves with the sidebar and resizes with the window.
    if (!view_observation_.IsObservingSource(card)) {
      view_observation_.AddObservation(card);
    }
    if (!view_observation_.IsObservingSource(browser_view_.get())) {
      view_observation_.AddObservation(browser_view_.get());
    }
    views::View* host = host_.view();
    if (!host) {
      return;
    }
    host->SetBoundsRect(views::View::ConvertRectToTarget(
        card, browser_view_, card->GetLocalBounds()));
    host->SetVisible(true);
  }

  raw_ptr<BrowserView> browser_view_;
  std::unique_ptr<Overlay> overlay_;
  // The overlay's place in the BrowserView, which owns (and may delete) it.
  views::ViewTracker host_;
  base::ScopedMultiSourceObservation<views::View, views::ViewObserver>
      view_observation_{this};
};

}  // namespace
}  // namespace mori

DEFINE_UI_CLASS_PROPERTY_TYPE(mori::NewTabPages*)

namespace mori {
namespace {
DEFINE_OWNED_UI_CLASS_PROPERTY_KEY(NewTabPages, kNewTabPagesKey)
}  // namespace

bool IsNewTabPage(const GURL& url) {
  if (url.SchemeIs("chrome-search")) {
    return true;
  }
  return url.SchemeIs("chrome") &&
         (url.host() == "newtab" || url.host() == "new-tab-page" ||
          url.host() == "new-tab-page-third-party");
}

void WatchNewTabPages(BrowserWindowInterface* browser) {
  if (browser->GetType() != BrowserWindowInterface::TYPE_NORMAL) {
    return;
  }
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view || browser_view->GetProperty(kNewTabPagesKey)) {
    return;
  }
  browser_view->SetProperty(kNewTabPagesKey,
                            std::make_unique<NewTabPages>(browser_view));
}

}  // namespace mori
