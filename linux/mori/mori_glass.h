// Apple's Liquid Glass for Mori's sidebar, as in Mori (theme.rs): a clear
// fill, a bright rim where the glass bends light at its edge, and a line of
// light along its top. Three materials:
//
// - kRaised: a capsule of glass standing on the sidebar (the toolbar's
//   button groups);
// - kInset: a control set into the sidebar (the address field, favorites),
//   whose fill its owner paints; only the rim is added;
// - kLens: the selected tab, a lens of brighter glass.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_GLASS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_GLASS_H_

class SkPath;

namespace gfx {
class Canvas;
}

namespace views {
class View;
}

namespace mori {

enum class Glass { kRaised, kInset, kLens };

// Paints `glass` in `path`, which is in `canvas`'s current coordinates.
// `dark` picks the dark-mode values.
void PaintGlass(gfx::Canvas* canvas, const SkPath& path, Glass glass,
                bool dark);

// Whether `view` is drawn in dark mode (Mori's palette: light text).
bool IsDarkChrome(const views::View* view);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_GLASS_H_
