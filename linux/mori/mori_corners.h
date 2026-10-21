// Zen's corners: every rounded rect in Zen is drawn with
// `corner-shape: superellipse(1.3)` (zen-theme.css), which eases each
// corner into its edges instead of meeting them with a circular arc. Mori
// draws Chromium's rounded rects (Canvas::DrawRoundRect, rounded
// backgrounds, painters, highlights, menus, tab rows) the same way.
//
// Compiled into ui/gfx, like mori_icons, so ui/views can use it too.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_CORNERS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_CORNERS_H_

#include "base/component_export.h"
#include "third_party/skia/include/core/SkPath.h"
#include "third_party/skia/include/core/SkRRect.h"
#include "third_party/skia/include/core/SkScalar.h"

struct SkRect;

namespace mori {

// `rrect` with Zen's superellipse corners, on radii 1.3 times as long (as
// Zen lengthens them), up to half the rect's side. Corners that make a
// circle or the round end of a pill stay circular, as Zen keeps them
// (squircle-exclusions.inc.css).
COMPONENT_EXPORT(GFX) SkPath SmoothRRectPath(const SkRRect& rrect);

// The same for a rect whose corners all have `radius`.
COMPONENT_EXPORT(GFX)
SkPath SmoothRoundRectPath(const SkRect& rect, SkScalar radius);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_CORNERS_H_
