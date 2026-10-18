// Boosts (Arc's Boosts, Mori 3's Boost this site): a site's own CSS, its own
// JavaScript and elements the user zapped away, applied whenever a page of
// that site (or its subdomains) loads. Kept in the profile's
// "Mori Boosts.json".

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_BOOSTS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_BOOSTS_H_

class BrowserWindowInterface;

namespace content {
class WebContents;
}

namespace mori {

// Lets `contents` (a tab) apply boosts; called as tab helpers attach.
void AttachBoosts(content::WebContents* contents);

// Opens the boost editor for the site of `browser`'s active tab (Ctrl+Shift+B).
void EditBoost(BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_BOOSTS_H_
