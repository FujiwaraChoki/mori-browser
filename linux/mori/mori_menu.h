// Mori 3's context menus (crates/mori/src/ui/menus.rs) in place of
// Chromium's: the tab menu, the sidebar's menu, and the page menu, which
// keeps Chromium's items (spelling, extensions, autofill) but reads like
// Mori's: Title Case, Lucide icons, Mori's actions.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_MENU_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_MENU_H_

#include <string>

#include "ui/base/mojom/menu_source_type.mojom-forward.h"

class BrowserWindowInterface;

namespace content {
struct ContextMenuParams;
}

namespace gfx {
class Point;
}

namespace tabs {
class TabInterface;
}

namespace ui {
class SimpleMenuModel;
}

namespace views {
class Widget;
}

namespace mori {

// The menu of `tab`'s row, at `point` (screen coordinates).
void ShowTabMenu(BrowserWindowInterface* browser,
                 tabs::TabInterface* tab,
                 const gfx::Point& point,
                 ui::mojom::MenuSourceType source_type,
                 views::Widget* parent);

// The menu of the sidebar's empty space: Mori 3 shows its "..." menu there
// (Chromium shows the window's system menu). False when `browser` isn't a
// Mori window and Chromium's menu should show.
bool ShowSidebarMenu(BrowserWindowInterface* browser,
                     const gfx::Point& point,
                     ui::mojom::MenuSourceType source_type,
                     views::Widget* parent);

// The menu of a space's chip in the sidebar's bottom bar, for the space on
// screen (the chip's space is shown first): edit, new, delete.
void ShowSpaceMenu(BrowserWindowInterface* browser,
                   const gfx::Point& point,
                   ui::mojom::MenuSourceType source_type,
                   views::Widget* parent);

// The menu of a folder's header (mori_folders.h).
void ShowFolderMenu(BrowserWindowInterface* browser,
                    const std::string& folder_id,
                    const gfx::Point& point,
                    ui::mojom::MenuSourceType source_type,
                    views::Widget* parent);

// Rewrites the page's context menu, built by Chromium, into Mori 3's.
void RestylePageMenu(ui::SimpleMenuModel* model,
                     BrowserWindowInterface* browser,
                     const content::ContextMenuParams& params);

// Items RestylePageMenu adds run their own callback; the page menu only
// has to treat their IDs as enabled and otherwise ignore them.
bool IsMoriMenuItem(int command_id);

// Ctrl+D: bookmarks the page on screen, or removes its bookmark, and says
// which in a toast (Mori 3 has no bookmark dialog).
void ToggleActiveTabBookmark(BrowserWindowInterface* browser);

// Shortcuts Mori 3's page menu doesn't show (Back's Alt+Left, Save's
// Ctrl+S, which hides the sidebar in Mori).
bool HidesPageMenuShortcut(int command_id);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_MENU_H_
