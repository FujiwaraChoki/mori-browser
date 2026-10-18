// Little Mori (Arc's Little Arc, Mori 3's Little Mori): links handed over
// by other apps open in a small window of their own, with a slim header of
// back, the address, "Open in <space>" and close. Opening the page in Mori
// moves it into the space on screen of the last window.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_LITTLE_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_LITTLE_H_

#include <memory>

class BrowserView;
class BrowserWindowInterface;
class GURL;
class Profile;
class ToolbarView;

namespace mori {

// Whether `url`, handed over by another app to `browser`'s profile, opens
// in Little Mori: the setting is on and no Air Traffic Control rule claims
// the site.
bool ShouldOpenInLittle(BrowserWindowInterface* browser, const GURL& url);

// A new Little Mori window for `url`.
void OpenLittle(Profile* profile, const GURL& url);

// The setting (Mori 3's "Open links from other apps in"): Little Mori, or
// the space on screen.
bool LinksOpenInLittle(Profile* profile);
void SetLinksOpenInLittle(Profile* profile, bool little);

// Called as a popup window's toolbar is built: in a Little Mori window, adds
// its header buttons.
void MaybeAddLittleHeader(ToolbarView* toolbar, BrowserView* browser_view);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_LITTLE_H_
