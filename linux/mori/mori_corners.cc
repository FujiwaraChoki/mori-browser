#include "chrome/browser/ui/views/mori/mori_corners.h"

#include <algorithm>

#include "third_party/skia/include/core/SkPathBuilder.h"
#include "third_party/skia/include/core/SkRect.h"

namespace mori {
namespace {

// Zen's --zen-squircle-value on Linux is 1.3. CSS's superellipse(K) is the
// curve |x|^n + |y|^n = 1 with n = 2^K (K = 1 is a circle). Each corner is
// one cubic through the curve's 45-degree point (m, m), m = 2^(-1/n), which
// puts its control points (8m - 4) / 3 of the radius from its ends; these
// are the rest of the radius, measured from the corner:
//   1 - (8m - 4) / 3 for K = 1.3, and for a circle (0.552 from the ends).
constexpr float kSmooth = 0.32094f;
constexpr float kCircle = 0.44772f;

// Zen also lengthens its radii by the same 1.3 (calc(10px *
// var(--zen-squircle-value))): a superellipse hugs the corner more than an
// arc, so a longer one cuts about as deep at 45 degrees and eases in from
// further along each edge.
constexpr float kRadiusScale = 1.3f;

}  // namespace

SkPath SmoothRRectPath(const SkRRect& rrect) {
  if (rrect.isEmpty() || rrect.isRect() || rrect.isOval()) {
    return SkPath::RRect(rrect);
  }
  const SkRect& r = rrect.rect();

  // A corner that spans half the rect's side is a pill's end: it stays an
  // arc. Others get the superellipse, on a longer radius.
  auto is_pill = [&r](const SkVector& radii) {
    return 2 * radii.x() >= r.width() - 0.5f ||
           2 * radii.y() >= r.height() - 0.5f;
  };
  auto corner = [&](SkRRect::Corner which, float* share) {
    const SkVector radii = rrect.radii(which);
    if (is_pill(radii)) {
      *share = kCircle;
      return radii;
    }
    *share = kSmooth;
    return SkVector::Make(
        std::min(radii.x() * kRadiusScale, r.width() / 2),
        std::min(radii.y() * kRadiusScale, r.height() / 2));
  };
  float s_ul, s_ur, s_lr, s_ll;
  const SkVector ul = corner(SkRRect::kUpperLeft_Corner, &s_ul);
  const SkVector ur = corner(SkRRect::kUpperRight_Corner, &s_ur);
  const SkVector lr = corner(SkRRect::kLowerRight_Corner, &s_lr);
  const SkVector ll = corner(SkRRect::kLowerLeft_Corner, &s_ll);

  SkPathBuilder path;
  path.moveTo(r.fLeft + ul.x(), r.fTop);
  path.lineTo(r.fRight - ur.x(), r.fTop);
  path.cubicTo(r.fRight - ur.x() * s_ur, r.fTop, r.fRight,
               r.fTop + ur.y() * s_ur, r.fRight, r.fTop + ur.y());
  path.lineTo(r.fRight, r.fBottom - lr.y());
  path.cubicTo(r.fRight, r.fBottom - lr.y() * s_lr, r.fRight - lr.x() * s_lr,
               r.fBottom, r.fRight - lr.x(), r.fBottom);
  path.lineTo(r.fLeft + ll.x(), r.fBottom);
  path.cubicTo(r.fLeft + ll.x() * s_ll, r.fBottom, r.fLeft,
               r.fBottom - ll.y() * s_ll, r.fLeft, r.fBottom - ll.y());
  path.lineTo(r.fLeft, r.fTop + ul.y());
  path.cubicTo(r.fLeft, r.fTop + ul.y() * s_ul, r.fLeft + ul.x() * s_ul,
               r.fTop, r.fLeft + ul.x(), r.fTop);
  path.close();
  return path.detach();
}

SkPath SmoothRoundRectPath(const SkRect& rect, SkScalar radius) {
  return SmoothRRectPath(SkRRect::MakeRectXY(rect, radius, radius));
}

}  // namespace mori
