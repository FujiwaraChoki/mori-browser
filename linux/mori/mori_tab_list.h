// The sidebar around Chromium's list of tabs: "Add Tab" between the
// favorites and the tabs, and the bar at the bottom (library, assistant).
// The bar also runs Arc's auto-archive and Mori's new tab page.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_TAB_LIST_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_TAB_LIST_H_

#include <memory>

class BrowserWindowInterface;

namespace views {
class View;
}

namespace mori {

// Where a tab Mori opens goes: the top of the tabs, under the favorites, as
// in Arc and Mori 3.
int IndexForNewTab(BrowserWindowInterface* browser);

// Arc's "Archive Today" (Mori 3's Clear): closes every tab that isn't a
// favorite, leaving a new tab. They stay in History and Reopen Closed Tab.
void ArchiveUnpinnedTabs(BrowserWindowInterface* browser);

// The header between the favorites and the tabs: a hairline under the
// favorites and an "Add Tab" row. Its class name is "MoriTabListHeader".
std::unique_ptr<views::View> CreateTabListHeader(
    BrowserWindowInterface* browser);

// The bar at the bottom of the sidebar.
std::unique_ptr<views::View> CreateSidebarBar(BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_TAB_LIST_H_
