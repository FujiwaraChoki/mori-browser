// What makes a space look like itself (Arc's and Mori 3's space themes): a
// symbol (a Lucide icon or an emoji) and a two-colour gradient that washes
// over the window's chrome while the space is on screen. Kept per profile
// in "Mori Spaces.json", by the space's saved tab group.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_SPACE_THEMES_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_SPACE_THEMES_H_

#include <array>
#include <optional>
#include <string>
#include <string_view>
#include <utility>

#include "base/callback_list.h"
#include "components/tab_groups/tab_group_id.h"
#include "third_party/skia/include/core/SkColor.h"

class BrowserView;
class BrowserWindowInterface;

namespace gfx {
class Canvas;
class Rect;
}  // namespace gfx

namespace views {
class View;
}

namespace mori {

struct SpaceTheme {
  // A Lucide icon's name or an emoji; empty is the plain dot.
  std::string symbol;
  // The gradient; none is the plain chrome.
  std::optional<std::pair<SkColor, SkColor>> colors;

  bool operator==(const SpaceTheme&) const = default;
};

// Mori 3's colour pairs for spaces (workspace.rs NEW_SPACE_COLORS).
inline constexpr std::array<std::pair<SkColor, SkColor>, 8> kSpaceGradients = {{
    {SkColorSetRGB(0x6E, 0x5B, 0xD8), SkColorSetRGB(0x9B, 0x5B, 0xD8)},
    {SkColorSetRGB(0x35, 0x77, 0xD4), SkColorSetRGB(0x5B, 0x5B, 0xD8)},
    {SkColorSetRGB(0x1F, 0x94, 0x88), SkColorSetRGB(0x2F, 0x78, 0xA8)},
    {SkColorSetRGB(0xD9, 0x70, 0x2E), SkColorSetRGB(0xC9, 0x4E, 0x5A)},
    {SkColorSetRGB(0xD2, 0x4E, 0x7C), SkColorSetRGB(0x9C, 0x4E, 0xD2)},
    {SkColorSetRGB(0x4F, 0x9A, 0x3C), SkColorSetRGB(0x2C, 0x8C, 0x78)},
    {SkColorSetRGB(0xC4, 0x8A, 0x1E), SkColorSetRGB(0xC4, 0x5A, 0x1E)},
    {SkColorSetRGB(0x8E, 0x5B, 0xD0), SkColorSetRGB(0xC8, 0x5B, 0xA8)},
}};

SpaceTheme GetSpaceTheme(BrowserWindowInterface* browser,
                         const tab_groups::TabGroupId& space);
void SetSpaceTheme(BrowserWindowInterface* browser,
                   const tab_groups::TabGroupId& space,
                   const SpaceTheme& theme);

// Runs when any space's theme changes (or the saved themes are read).
base::CallbackListSubscription AddSpaceThemeObserver(
    BrowserWindowInterface* browser,
    base::RepeatingClosure closure);

// Paints the current space's wash over a frame or toolbar background that
// `view` just painted (called from ThemedBackground).
void PaintSpaceWash(gfx::Canvas* canvas,
                    const views::View* view,
                    const BrowserView* browser_view);

// Repaints the chrome of `browser`'s window, for a new space or theme.
void RepaintSpaceWash(BrowserWindowInterface* browser);

// Whether `symbol` is an emoji (drawn as text) rather than a Lucide name.
bool IsEmojiSymbol(std::string_view symbol);

// A space symbol, `size` px, centred in `bounds`.
void PaintSpaceSymbol(gfx::Canvas* canvas,
                      std::string_view symbol,
                      const gfx::Rect& bounds,
                      int size,
                      SkColor color);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_SPACE_THEMES_H_
