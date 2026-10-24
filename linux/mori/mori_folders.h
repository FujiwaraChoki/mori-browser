// Nested sidebar folders. Every tab saves its immediate folder and all
// ancestors in Chromium's session. Folder subtrees precede Add Tab; ordinary
// tabs follow it. Collapsing a folder hides its complete subtree.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_FOLDERS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_FOLDERS_H_

#include <map>
#include <memory>
#include <optional>
#include <string>
#include <vector>

#include "base/memory/raw_ptr.h"
#include "components/tabs/public/tab_interface.h"

class BrowserWindowInterface;
class TabCollectionNode;

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
  std::string parent;
};

// How UnpinnedTabContainerViewLayout lays out one of its rows (a tab or a
// split pair) when folders are involved.
struct FolderRow {
  raw_ptr<views::View> view = nullptr;
  bool hidden = false;
  int indent = 0;
};

// Folder headers, member tabs, Add Tab, then ordinary tabs in display order.
std::vector<FolderRow> GetFolderRows(const views::View* container,
                                     const TabCollectionNode* node);

// Folder headers of `container` that `rows` don't place (hidden).
std::vector<views::View*> GetUnplacedFolderHeaders(
    const views::View* container,
    const std::vector<FolderRow>& rows);

// Added to the vertical tab list (UnpinnedTabContainerView): keeps its
// folder headers in step with its tabs. Never visible.
std::unique_ptr<views::View> CreateFolderHeaders();

// The session data of `contents`' folder, empty when it has none (for
// rewrites of the session file).
std::string GetTabFolderData(const content::WebContents* contents);

// Persists a newly imported tab's folder once the tab belongs to a window.
void PersistTabFolder(content::WebContents* contents);

// From a restored tab's session data.
void RestoreTabFolder(content::WebContents* contents,
                      const std::map<std::string, std::string>& extra_data);

// `tab`'s folder, if it is in one.
std::optional<FolderInfo> GetTabFolder(const tabs::TabInterface* tab);

// The folders, in sidebar order.
std::vector<FolderInfo> ListFolders(BrowserWindowInterface* browser);

// Creates a folder with a blank tab and starts renaming its header.
void NewFolder(BrowserWindowInterface* browser);
void NewSubfolder(BrowserWindowInterface* browser, const std::string& parent);
// An empty parent moves a folder to the root. Rejects cycles.
void MoveFolder(BrowserWindowInterface* browser, const std::string& folder,
                const std::string& parent);
int IndexAfterFolders(BrowserWindowInterface* browser);

// Makes a "New Folder" holding `tab` and starts renaming it.
void NewFolderWithTab(BrowserWindowInterface* browser, tabs::TabInterface* tab);

// Moves `tab` to the end of the folder `folder_id`.
void AddTabToFolder(BrowserWindowInterface* browser,
                    tabs::TabInterface* tab,
                    const std::string& folder_id);

// Takes `tab` out of its folder, to just below it.
void RemoveTabFromFolder(BrowserWindowInterface* browser,
                         tabs::TabInterface* tab);

// Resolve the release position before native drag cleanup, then commit once
// Chromium has released its dragged views. Canceled drags never call this.
void QueueSidebarTabDrop(BrowserWindowInterface* browser,
                         std::vector<tabs::TabHandle> tabs,
                         const gfx::Point& screen_point);

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
