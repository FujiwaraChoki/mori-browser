// Mori 3's new tab page (root.rs render_new_tab_page): the faint Mori logo
// and a "Search or enter address" field that opens the command bar, drawn
// over Chromium's new tab page.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_NEW_TAB_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_NEW_TAB_H_

class BrowserWindowInterface;
class GURL;

namespace mori {

// Starts showing Mori's page whenever `browser`'s tab on screen is a new
// tab page.
void WatchNewTabPages(BrowserWindowInterface* browser);

// Chromium's new tab pages, which Mori's covers.
bool IsNewTabPage(const GURL& url);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_NEW_TAB_H_
