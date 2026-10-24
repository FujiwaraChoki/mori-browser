// Peek (Arc's Peek, Zen's Glance, Mori's Peek): Alt+click on a link shows
// it in a large card over the page. The page behind dims, and round buttons
// beside the card close it, open it as a tab, or open it in a split.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_PEEK_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_PEEK_H_

class BrowserWindowInterface;
class GURL;

namespace mori {

// Shows `url` in Peek over `browser`'s page. False when the window has no
// Peek (an app or popup window).
bool ShowPeek(BrowserWindowInterface* browser, const GURL& url);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_PEEK_H_
