#include "chrome/browser/ui/views/mori/mori_toast.h"

#include <memory>
#include <utility>

#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/timer/timer.h"
#include "cc/paint/paint_flags.h"
#include "cc/paint/paint_shader.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_overlay.h"
#include "third_party/skia/include/core/SkPoint.h"
#include "ui/base/class_property.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/color/color_id.h"
#include "ui/color/color_provider.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/font_list.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/gfx/shadow_value.h"
#include "ui/gfx/skia_paint_util.h"
#include "ui/gfx/text_utils.h"
#include "ui/views/view.h"

namespace mori {
namespace {

constexpr base::TimeDelta kShowFor = base::Milliseconds(3500);
constexpr int kHeight = 40;
constexpr int kPadding = 14;
constexpr int kInset = 8;
// Room around the pill for its shadow.
constexpr int kShadow = 12;

gfx::FontList ToastFont() {
  const gfx::FontList base;
  return base.DeriveWithSizeDelta(14 - base.GetFontSize())
      .DeriveWithWeight(gfx::Font::Weight::BOLD);
}

class ToastView : public views::View {
  METADATA_HEADER(ToastView, views::View)

 public:
  explicit ToastView(base::RepeatingClosure dismiss)
      : dismiss_(std::move(dismiss)) {}

  void SetText(std::u16string text) {
    text_ = std::move(text);
    SchedulePaint();
  }

  gfx::Size PillSize() const {
    return gfx::Size(gfx::GetStringWidth(text_, ToastFont()) + 2 * kPadding,
                     kHeight);
  }

  void OnPaint(gfx::Canvas* canvas) override {
    const bool dark = color_utils::IsDark(
        GetColorProvider()->GetColor(ui::kColorFrameActive));
    gfx::RectF pill(GetLocalBounds());
    pill.Inset(kShadow);
    constexpr float kRadius = 6;
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    // Zen's pill: a vertical navy gradient (blue in light mode) on a soft
    // shadow, with a light outline and white text.
    flags.setColor(SK_ColorBLACK);
    flags.setLooper(gfx::CreateShadowDrawLooper(
        {gfx::ShadowValue(gfx::Vector2d(0, 4), 16,
                          SkColorSetA(SK_ColorBLACK, dark ? 0x4D : 0x1F))}));
    canvas->DrawRoundRect(pill, kRadius, flags);
    flags.setLooper(nullptr);
    const SkPoint points[2] = {SkPoint::Make(0, pill.y()),
                               SkPoint::Make(0, pill.bottom())};
    const SkColor4f colors[2] = {
        SkColor4f::FromColor(dark ? SkColorSetRGB(0x13, 0x2A, 0x47)
                                  : SkColorSetRGB(0x1A, 0x78, 0xF0)),
        SkColor4f::FromColor(dark ? SkColorSetRGB(0x12, 0x26, 0x40)
                                  : SkColorSetRGB(0x19, 0x69, 0xD0))};
    flags.setShader(cc::PaintShader::MakeLinearGradient(
        points, colors, nullptr, 2, SkTileMode::kClamp));
    canvas->DrawRoundRect(pill, kRadius, flags);
    cc::PaintFlags outline;
    outline.setAntiAlias(true);
    outline.setStyle(cc::PaintFlags::kStroke_Style);
    outline.setStrokeWidth(1);
    outline.setColor(SkColorSetA(SK_ColorWHITE, dark ? 0xA6 : 0x59));
    gfx::RectF edge = pill;
    edge.Inset(0.5f);
    canvas->DrawRoundRect(edge, kRadius - 0.5f, outline);
    canvas->DrawStringRectWithFlags(text_, ToastFont(), SK_ColorWHITE,
                                    gfx::ToEnclosingRect(pill),
                                    gfx::Canvas::TEXT_ALIGN_CENTER);
  }

  bool OnMousePressed(const ui::MouseEvent& event) override {
    dismiss_.Run();
    return true;
  }

 private:
  std::u16string text_;
  base::RepeatingClosure dismiss_;
};

BEGIN_METADATA(ToastView)
END_METADATA

class Toast {
 public:
  explicit Toast(BrowserView* browser_view) : browser_view_(browser_view) {
    overlay_ = std::make_unique<Overlay>(
        browser_view,
        std::make_unique<ToastView>(
            base::BindRepeating(&Toast::Hide, base::Unretained(this))),
        "MoriToast");
  }

  void Show(const std::u16string& text) {
    auto* view = static_cast<ToastView*>(overlay_->contents());
    views::View* area = browser_view_->contents_container();
    if (!view || !area) {
      return;
    }
    view->SetText(text);
    const gfx::Size pill = view->PillSize();
    const gfx::Rect page = views::View::ConvertRectToTarget(
        area, browser_view_, area->GetLocalBounds());
    overlay_->Show(gfx::Rect(page.right() - kInset - pill.width() - kShadow,
                             page.y() + kInset - kShadow,
                             pill.width() + 2 * kShadow,
                             pill.height() + 2 * kShadow));
    timer_.Start(FROM_HERE, kShowFor,
                 base::BindOnce(&Toast::Hide, base::Unretained(this)));
  }

  void Hide() {
    timer_.Stop();
    overlay_->Hide();
  }

 private:
  raw_ptr<BrowserView> browser_view_;
  std::unique_ptr<Overlay> overlay_;
  base::OneShotTimer timer_;
};

}  // namespace
}  // namespace mori

DEFINE_UI_CLASS_PROPERTY_TYPE(mori::Toast*)

namespace mori {
namespace {
DEFINE_OWNED_UI_CLASS_PROPERTY_KEY(Toast, kToastKey)
}  // namespace

void ShowToast(BrowserWindowInterface* browser, const std::u16string& text) {
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view || !browser_view->GetWidget()) {
    return;
  }
  Toast* toast = browser_view->GetProperty(kToastKey);
  if (!toast) {
    toast = browser_view->SetProperty(kToastKey,
                                      std::make_unique<Toast>(browser_view));
  }
  toast->Show(text);
}

}  // namespace mori
