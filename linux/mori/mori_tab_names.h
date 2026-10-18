// Tab names (Arc's and Mori 3's Rename Tab): a name of the user's that
// replaces the page title in the sidebar, kept across restarts.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_TAB_NAMES_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_TAB_NAMES_H_

#include <map>
#include <optional>
#include <string>

class BrowserWindowInterface;

namespace content {
class WebContents;
}

namespace tabs {
class TabInterface;
}

namespace views {
class View;
}

namespace mori {

// The session's per-tab extra data key for the name.
inline constexpr char kTabNameKey[] = "mori_tab_name";

// `contents`' name, when the user gave it one.
std::optional<std::u16string> GetTabName(const content::WebContents* contents);

// Names `tab`; an empty name goes back to the page title.
void SetTabName(tabs::TabInterface* tab, const std::u16string& name);

// From a restored tab's session data.
void RestoreTabName(content::WebContents* contents,
                    const std::map<std::string, std::string>& extra_data);

// An edit field over the title of `tab_view` (a TabView): Enter keeps the
// name, Escape drops it.
void BeginTabRename(views::View* tab_view);

// The same for the active tab of `browser` (the "Rename Tab" command).
void RenameActiveTab(BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_TAB_NAMES_H_
