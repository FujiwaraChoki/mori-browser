// Ctrl+/: every shortcut, grouped and searchable (Mori 3's cheatsheet). The
// keys come from the window's live bindings, so they never drift.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_CHEATSHEET_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_CHEATSHEET_H_

class BrowserWindowInterface;

namespace mori {

void ToggleCheatsheet(BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_CHEATSHEET_H_
