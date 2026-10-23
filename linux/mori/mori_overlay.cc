#include "chrome/browser/ui/views/mori/mori_overlay.h"

#include <utility>

#include "build/build_config.h"
#include "cc/paint/paint_flags.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/theme_copying_widget.h"
#include "ui/color/color_id.h"
#include "ui/color/color_provider.h"
#include "ui/compositor/layer.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/gfx/shadow_value.h"
#include "ui/gfx/skia_paint_util.h"
#include "ui/views/layout/fill_layout.h"
#include "ui/views/view.h"
#include "ui/views/view_class_properties.h"
#include "ui/views/widget/widget.h"

namespace mori {
namespace {
DEFINE_UI_CLASS_PROPERTY_KEY(int, kPageOverlayLayerKey, 0)
}  // namespace

void PlaceChildWidget(BrowserView* browser_view,
                      views::Widget* widget,
                      const gfx::Rect& bounds) {
#if BUILDFLAG(IS_MAC)
  // NativeWidgetMac adds the parent's NSWindow frame origin in SetBounds().
  // Subtract that origin after converting from the browser's client space,
  // retaining any titlebar inset between the frame and the BrowserView.
  gfx::Rect child_bounds = bounds;
  views::View::ConvertRectToScreen(browser_view, &child_bounds);
  child_bounds.Offset(-browser_view->GetWidget()
                           ->GetWindowBoundsInScreen()
                           .OffsetFromOrigin());
#else
  const gfx::Rect child_bounds = browser_view->ConvertRectToWidget(bounds);
#endif
  widget->SetBounds(child_bounds);
}

Overlay::Overlay(BrowserView* browser_view,
                 std::unique_ptr<views::View> contents,
                 const char* name,
                 Layer layer)
    : browser_view_(browser_view) {
  if (layer != Layer::kWindow) {
    // Page sheets belong to the browser's compositor tree. A native child
    // window would sit above the hover sidebar regardless of view ordering.
    views::View* page = browser_view->contents_container();
    while (page->parent() != browser_view) {
      page = page->parent();
    }
    size_t index = *browser_view->GetIndexOf(page) + 1;
    // Keep new-tab content below settings even when it is created later.
    while (index < browser_view->children().size() &&
           browser_view->children()[index]->GetProperty(kPageOverlayLayerKey) >
               0 &&
           browser_view->children()[index]->GetProperty(kPageOverlayLayerKey) <=
               static_cast<int>(layer)) {
      ++index;
    }
    auto host = std::make_unique<views::View>();
    host->SetProperty(kPageOverlayLayerKey, static_cast<int>(layer));
    host->SetProperty(views::kViewIgnoredByLayoutKey, true);
    host->SetLayoutManager(std::make_unique<views::FillLayout>());
    host->SetPaintToLayer();
    host->layer()->SetFillsBoundsOpaquely(false);
    host->SetVisible(false);
    contents_.SetView(host->AddChildView(std::move(contents)));
    page_host_.SetView(browser_view->AddChildViewAt(std::move(host), index));
    return;
  }
  views::Widget* parent = browser_view->GetWidget();
  widget_ = std::make_unique<ThemeCopyingWidget>(parent);
  views::Widget::InitParams params(
      views::Widget::InitParams::CLIENT_OWNS_WIDGET,
      views::Widget::InitParams::TYPE_CONTROL);
  params.name = name;
  params.parent = parent->GetNativeView();
  params.opacity = views::Widget::InitParams::WindowOpacity::kTranslucent;
  // The window stays active; see the command bar.
  params.activatable = views::Widget::InitParams::Activatable::kNo;
  widget_->Init(std::move(params));
  contents_.SetView(widget_->SetContentsView(std::move(contents)));
  // Control widgets start out shown.
  widget_->Hide();
  observation_.Observe(widget_.get());
}

Overlay::~Overlay() {
  if (views::View* host = page_host_.view()) {
    host->parent()->RemoveChildViewT(host);
  }
}

bool Overlay::IsVisible() const {
  if (const views::View* host = page_host_.view()) {
    return contents_.view() && host->GetVisible();
  }
  return widget_ && contents_.view() && widget_->IsVisible();
}

void Overlay::Show(const gfx::Rect& bounds) {
  if (!contents_.view()) {
    return;
  }
  if (views::View* host = page_host_.view()) {
    host->SetBoundsRect(bounds);
    host->SetVisible(true);
    return;
  }
  PlaceChildWidget(browser_view_, widget_.get(), bounds);
  if (!widget_->IsVisible()) {
    // Above the page and the panels opened before it.
    widget_->StackAtTop();
  }
  widget_->Show();
}

void Overlay::Hide() {
  if (views::View* host = page_host_.view()) {
    host->SetVisible(false);
    return;
  }
  if (widget_ && contents_.view() && widget_->IsVisible()) {
    widget_->Hide();
  }
}

void Overlay::OnWidgetDestroying(views::Widget* widget) {
  contents_.SetView(nullptr);
  observation_.Reset();
}

void PaintPopover(const views::View* view,
                  gfx::Canvas* canvas,
                  const gfx::Rect& popover) {
  const SkColor frame =
      view->GetColorProvider()->GetColor(ui::kColorFrameActive);
  const bool dark = color_utils::IsDark(frame);
  const SkColor panel = dark ? SkColorSetRGB(0x18, 0x18, 0x18) : SK_ColorWHITE;
  const SkColor edge =
      dark ? SkColorSetRGB(0x4B, 0x4B, 0x4B) : SkColorSetRGB(0xE2, 0xE2, 0xE2);
  const gfx::RectF bounds(popover);
  constexpr float kRadius = 12;
  cc::PaintFlags shadow;
  shadow.setAntiAlias(true);
  shadow.setColor(panel);
  shadow.setLooper(gfx::CreateShadowDrawLooper(
      {gfx::ShadowValue(gfx::Vector2d(0, 16), 60,
                        SkColorSetA(SK_ColorBLACK, dark ? 90 : 36))}));
  canvas->DrawRoundRect(bounds, kRadius, shadow);
  cc::PaintFlags fill;
  fill.setAntiAlias(true);
  fill.setColor(panel);
  canvas->DrawRoundRect(bounds, kRadius, fill);
  cc::PaintFlags stroke;
  stroke.setAntiAlias(true);
  stroke.setStyle(cc::PaintFlags::kStroke_Style);
  stroke.setStrokeWidth(1);
  stroke.setColor(edge);
  gfx::RectF inner = bounds;
  inner.Inset(0.5f);
  canvas->DrawRoundRect(inner, kRadius - 0.5f, stroke);
}

}  // namespace mori
