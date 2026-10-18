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

struct Palette {
  SkColor window;          // Sidebar, toolbar and the margin around the page.
  SkColor selected;        // The active tab.
  SkColor hover;           // A hovered tab.
  SkColor field;           // The address field.
  SkColor field_hover;
  SkColor popover;         // Command bar and suggestion lists.
  SkColor foreground;
  SkColor foreground_dim;
};

constexpr Palette kDark = {
    .window = SkColorSetRGB(0x13, 0x13, 0x13),
    .selected = SkColorSetRGB(0x2A, 0x2A, 0x2A),
    .hover = SkColorSetRGB(0x1F, 0x1F, 0x1F),
    .field = SkColorSetRGB(0x22, 0x22, 0x22),
    .field_hover = SkColorSetRGB(0x2A, 0x2A, 0x2A),
    .popover = SkColorSetRGB(0x1B, 0x1B, 0x1D),
    .foreground = SkColorSetRGB(0xEE, 0xEE, 0xEF),
    .foreground_dim = SkColorSetRGB(0xA0, 0xA1, 0xA2),
};

constexpr Palette kLight = {
    .window = SkColorSetRGB(0xE9, 0xE9, 0xE9),
    .selected = SkColorSetRGB(0xFF, 0xFF, 0xFF),
    .hover = SkColorSetRGB(0xF2, 0xF2, 0xF2),
    .field = SkColorSetRGB(0xF4, 0xF4, 0xF4),
    .field_hover = SkColorSetRGB(0xFB, 0xFB, 0xFB),
    .popover = SkColorSetRGB(0xFB, 0xFB, 0xFB),
    .foreground = SkColorSetRGB(0x1D, 0x1F, 0x22),
    .foreground_dim = SkColorSetRGB(0x5A, 0x5C, 0x5E),
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

  // Tabs.
  mixer[kColorTabBackgroundActiveFrameActive] = {p.selected};
  mixer[kColorTabBackgroundActiveFrameInactive] = {p.selected};
  mixer[kColorTabBackgroundInactiveHoverFrameActive] = {p.hover};
  mixer[kColorTabBackgroundInactiveHoverFrameInactive] = {p.hover};
  mixer[kColorTabForegroundActiveFrameActive] = {p.foreground};
  mixer[kColorTabForegroundActiveFrameInactive] = {p.foreground};
  mixer[kColorTabForegroundInactiveFrameActive] = {p.foreground_dim};
  mixer[kColorTabForegroundInactiveFrameInactive] = {p.foreground_dim};

  // Address field and suggestions.
  mixer[kColorLocationBarBackground] = {p.field};
  mixer[kColorLocationBarBackgroundHovered] = {p.field_hover};
  mixer[kColorOmniboxResultsBackground] = {p.popover};
  mixer[kColorOmniboxResultsBackgroundSelected] = {p.selected};
  mixer[kColorOmniboxText] = {p.foreground};
  mixer[kColorOmniboxResultsTextDimmed] = {p.foreground_dim};
}

}  // namespace mori
