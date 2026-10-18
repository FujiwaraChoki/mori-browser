#include "chrome/browser/ui/views/mori/mori_commands.h"

#include <algorithm>
#include <array>
#include <iterator>

#include "base/strings/string_split.h"
#include "base/strings/string_util.h"
#include "chrome/app/chrome_command_ids.h"
#include "chrome/app/vector_icons/vector_icons.h"
#include "chrome/browser/ui/browser_commands.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/mori/mori_spaces.h"
#include "components/tabs/public/tab_interface.h"
#include "components/vector_icons/vector_icons.h"

namespace mori {
namespace {

template <int kCommandId>
void Run(BrowserWindowInterface* browser) {
  chrome::ExecuteCommand(browser, kCommandId);
}

bool ActiveTabPinned(BrowserWindowInterface* browser) {
  tabs::TabInterface* tab = browser->GetTabStripModel()->GetActiveTab();
  return tab && tab->IsPinned();
}

bool ActiveTabUnpinned(BrowserWindowInterface* browser) {
  tabs::TabInterface* tab = browser->GetTabStripModel()->GetActiveTab();
  return tab && !tab->IsPinned();
}

void SetActiveTabPinned(BrowserWindowInterface* browser, bool pinned) {
  TabStripModel* model = browser->GetTabStripModel();
  if (model->active_index() != TabStripModel::kNoTab) {
    model->SetTabPinned(model->active_index(), pinned);
  }
}

const auto& Commands() {
  static const std::array<Command, 20> kCommands = {{
      {u"New Space", u"workspace create", &kAddIcon, &NewSpace},
      {u"Next Space", u"workspace switch", &vector_icons::kArrowForwardIcon,
       [](BrowserWindowInterface* b) { SwitchSpace(b, true); },
       &HasSpaces},
      {u"Previous Space", u"workspace switch", &vector_icons::kArrowBackIcon,
       [](BrowserWindowInterface* b) { SwitchSpace(b, false); },
       &HasSpaces},
      {u"Add to Favorites", u"pin tab favourite", &kKeepIcon,
       [](BrowserWindowInterface* b) { SetActiveTabPinned(b, true); },
       &ActiveTabUnpinned},
      {u"Remove from Favorites", u"unpin tab favourite", &kKeepOffIcon,
       [](BrowserWindowInterface* b) { SetActiveTabPinned(b, false); },
       &ActiveTabPinned},
      {u"Toggle Sidebar", u"collapse compact hide show",
       &kDockToLeftIcon, &Run<IDC_TOGGLE_VERTICAL_TABS_COLLAPSE>},
      {u"Split View", u"side by side split screen", &kSplitSceneIcon,
       &Run<IDC_NEW_SPLIT_TAB>},
      {u"Duplicate Tab", u"copy", &vector_icons::kContentCopyIcon,
       &Run<IDC_DUPLICATE_TAB>},
      {u"Copy URL", u"link address", &vector_icons::kContentCopyIcon,
       &Run<IDC_COPY_URL>},
      {u"Close Tab", u"", &kTabCloseIcon, &Run<IDC_CLOSE_TAB>},
      {u"New Window", u"", &kNewWindowIcon, &Run<IDC_NEW_WINDOW>},
      {u"New Private Window", u"incognito", &kIncognitoIcon,
       &Run<IDC_NEW_INCOGNITO_WINDOW>},
      {u"History", u"", &kHistoryIcon, &Run<IDC_SHOW_HISTORY>},
      {u"Downloads", u"files", &kDownloadIcon, &Run<IDC_SHOW_DOWNLOADS>},
      {u"Bookmarks", u"manager", &kBookmarkManagerIcon,
       &Run<IDC_SHOW_BOOKMARK_MANAGER>},
      {u"Extensions", u"add-ons plugins", &vector_icons::kExtensionChromeRefreshOldIcon,
       &Run<IDC_MANAGE_EXTENSIONS>},
      {u"Settings", u"preferences options", &kSettingsIcon,
       &Run<IDC_OPTIONS>},
      {u"Clear Browsing Data", u"history cookies cache delete", &kDeleteIcon,
       &Run<IDC_CLEAR_BROWSING_DATA>},
      {u"Developer Tools", u"devtools inspect console", &kCodeIcon,
       &Run<IDC_DEV_TOOLS>},
      {u"Print", u"pdf", &kPrintIcon, &Run<IDC_PRINT>},
  }};
  return kCommands;
}

std::vector<std::u16string> Words(std::u16string_view text) {
  return base::SplitString(base::ToLowerASCII(text), u" ",
                           base::TRIM_WHITESPACE, base::SPLIT_WANT_NONEMPTY);
}

bool EveryWordStartsOneOf(const std::vector<std::u16string>& query,
                          const std::vector<std::u16string>& words) {
  return std::ranges::all_of(query, [&](const std::u16string& q) {
    return std::ranges::any_of(words, [&](const std::u16string& w) {
      return w.starts_with(q);
    });
  });
}

}  // namespace

std::vector<const Command*> MatchCommands(BrowserWindowInterface* browser,
                                          std::u16string_view query,
                                          size_t limit) {
  const std::vector<std::u16string> query_words = Words(query);
  if (query_words.empty()) {
    return {};
  }
  // Title matches rank above keyword matches; the list order breaks ties.
  std::vector<const Command*> by_title;
  std::vector<const Command*> by_keyword;
  for (const Command& command : Commands()) {
    if (command.available && !command.available(browser)) {
      continue;
    }
    const std::vector<std::u16string> title = Words(command.title);
    std::vector<std::u16string> all = title;
    std::ranges::move(Words(command.keywords), std::back_inserter(all));
    if (EveryWordStartsOneOf(query_words, title)) {
      by_title.push_back(&command);
    } else if (EveryWordStartsOneOf(query_words, all)) {
      by_keyword.push_back(&command);
    }
  }
  by_title.insert(by_title.end(), by_keyword.begin(), by_keyword.end());
  if (by_title.size() > limit) {
    by_title.resize(limit);
  }
  return by_title;
}

}  // namespace mori
