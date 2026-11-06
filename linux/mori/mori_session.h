// How Mori's session and Chromium's reopen-closed list stay consistent.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_SESSION_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_SESSION_H_

#include <memory>
#include <vector>

class Profile;

namespace sessions {
struct SessionWindow;
}

namespace mori {

// Closing a window adds it to the reopen-closed list, and restarting then
// restores that same window from the session. Left alone, Reopen Closed Tab
// would open a second copy of it, and only then the tab really closed last.
// Call this when a session restore is about to reopen `windows`: it drops the
// list's window entries holding the same pages. Other entries are untouched, so
// without a restore (a "New Tab" startup) the list still has the window.
void ForgetRestoredWindowEntries(
    Profile* profile,
    const std::vector<std::unique_ptr<sessions::SessionWindow>>& windows);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_SESSION_H_
