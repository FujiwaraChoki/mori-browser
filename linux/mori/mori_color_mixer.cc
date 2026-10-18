// The palette matches Mori 3's (mori-core/src/theme.rs), which was sampled
// from Zen Browser: neutral greys, a #131313 window in dark mode and #E9E9E9
// in light mode.

#include "chrome/browser/ui/views/mori/mori_color_mixer.h"

#include "chrome/browser/ui/color/chrome_color_id.h"
#include "third_party/skia/include/core/SkColor.h"
#include "ui/color/color_id.h"
#include "ui/color/color_mixer.h"
#include "ui/color/color_provider.h"
#include "ui/color/color_recipe.h"

namespace mori {
namespace {

// The sidebar's surfaces are translucent, as in Mori 3, so a space's
// gradient wash shows through them; over the plain window they come out as
// the greys Zen uses.
struct Palette {
  SkColor window;          // Sidebar, toolbar and the margin around the page.
  SkColor selected;        // The active tab.
  SkColor hover;           // A hovered tab.
  SkColor tile;            // A favorite at rest.
  SkColor field;           // The address field.
  SkColor field_hover;
  SkColor popover;         // Command bar and suggestion lists.
  SkColor card;            // An empty page (the new tab page).
  SkColor foreground;
  SkColor foreground_dim;
  SkColor menu;                   // Menus (Zen's #1f1f1f / #f4f4f4).
  SkColor menu_border;
  SkColor menu_hover;
  SkColor menu_separator;
  SkColor menu_icon;
  SkColor menu_secondary;
  SkColor toolbar_icon;           // Zen's toolbar greys, as in Mori 3.
  SkColor toolbar_icon_disabled;
};

constexpr Palette kDark = {
    .window = SkColorSetRGB(0x13, 0x13, 0x13),
    // Mori 3: white at 17% (selected), 7% (hover), 12% (pill and tiles).
    .selected = SkColorSetA(SK_ColorWHITE, 0x2B),
    .hover = SkColorSetA(SK_ColorWHITE, 0x12),
    .tile = SkColorSetA(SK_ColorWHITE, 0x1F),
    .field = SkColorSetA(SK_ColorWHITE, 0x1F),
    .field_hover = SkColorSetA(SK_ColorWHITE, 0x2B),
    .popover = SkColorSetRGB(0x1B, 0x1B, 0x1D),
    .card = SkColorSetRGB(0x2B, 0x2B, 0x2B),
    .foreground = SkColorSetRGB(0xEE, 0xEE, 0xEF),
    .foreground_dim = SkColorSetRGB(0xA0, 0xA1, 0xA2),
    .menu = SkColorSetRGB(0x1F, 0x1F, 0x1F),
    .menu_border = SkColorSetRGB(0x4C, 0x4C, 0x4C),
    .menu_hover = SkColorSetRGB(0x35, 0x35, 0x35),
    .menu_separator = SkColorSetRGB(0x31, 0x31, 0x31),
    .menu_icon = SkColorSetRGB(0x9B, 0x9B, 0x9B),
    .menu_secondary = SkColorSetRGB(0x8E, 0x8F, 0x90),
    .toolbar_icon = SkColorSetRGB(0xB9, 0xB9, 0xB9),
    .toolbar_icon_disabled = SkColorSetRGB(0x5F, 0x5F, 0x5F),
};

constexpr Palette kLight = {
    .window = SkColorSetRGB(0xE9, 0xE9, 0xE9),
    // Mori 3: white at 72% (selected), black at 5% (hover), 7% (pill and
    // tiles).
    .selected = SkColorSetA(SK_ColorWHITE, 0xB8),
    .hover = SkColorSetA(SK_ColorBLACK, 0x0D),
    .tile = SkColorSetA(SK_ColorBLACK, 0x12),
    .field = SkColorSetA(SK_ColorBLACK, 0x12),
    .field_hover = SkColorSetA(SK_ColorBLACK, 0x1A),
    .popover = SkColorSetRGB(0xFB, 0xFB, 0xFB),
    .card = SkColorSetRGB(0xFB, 0xFB, 0xFB),
    .foreground = SkColorSetRGB(0x1D, 0x1F, 0x22),
    .foreground_dim = SkColorSetRGB(0x5A, 0x5C, 0x5E),
    .menu = SkColorSetRGB(0xF4, 0xF4, 0xF4),
    .menu_border = SkColorSetRGB(0xC5, 0xC5, 0xC6),
    .menu_hover = SkColorSetRGB(0xE3, 0xE3, 0xE3),
    .menu_separator = SkColorSetRGB(0xE0, 0xE0, 0xE0),
    .menu_icon = SkColorSetRGB(0x76, 0x77, 0x79),
    .menu_secondary = SkColorSetRGB(0x6B, 0x6D, 0x6F),
    .toolbar_icon = SkColorSetRGB(0x69, 0x69, 0x6C),
    .toolbar_icon_disabled = SkColorSetRGB(0xAE, 0xAE, 0xB0),
};

}  // namespace

void AddMoriColorMixer(ui::ColorProvider* provider,
                       const ui::ColorProviderKey& key) {
  if (key.custom_theme ||
      key.system_theme != ui::SystemTheme::kDefault) {
    return;
  }
  const bool dark = key.color_mode == ui::ColorProviderKey::ColorMode::kDark;
  const Palette& p = dark ? kDark : kLight;
  ui::ColorMixer& mixer = provider->AddMixer();

  // One surface for the window chrome.
  mixer[ui::kColorFrameActive] = {p.window};
  mixer[ui::kColorFrameInactive] = {p.window};
  mixer[kColorToolbar] = {p.window};
  mixer[kColorToolbarContentAreaSeparator] = {SK_ColorTRANSPARENT};
  mixer[kColorToolbarSeparator] = {SK_ColorTRANSPARENT};
  mixer[kColorTabDividerFrameActive] = {SK_ColorTRANSPARENT};
  mixer[kColorTabDividerFrameInactive] = {SK_ColorTRANSPARENT};
  // The sidebar's trailing outline.
  mixer[kColorVerticalTabStripShadow] = {SK_ColorTRANSPARENT};

  // Menus.
  mixer[ui::kColorMenuBackground] = {p.menu};
  mixer[ui::kColorMenuBorder] = {p.menu_border};
  mixer[ui::kColorMenuItemBackgroundSelected] = {p.menu_hover};
  mixer[ui::kColorMenuItemBackgroundHighlighted] = {p.menu_hover};
  mixer[ui::kColorMenuSelectionBackground] = {p.menu_hover};
  mixer[ui::kColorMenuItemForeground] = {p.foreground};
  mixer[ui::kColorMenuItemForegroundSelected] = {p.foreground};
  mixer[ui::kColorMenuItemForegroundHighlighted] = {p.foreground};
  mixer[ui::kColorMenuItemForegroundSecondary] = {p.menu_secondary};
  mixer[ui::kColorMenuSeparator] = {p.menu_separator};
  mixer[ui::kColorMenuIcon] = {p.menu_icon};

  mixer[kColorToolbarButtonIcon] = {p.toolbar_icon};
  mixer[kColorToolbarButtonIconDefault] = {p.toolbar_icon};
  mixer[kColorToolbarButtonIconHovered] = {p.foreground};
  mixer[kColorToolbarButtonIconPressed] = {p.foreground};
  mixer[kColorToolbarButtonIconInactive] = {p.toolbar_icon_disabled};
  mixer[kColorToolbarButtonIconDisabled] = {p.toolbar_icon_disabled};

  // Tabs. Like Arc, only the active and hovered tabs get a fill.
  mixer[kColorTabBackgroundInactiveFrameActive] = {p.tile};
  mixer[kColorTabBackgroundInactiveFrameInactive] = {p.tile};
  mixer[kColorTabBackgroundSelectedFrameActive] = {p.hover};
  mixer[kColorTabBackgroundSelectedFrameInactive] = {p.hover};
  mixer[kColorTabBackgroundSelectedHoverFrameActive] = {p.hover};
  mixer[kColorTabBackgroundSelectedHoverFrameInactive] = {p.hover};
  mixer[kColorNewTabButtonCRBackgroundFrameActive] = {p.window};
  mixer[kColorNewTabButtonCRBackgroundFrameInactive] = {p.window};
  mixer[kColorTabBackgroundActiveFrameActive] = {p.selected};
  mixer[kColorTabBackgroundActiveFrameInactive] = {p.selected};
  mixer[kColorTabBackgroundInactiveHoverFrameActive] = {p.hover};
  mixer[kColorTabBackgroundInactiveHoverFrameInactive] = {p.hover};
  mixer[kColorTabForegroundActiveFrameActive] = {p.foreground};
  mixer[kColorTabForegroundActiveFrameInactive] = {p.foreground};
  mixer[kColorTabForegroundInactiveFrameActive] = {p.foreground};
  mixer[kColorTabForegroundInactiveFrameInactive] = {p.foreground};

  mixer[kColorNewTabPageBackground] = {p.card};

  // Address field and suggestions.
  // The site icon sits in the pill without Chromium's round chip.
  // The chip sits on the field, which is translucent; a second coat of it
  // would show.
  mixer[kColorOmniboxIconBackground] = {SK_ColorTRANSPARENT};
  mixer[kColorOmniboxIconBackgroundTonal] = {SK_ColorTRANSPARENT};
  mixer[kColorOmniboxIconForeground] = {p.toolbar_icon};
  mixer[kColorOmniboxIconForegroundTonal] = {p.toolbar_icon};
  mixer[kColorLocationBarBackground] = {p.field};
  mixer[kColorLocationBarBackgroundHovered] = {p.field_hover};
  mixer[kColorOmniboxResultsBackground] = {p.popover};
  mixer[kColorOmniboxResultsBackgroundSelected] = {p.selected};
  mixer[kColorOmniboxText] = {p.foreground};
  mixer[kColorOmniboxResultsTextDimmed] = {p.foreground_dim};
}

}  // namespace mori
