// Mori's own browser commands (menu items and shortcuts that are not
// Chromium commands), and the shortcuts Mori adds to Chromium's.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_COMMAND_IDS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_COMMAND_IDS_H_

#include <vector>

class BrowserWindowInterface;
struct AcceleratorMapping;

namespace mori {

// A block of IDs Chromium leaves unused: below its unbounded menu range at
// 0xE000, which the app menu reserves for bookmarks and recent tabs.
enum CommandId {
  kCommandFirst = 56900,
  kCommandSleepBackgroundTabs = kCommandFirst,
  kCommandEditTheme,
  kCommandTogglePin,
  // Mori 3's Clear: closes the tabs that aren't favorites.
  kCommandArchiveUnpinnedTabs,
  // Ctrl+Tab and Ctrl+Shift+Tab: recent tabs (mori_tab_switcher.h).
  kCommandSwitchRecentTab,
  kCommandSwitchRecentTabBack,
  // Ctrl+/: the shortcut cheatsheet.
  kCommandShowShortcuts,
  // Ctrl+Shift+O: the link on the clipboard in Peek.
  kCommandPeekClipboard,
  // Ctrl+Shift+B: the boost editor for the site on screen.
  kCommandBoostSite,
  // Ctrl+Shift+K: the assistant panel.
  kCommandToggleAssistant,
  kCommandNewFolder,
  kCommandLast = kCommandNewFolder,
};

bool IsMoriCommand(int command_id);
void ExecuteMoriCommand(BrowserWindowInterface* browser, int command_id);

// Handles Close Tab when its selection contains pins. Pins keep their sidebar
// entries; closing the active pin switches away and unloads its page.
bool HandlePinnedTabClose(BrowserWindowInterface* browser);

// Mori 3's shortcuts on top of Chromium's (Ctrl+S hides the sidebar,
// Ctrl+Shift+C copies the URL, ...). Replaces
// Chromium's binding where a key has one.
void ApplyMoriAccelerators(std::vector<AcceleratorMapping>* accelerators);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_COMMAND_IDS_H_
