// Air Traffic Control (Arc's, Mori 3's): rules that send pages of a site
// opened from other apps into a chosen space. Kept per profile in
// "Mori Routes.json".

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_ROUTES_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_ROUTES_H_

#include <optional>
#include <string>
#include <vector>

class BrowserWindowInterface;
class GURL;

namespace content {
class WebContents;
}

namespace mori {

struct Route {
  // A site, lowercased and without "www."; covers its subdomains.
  std::string host;
  // The space's key (see SpaceKey()).
  std::string space;
};

// The rule for `url`, the most specific one if several cover it.
std::optional<Route> RouteFor(BrowserWindowInterface* browser,
                              const GURL& url);

// Pages of `url`'s site go to the space on screen from now on.
void RouteSiteToCurrentSpace(BrowserWindowInterface* browser, const GURL& url);
void RemoveRoute(BrowserWindowInterface* browser, const std::string& host);

// Every rule, oldest first (for settings).
std::vector<Route> ListRoutes(BrowserWindowInterface* browser);

// A tab opened from another app (`first` is the one shown): moves it into
// its rule's space and, for the first, shows that space.
void RouteExternalTab(BrowserWindowInterface* browser,
                      content::WebContents* contents,
                      bool first);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_ROUTES_H_
