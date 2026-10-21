// Boosts (Arc's Boosts, Mori 3's Boost this site): a site's own CSS, its own
// JavaScript and elements the user zapped away, applied whenever a page of
// that site (or its subdomains) loads. Kept in the profile's
// "Mori Boosts.json".

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_BOOSTS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_BOOSTS_H_

#include <string>
#include <vector>

class BrowserWindowInterface;
class Profile;

namespace content {
class WebContents;
}

namespace mori {

// Lets `contents` (a tab) apply boosts; called as tab helpers attach.
void AttachBoosts(content::WebContents* contents);

// Opens the boost editor for the site of `browser`'s active tab (Ctrl+Shift+B).
void EditBoost(BrowserWindowInterface* browser);

// Zap mode on `browser`'s active tab: elements clicked go away, also on later
// visits (the tab menu's Zap an Element).
void ZapElement(BrowserWindowInterface* browser);

// The sites with a boost, and removing one (for settings).
std::vector<std::string> ListBoostedSites(Profile* profile);
void RemoveBoost(Profile* profile, const std::string& host);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_BOOSTS_H_
