// The browser tools Mori's assistant offers Codex as `dynamicTools` (Mori
// 3's, ported from its macOS BrowserAutomation):
//
// * mori_browser_snapshot: open tabs plus the selected page's contents.
// * mori_browser_action: tab, navigation and page actions.
// * mori_search_history: keyword search over browsing history.
//
// ApprovalFor() says whether a call needs the user's OK (and describes it);
// ExecuteTool() runs it. Everything that came from a web page goes back
// inside explicit untrusted-content delimiters, capped in length.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_ASSISTANT_TOOLS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_ASSISTANT_TOOLS_H_

#include <optional>
#include <string>

#include "base/functional/callback.h"
#include "base/values.h"

class BrowserWindowInterface;

namespace mori {

struct ToolOutcome {
  // What the model sees.
  std::string text;
  bool success = false;
  // A one-line, past-tense description for the conversation.
  std::string summary;

  // The `item/tool/call` response: {contentItems, success}.
  base::Value RpcResult() const;

  static ToolOutcome Stopped();
  static ToolOutcome Denied(const std::string& tool,
                            const base::DictValue& args);
};

// The `dynamicTools` array for `thread/start`.
base::Value DynamicTools();

// The text sent for a user request (with the agent's instructions when the
// thread has the browser tools).
std::string PromptFor(const std::string& text, bool uses_tools);

// The in-progress line for a call ("Opening example.com…").
std::string RunningLabel(const std::string& tool, const base::DictValue& args);

struct Approval {
  std::string text;
  // Only observes; could be auto-approved one day.
  bool read_only = false;
};

// Whether the call needs the user's OK, and what to ask.
std::optional<Approval> ApprovalFor(BrowserWindowInterface* browser,
                                    const std::string& tool,
                                    const base::DictValue& args);

// Runs a call (after approval) against `browser`.
void ExecuteTool(BrowserWindowInterface* browser,
                 const std::string& tool,
                 base::DictValue args,
                 base::OnceCallback<void(ToolOutcome)> done,
                 base::RepeatingCallback<bool()> is_current);

// `text` capped at `max` characters, marking the cut.
std::string Clip(const std::string& text, size_t max);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_ASSISTANT_TOOLS_H_
