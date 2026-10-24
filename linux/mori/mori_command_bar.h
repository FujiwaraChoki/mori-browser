// Mori's command bar: a floating search field over the page that opens
// URLs, searches and open tabs (Arc's Ctrl+T / Ctrl+L).

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_COMMAND_BAR_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_COMMAND_BAR_H_

class BrowserWindowInterface;

namespace mori {

enum class CommandBarMode {
  // Ctrl+T: the result opens in a new tab.
  kNewTab,
  // Ctrl+L: starts with the current URL; the result replaces the page.
  kCurrentTab,
};

// Shows the command bar in `browser`'s window. Returns false when the window
// has no command bar (popups, apps), so the caller can fall back to
// Chromium's own behaviour.
bool ShowCommandBar(BrowserWindowInterface* browser, CommandBarMode mode);

// Whether `browser`'s address field opens the command bar instead of taking
// focus: Mori's windows, as in Mori, where the field is only a display.
bool OpensCommandBarFromAddressField(BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_COMMAND_BAR_H_
