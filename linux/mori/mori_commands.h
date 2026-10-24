// Actions offered by name in the command bar (Arc and Mori's commands):
// typing "settings", "new space" or "move tab" lists them.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_COMMANDS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_COMMANDS_H_

#include <string>
#include <string_view>
#include <vector>

#include "base/functional/callback.h"

class BrowserWindowInterface;

namespace gfx {
struct VectorIcon;
}

namespace mori {

struct CommandItem {
  std::u16string title;
  const gfx::VectorIcon* icon = nullptr;
  // Its keyboard shortcut, shown at the end of the row ("Ctrl+S").
  std::u16string shortcut;
  base::RepeatingClosure run;
};

// Commands matching `query` right now, best first, at most `limit`. Every
// word of the query has to start a word of the command's title or keywords.
std::vector<CommandItem> MatchCommands(BrowserWindowInterface* browser,
                                       std::u16string_view query,
                                       size_t limit);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_COMMANDS_H_
