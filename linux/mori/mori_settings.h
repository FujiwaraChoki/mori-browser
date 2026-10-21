// Mori 3's settings (crates/mori/src/ui/settings.rs): a sheet over the page
// with a rail of categories, in place of Chromium's settings page. Every
// control reads and writes Chromium's own preferences (or Mori's, for
// Mori-only features); "All Settings" opens Chromium's full page for
// what Mori doesn't show.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_SETTINGS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_SETTINGS_H_

class BrowserWindowInterface;

namespace mori {

// Ctrl+, and the menus' Settings. False when `browser` isn't a Mori window
// (Chromium's page opens instead).
bool ShowSettings(BrowserWindowInterface* browser);

// The settings on their Appearance category, where the theme is.
void ShowThemeSettings(BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_SETTINGS_H_
