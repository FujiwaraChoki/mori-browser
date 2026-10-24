// Mori's library: a popover beside the
// sidebar with History, Bookmarks and Archive (recently closed tabs), a
// search field, and rows that open in the space on screen.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_LIBRARY_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_LIBRARY_H_

class BrowserWindowInterface;

namespace mori {

// The library button and Ctrl+Y.
void ToggleLibrary(BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_LIBRARY_H_
