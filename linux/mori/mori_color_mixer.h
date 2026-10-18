// Mori's colours: one surface for the sidebar and toolbar, with the page as
// a card on top (Zen Browser's look).

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_COLOR_MIXER_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_COLOR_MIXER_H_

#include "ui/color/color_provider_key.h"

namespace ui {
class ColorProvider;
}

namespace mori {

// Adds Mori's palette on top of Chromium's default theme. Installed themes
// and the GTK system theme are left alone.
void AddMoriColorMixer(ui::ColorProvider* provider,
                       const ui::ColorProviderKey& key);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_COLOR_MIXER_H_
