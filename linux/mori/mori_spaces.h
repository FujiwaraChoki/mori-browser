// Mori's spaces (Arc's spaces, Zen's workspaces) built on Chromium's tab
// groups: a space is a tab group, and switching to it "focuses" the group,
// so the sidebar shows that space's tabs plus the pinned tabs (favorites),
// which every space shares.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_SPACES_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_SPACES_H_

#include <memory>
#include <optional>

#include "components/tab_groups/tab_group_id.h"

class BrowserWindowInterface;

namespace views {
class View;
}

namespace mori {

// Whether `browser` has any spaces (tab groups) to switch between.
bool HasSpaces(BrowserWindowInterface* browser);

// The space shown in `browser`'s sidebar, if any.
std::optional<tab_groups::TabGroupId> GetCurrentSpace(
    BrowserWindowInterface* browser);

// Where a new tab in the current space goes: its top, as in Arc and Mori 3.
// -1 when there is no current space.
int IndexForNewTab(BrowserWindowInterface* browser);

// Shows the space that holds the active tab. Used after switching to a tab
// in another space, which would otherwise show every space's tabs at once.
void ShowSpaceOfActiveTab(BrowserWindowInterface* browser);

// Makes a new space with one new tab and shows it.
void NewSpace(BrowserWindowInterface* browser);

// Deletes the space on screen and its tabs (there has to be another space).
// Otherwise a space whose last tab goes away comes back with a new tab.
void DeleteSpace(BrowserWindowInterface* browser);

// Shows the `index`th space (from 0, in sidebar order), if there is one.
void SelectSpace(BrowserWindowInterface* browser, int index);

// Moves to the next or previous space.
void SwitchSpace(BrowserWindowInterface* browser, bool forward);

// The space header above the space's tabs: its name, a divider and a
// "New Tab" row (Mori 3's sidebar). Its class name is "MoriSpaceHeader".
std::unique_ptr<views::View> CreateSpaceHeader(BrowserWindowInterface* browser);

// The row of spaces at the bottom of the sidebar.
std::unique_ptr<views::View> CreateSpaceBar(BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_SPACES_H_
