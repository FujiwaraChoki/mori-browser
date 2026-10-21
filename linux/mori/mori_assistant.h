// Mori's assistant (Mori 3's): a panel docked beside the page (Ctrl+Shift+K
// or the sparkles button) holding a conversation with the local Codex app
// server, which can see and drive the browser through Mori's tools. Every
// read of page data or change to the browser waits for the user's Allow.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_ASSISTANT_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_ASSISTANT_H_

#include <string>

class BrowserWindowInterface;

namespace views {
class View;
}

namespace mori {

// Width of the docked panel, as Mori 3's.
inline constexpr int kAssistantPanelWidth = 360;

void ToggleAssistant(BrowserWindowInterface* browser);
bool IsAssistantOpen(BrowserWindowInterface* browser);

// Opens the panel and sends `prompt` (the command bar's Ask Assistant).
void AskAssistant(BrowserWindowInterface* browser, const std::string& prompt);

// The panel to dock beside the page in `browser_view`, if it's open (for
// the browser layout).
views::View* GetVisibleAssistantPanel(views::View* browser_view);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_ASSISTANT_H_
