// Ctrl+Tab in most-recently-used order, as in Arc and Mori: hold Ctrl and
// press Tab to move through an overlay of recent tabs, release Ctrl to go to
// the highlighted one. A quick tap switches without showing the overlay.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_TAB_SWITCHER_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_TAB_SWITCHER_H_

class BrowserWindowInterface;

namespace mori {

// Ctrl+Tab (`forward`) or Ctrl+Shift+Tab: starts the switcher or moves its
// highlight.
void StepTabSwitcher(BrowserWindowInterface* browser, bool forward);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_TAB_SWITCHER_H_
