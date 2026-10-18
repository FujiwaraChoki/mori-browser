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
  kCommandNewSpace = kCommandFirst,
  kCommandSleepBackgroundTabs,
  kCommandEditTheme,
  kCommandTogglePin,
  kCommandSelectSpace1,
  kCommandSelectSpace9 = kCommandSelectSpace1 + 8,
  kCommandLast = kCommandSelectSpace9,
};

bool IsMoriCommand(int command_id);
void ExecuteMoriCommand(BrowserWindowInterface* browser, int command_id);

// Mori 3's shortcuts on top of Chromium's (Ctrl+S hides the sidebar,
// Alt+1-9 picks a space, Ctrl+Shift+C copies the URL, ...). Replaces
// Chromium's binding where a key has one.
void ApplyMoriAccelerators(std::vector<AcceleratorMapping>* accelerators);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_COMMAND_IDS_H_
