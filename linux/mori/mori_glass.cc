#include "chrome/browser/ui/views/mori/mori_glass.h"

#include "cc/paint/paint_filter.h"
#include "cc/paint/paint_flags.h"
#include "cc/paint/paint_shader.h"
#include "chrome/browser/ui/color/chrome_color_id.h"
#include "third_party/skia/include/core/SkColor.h"
#include "third_party/skia/include/core/SkPath.h"
#include "third_party/skia/include/core/SkPoint.h"
#include "third_party/skia/include/pathops/SkPathOps.h"
#include "ui/color/color_provider.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/scoped_canvas.h"
#include "ui/views/view.h"

namespace mori {
namespace {

SkColor White(float alpha) {
  return SkColorSetA(SK_ColorWHITE, static_cast<U8CPU>(alpha * 255));
}

SkColor Black(float alpha) {
  return SkColorSetA(SK_ColorBLACK, static_cast<U8CPU>(alpha * 255));
}

// A 1px line just inside `path`'s edge.
void PaintRing(gfx::Canvas* canvas, const SkPath& path, SkColor color) {
  gfx::ScopedCanvas scoped(canvas);
  canvas->ClipPath(path, /*do_anti_alias=*/true);
  cc::PaintFlags flags;
  flags.setAntiAlias(true);
  flags.setStyle(cc::PaintFlags::kStroke_Style);
  flags.setStrokeWidth(2);
  flags.setColor(color);
  canvas->DrawPath(path, flags);
}

// Light along `path`'s top edge, `depth` deep, thinning out around its
// corners: the part of the shape its copy `depth` lower doesn't cover.
void PaintTopLight(gfx::Canvas* canvas,
                   const SkPath& path,
                   SkColor color,
                   float depth) {
  SkPath lower = path.makeOffset(0, depth);
  SkPath band;
  if (!Op(path, lower, kDifference_SkPathOp, &band)) {
    return;
  }
  cc::PaintFlags flags;
  flags.setAntiAlias(true);
  flags.setColor(color);
  canvas->DrawPath(band, flags);
}

}  // namespace

void PaintGlass(gfx::Canvas* canvas,
                const SkPath& path,
                Glass glass,
                bool dark) {
  const SkRect bounds = path.getBounds();
  switch (glass) {
    case Glass::kRaised: {
      // A soft shadow under the capsule, then its clear fill.
      cc::PaintFlags shadow;
      shadow.setAntiAlias(true);
      shadow.setColor(Black(dark ? 0.22f : 0.07f));
      shadow.setImageFilter(sk_make_sp<cc::BlurPaintFilter>(
          1.5f, 1.5f, SkTileMode::kDecal, nullptr));
      canvas->DrawPath(path.makeOffset(0, 1), shadow);
      cc::PaintFlags fill;
      fill.setAntiAlias(true);
      fill.setColor(dark ? White(0.09f) : White(0.55f));
      canvas->DrawPath(path, fill);
      PaintRing(canvas, path, dark ? White(0.10f) : White(0.95f));
      PaintTopLight(canvas, path, dark ? White(0.14f) : White(1.0f), 1);
      break;
    }
    case Glass::kInset:
      PaintRing(canvas, path, dark ? White(0.06f) : White(0.30f));
      PaintTopLight(canvas, path, dark ? White(0.07f) : White(0.50f), 1);
      break;
    case Glass::kLens: {
      // Brighter at the top, as light falls on it from above.
      const SkPoint points[] = {{bounds.x(), bounds.y()},
                                {bounds.x(), bounds.bottom()}};
      const SkColor4f colors[] = {
          SkColor4f::FromColor(dark ? White(0.22f) : White(0.97f)),
          SkColor4f::FromColor(dark ? White(0.13f) : White(0.80f))};
      cc::PaintFlags fill;
      fill.setAntiAlias(true);
      fill.setShader(cc::PaintShader::MakeLinearGradient(points, colors,
                                                         nullptr, 2,
                                                         SkTileMode::kClamp));
      canvas->DrawPath(path, fill);
      // The rim: a hairline that holds the lens's shape over a pale sidebar,
      // a light one on a dark sidebar. Then light along its top.
      PaintRing(canvas, path, dark ? White(0.12f) : Black(0.08f));
      PaintTopLight(canvas, path, dark ? White(0.22f) : White(1.0f), 1);
      break;
    }
  }
}

bool IsDarkChrome(const views::View* view) {
  const ui::ColorProvider* provider = view->GetColorProvider();
  if (!provider) {
    return false;
  }
  // Mori's palette draws light text on dark chrome.
  return !color_utils::IsDark(
      provider->GetColor(kColorTabForegroundActiveFrameActive));
}

}  // namespace mori
