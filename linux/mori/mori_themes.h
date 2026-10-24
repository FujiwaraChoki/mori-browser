// Mori's theme (Arc's and Mori's gradient themes): a two-colour gradient
// that washes over the window's chrome. One per profile, chosen in
// Settings, kept in "Mori Theme.json".

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_THEMES_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_THEMES_H_

#include <array>
#include <optional>
#include <utility>

#include "base/callback_list.h"
#include "third_party/skia/include/core/SkColor.h"

class BrowserView;
class BrowserWindowInterface;

namespace gfx {
class Canvas;
}  // namespace gfx

namespace views {
class View;
}

namespace mori {

// The theme's gradient; none is the plain chrome.
using ThemeColors = std::optional<std::pair<SkColor, SkColor>>;

// Mori's colour pairs (workspace.rs NEW_SPACE_COLORS).
inline constexpr std::array<std::pair<SkColor, SkColor>, 8> kThemeGradients = {{
    {SkColorSetRGB(0x6E, 0x5B, 0xD8), SkColorSetRGB(0x9B, 0x5B, 0xD8)},
    {SkColorSetRGB(0x35, 0x77, 0xD4), SkColorSetRGB(0x5B, 0x5B, 0xD8)},
    {SkColorSetRGB(0x1F, 0x94, 0x88), SkColorSetRGB(0x2F, 0x78, 0xA8)},
    {SkColorSetRGB(0xD9, 0x70, 0x2E), SkColorSetRGB(0xC9, 0x4E, 0x5A)},
    {SkColorSetRGB(0xD2, 0x4E, 0x7C), SkColorSetRGB(0x9C, 0x4E, 0xD2)},
    {SkColorSetRGB(0x4F, 0x9A, 0x3C), SkColorSetRGB(0x2C, 0x8C, 0x78)},
    {SkColorSetRGB(0xC4, 0x8A, 0x1E), SkColorSetRGB(0xC4, 0x5A, 0x1E)},
    {SkColorSetRGB(0x8E, 0x5B, 0xD0), SkColorSetRGB(0xC8, 0x5B, 0xA8)},
}};

ThemeColors GetTheme(BrowserWindowInterface* browser);
void SetTheme(BrowserWindowInterface* browser, ThemeColors colors);

// Runs when the theme changes (or the saved theme is read).
base::CallbackListSubscription AddThemeObserver(BrowserWindowInterface* browser,
                                                base::RepeatingClosure closure);

// Paints the theme's wash over a frame or toolbar background that `view`
// just painted (called from ThemedBackground).
void PaintThemeWash(gfx::Canvas* canvas,
                    const views::View* view,
                    const BrowserView* browser_view);

// Repaints the chrome of `browser`'s window, for a new theme.
void RepaintThemeWash(BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_THEMES_H_
