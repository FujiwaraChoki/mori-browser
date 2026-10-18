#include "chrome/browser/ui/views/mori/mori_overlay.h"

#include <utility>

#include "cc/paint/paint_flags.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/theme_copying_widget.h"
#include "ui/color/color_id.h"
#include "ui/color/color_provider.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/gfx/shadow_value.h"
#include "ui/gfx/skia_paint_util.h"
#include "ui/views/view.h"
#include "ui/views/widget/widget.h"

namespace mori {

Overlay::Overlay(BrowserView* browser_view,
                 std::unique_ptr<views::View> contents,
                 const char* name)
    : browser_view_(browser_view) {
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
  contents_ = widget_->SetContentsView(std::move(contents));
  // Control widgets start out shown.
  widget_->Hide();
  observation_.Observe(widget_.get());
}

Overlay::~Overlay() = default;

bool Overlay::IsVisible() const {
  return widget_ && contents_ && widget_->IsVisible();
}

void Overlay::Show(const gfx::Rect& bounds) {
  if (!contents_) {
    return;
  }
  gfx::Rect screen = bounds;
  views::View::ConvertRectToScreen(browser_view_, &screen);
  widget_->SetBounds(screen);
  widget_->Show();
}

void Overlay::Hide() {
  if (contents_ && widget_->IsVisible()) {
    widget_->Hide();
  }
}

void Overlay::OnWidgetDestroying(views::Widget* widget) {
  contents_ = nullptr;
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
