// Little Mori (Arc's Little Arc, Mori 3's Little Mori): links handed over
// by other apps open in a small window of their own, with a slim header of
// back, the address, "Open in Mori" and close. Opening the page in Mori
// moves it to the top of the last window's tabs.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_LITTLE_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_LITTLE_H_

#include <memory>

#include "base/functional/callback_forward.h"

class BrowserView;
class BrowserWindowInterface;
class GURL;
class Profile;
class ToolbarView;

namespace mori {

// Whether `url`, handed over by another app to `browser`'s profile, opens
// in Little Mori (the setting is on).
bool ShouldOpenInLittle(BrowserWindowInterface* browser, const GURL& url);

// A new Little Mori window for `url`.
void OpenLittle(Profile* profile, const GURL& url);

// The setting (Mori 3's "Open links from other apps in"): Little Mori, or
// a tab of the last window.
bool LinksOpenInLittle(Profile* profile);
void SetLinksOpenInLittle(Profile* profile, bool little);

// Kept in the same file: hours a tab may sit unseen before Mori archives
// (closes) it, Arc's auto-archive; 0 turns it off.
int ArchiveAfterHours(Profile* profile);
void SetArchiveAfterHours(Profile* profile, int hours);

// Runs `closure` once these settings of `profile` are read.
void WhenMoriSettingsLoaded(Profile* profile, base::OnceClosure closure);

// Whether the tabs Mori kept in spaces (tab groups) came out of them.
bool SpacesUngrouped(Profile* profile);
void SetSpacesUngrouped(Profile* profile);

// Called as a popup window's toolbar is built: in a Little Mori window, adds
// its header buttons.
void MaybeAddLittleHeader(ToolbarView* toolbar, BrowserView* browser_view);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_LITTLE_H_
