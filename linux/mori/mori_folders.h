// Zen's folders (Arc's, Mori 3's) inside a space: a named run of the
// space's tabs under a header with Zen's folder icon, which opens and
// closes. A closed folder keeps showing its active tab, as in Zen.
//
// Chromium's tab groups are Mori's spaces and don't nest, so folders are
// Mori's own: each tab carries its folder (id, name, icon, open) in its
// session data, and a folder is the run of a space's tabs that share one.
// Moving a tab between two tabs of a folder puts it in the folder; moving
// it away from the run takes it out.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_FOLDERS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_FOLDERS_H_

#include <map>
#include <memory>
#include <optional>
#include <string>
#include <vector>

#include "base/memory/raw_ptr.h"

class BrowserWindowInterface;
class TabGroupView;

namespace content {
class WebContents;
}

namespace gfx {
class Point;
}

namespace tabs {
class TabInterface;
}

namespace views {
class View;
class Widget;
}

namespace mori {

// The session's per-tab extra data key for the tab's folder.
inline constexpr char kTabFolderKey[] = "mori_folder";

// Zen's folder indent (--zen-folder-indent).
inline constexpr int kFolderIndent = 14;

struct FolderInfo {
  std::string id;
  std::u16string name;
  // A Lucide icon's name shown on the folder; empty for none.
  std::string icon;
  bool open = true;
};

// How TabGroupViewLayout lays out one of the space's rows (a tab or a split
// pair) when folders are involved.
struct FolderRow {
  // The folder header that goes above the row, if the row starts a folder.
  raw_ptr<views::View> header = nullptr;
  // In a closed folder (and not the active tab).
  bool hidden = false;
  int indent = 0;
};

// One FolderRow per row of `group_view` (its collection node's children, in
// order), for the space on screen.
std::vector<FolderRow> GetFolderRows(const TabGroupView* group_view);

// Folder headers of `group_view` that `rows` don't place (hidden).
std::vector<views::View*> GetUnplacedFolderHeaders(
    const TabGroupView* group_view,
    const std::vector<FolderRow>& rows);

// Added to each vertical TabGroupView: keeps its folder headers in step
// with its tabs. Never visible.
std::unique_ptr<views::View> CreateFolderHeaders();

// The session data of `contents`' folder, empty when it has none (for
// rewrites of the session file).
std::string GetTabFolderData(const content::WebContents* contents);

// From a restored tab's session data.
void RestoreTabFolder(content::WebContents* contents,
                      const std::map<std::string, std::string>& extra_data);

// `tab`'s folder, if it is in one.
std::optional<FolderInfo> GetTabFolder(const tabs::TabInterface* tab);

// Folders in `tab`'s space, in sidebar order.
std::vector<FolderInfo> FoldersInSpaceOf(BrowserWindowInterface* browser,
                                         const tabs::TabInterface* tab);

// Makes a "New Folder" holding `tab` and starts renaming it.
void NewFolderWithTab(BrowserWindowInterface* browser, tabs::TabInterface* tab);

// Moves `tab` to the end of the folder `folder_id`.
void AddTabToFolder(BrowserWindowInterface* browser,
                    tabs::TabInterface* tab,
                    const std::string& folder_id);

// Takes `tab` out of its folder, to just below it.
void RemoveTabFromFolder(BrowserWindowInterface* browser,
                         tabs::TabInterface* tab);

// The folder header's menu actions.
void NewTabInFolder(BrowserWindowInterface* browser,
                    const std::string& folder_id);
void RenameFolder(BrowserWindowInterface* browser,
                  const std::string& folder_id);
void ToggleFolder(BrowserWindowInterface* browser,
                  const std::string& folder_id);
void SetFolderIcon(BrowserWindowInterface* browser,
                   const std::string& folder_id,
                   const std::string& icon);
// Its tabs stay where they are, out of the folder.
void UngroupFolder(BrowserWindowInterface* browser,
                   const std::string& folder_id);
// Closes its tabs (Zen's Delete Folder).
void DeleteFolder(BrowserWindowInterface* browser,
                  const std::string& folder_id);

// The folder `folder_id`, if a tab still holds it.
std::optional<FolderInfo> GetFolder(const std::string& folder_id);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_FOLDERS_H_
