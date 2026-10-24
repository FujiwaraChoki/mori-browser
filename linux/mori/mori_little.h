// Little Mori (Arc's Little Arc, Mori's Little Mori): links handed over
// by other apps open in a small window of their own, with a slim header of
// navigation, a centred domain, and an "Open in Mori" window picker. Opening
// the page in Mori moves it to the top of the last window's tabs.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_LITTLE_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_LITTLE_H_

#include <memory>

#include "base/functional/callback_forward.h"

class BrowserView;
class BrowserWindowInterface;
class GURL;
class Profile;
class ToolbarView;

namespace gfx {
class Point;
}

namespace mori {

// Whether `url`, handed over by another app to `browser`'s profile, opens
// in Little Mori (the setting is on).
bool ShouldOpenInLittle(BrowserWindowInterface* browser, const GURL& url);

// A new Little Mori window for `url`.
void OpenLittle(Profile* profile, const GURL& url);

// The setting (Mori's "Open links from other apps in"): Little Mori, or
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

// True only for Mori's disposable link windows, including during construction.
bool IsLittleWindow(BrowserWindowInterface* browser);

// Empty header space drags the window; buttons remain clickable.
bool IsLittleHeaderCaption(BrowserView* browser_view, const gfx::Point& point);

// Move the live tab into the last active regular window. False for other
// windows.
bool OpenLittleInMori(BrowserWindowInterface* browser);

inline constexpr int kLittleHeaderHeight = 44;

// Called as a popup window's toolbar is built: in a Little Mori window, adds
// its header buttons.
void MaybeAddLittleHeader(ToolbarView* toolbar, BrowserView* browser_view);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_LITTLE_H_
