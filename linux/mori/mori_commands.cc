#include "chrome/browser/ui/views/mori/mori_commands.h"

#include <algorithm>
#include <iterator>

#include "base/functional/bind.h"
#include "base/strings/string_split.h"
#include "base/strings/string_util.h"
#include "base/strings/utf_string_conversions.h"
#include "chrome/app/chrome_command_ids.h"
#include "chrome/browser/ui/browser_commands.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_command_ids.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_import.h"
#include "chrome/browser/ui/views/mori/mori_little.h"
#include "chrome/browser/ui/views/mori/mori_tab_names.h"
#include "components/tabs/public/tab_interface.h"
#include "ui/base/accelerators/accelerator.h"
#include "url/gurl.h"

namespace mori {
namespace {

struct Candidate {
  CommandItem item;
  std::u16string keywords;
};

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

// Mori 3's command list (crates/mori/src/commands.rs), in its order.
std::vector<Candidate> Candidates(BrowserWindowInterface* browser) {
  std::vector<Candidate> out;
  BrowserView* view = BrowserView::GetBrowserViewForBrowser(browser);
  auto shortcut = [view](int command_id) -> std::u16string {
    ui::Accelerator accelerator;
    return view && view->GetAccelerator(command_id, &accelerator)
               ? accelerator.GetShortcutText()
               : std::u16string();
  };
  auto command = [&](std::u16string title, std::u16string keywords,
                     std::string_view icon, int command_id) {
    out.push_back(
        {{std::move(title), &LucideIcon(icon), shortcut(command_id),
          base::BindRepeating(
              [](BrowserWindowInterface* b, int id) {
                chrome::ExecuteCommand(b, id);
              },
              browser, command_id)},
         std::move(keywords)});
  };
  auto action = [&](std::u16string title, std::u16string keywords,
                    std::string_view icon, base::RepeatingClosure run) {
    out.push_back({{std::move(title), &LucideIcon(icon), u"", std::move(run)},
                   std::move(keywords)});
  };

  TabStripModel* model = browser->GetTabStripModel();
  tabs::TabInterface* tab = model->GetActiveTab();

  command(u"New Tab", u"open blank", "plus", IDC_NEW_TAB);
  command(u"New Window", u"browser", "app-window", IDC_NEW_WINDOW);
  command(u"New Private Window", u"incognito", "hat-glasses",
          IDC_NEW_INCOGNITO_WINDOW);
  command(u"Reader View", u"read article distraction", "book-open-text",
          IDC_SHOW_READING_MODE_SIDE_PANEL);
  command(u"Sleep Background Tabs", u"memory unload", "moon",
          kCommandSleepBackgroundTabs);
  command(u"Reopen Closed Tab", u"undo restore", "rotate-ccw",
          IDC_RESTORE_TAB);
  command(u"Find in Page", u"search text", "search", IDC_FIND);
  command(u"Assistant", u"ai codex chat ask summarize agent", "sparkles",
          kCommandToggleAssistant);
  command(u"Peek Link on Clipboard", u"preview glance paste", "eye",
          kCommandPeekClipboard);
  command(u"Capture", u"screenshot snip image area", "camera",
          IDC_SHARING_HUB_SCREENSHOT);
  command(u"Toggle Sidebar", u"hide show compact mode zen", "panel-left",
          IDC_TOGGLE_VERTICAL_TABS);
  // Mori 3's setting for links from other apps.
  if (LinksOpenInLittle(browser->GetProfile())) {
    action(u"Open Links from Other Apps in a New Tab",
           u"little mori external default setting", "app-window",
           base::BindRepeating(
               [](BrowserWindowInterface* b) {
                 SetLinksOpenInLittle(b->GetProfile(), false);
               },
               browser));
  } else {
    action(u"Open Links from Other Apps in Little Mori",
           u"little mori external default setting", "app-window",
           base::BindRepeating(
               [](BrowserWindowInterface* b) {
                 SetLinksOpenInLittle(b->GetProfile(), true);
               },
               browser));
  }
  // Mori 3's import (Arc's onboarding does the same).
  for (auto [title, source] :
       {std::pair{u"Import Tabs from Zen", ImportSource::kZen},
        std::pair{u"Import Tabs from Firefox", ImportSource::kFirefox},
        std::pair{u"Import Tabs from Arc", ImportSource::kArc}}) {
    action(title, u"import migrate switch tabs workspaces essentials",
           "download", base::BindRepeating(&ImportTabs, browser, source));
  }
  command(u"Import Bookmarks and History", u"import migrate passwords",
          "bookmark", IDC_IMPORT_SETTINGS);
  command(u"Keyboard Shortcuts", u"keys hotkeys cheatsheet help", "keyboard",
          kCommandShowShortcuts);
  command(u"Settings", u"preferences options", "settings", IDC_OPTIONS);
  command(u"History & Bookmarks", u"library archive", "library",
          IDC_SHOW_HISTORY);
  command(u"Downloads", u"files", "download", IDC_SHOW_DOWNLOADS);
  command(u"Extensions", u"add-ons plugins", "puzzle",
          IDC_MANAGE_EXTENSIONS);
  command(u"Edit Theme", u"color colour gradient wash appearance", "palette",
          kCommandEditTheme);
  // Mori 3's Clear (Zen's, Arc's archive of Today).
  command(u"Archive Unpinned Tabs", u"clear today close tabs", "archive",
          kCommandArchiveUnpinnedTabs);
  command(u"Developer Tools", u"inspect console devtools", "code",
          IDC_DEV_TOOLS);
  command(u"Print…", u"pdf", "printer", IDC_PRINT);
  command(u"Clear Browsing Data", u"history cookies cache delete", "trash",
          IDC_CLEAR_BROWSING_DATA);
  if (tab) {
    command(u"Copy URL", u"link clipboard", "copy", IDC_COPY_URL);
    command(tab->IsPinned() ? u"Remove from Favorites" : u"Add to Favorites",
            u"pin unpin tab favourite", tab->IsPinned() ? "pin-off" : "pin",
            kCommandTogglePin);
    command(u"Duplicate Tab", u"copy clone", "copy", IDC_DUPLICATE_TAB);
    action(u"Rename Tab", u"title name", "pencil",
           base::BindRepeating(&RenameActiveTab, browser));
    if (tab->GetContents()->GetLastCommittedURL().SchemeIsHTTPOrHTTPS()) {
      command(u"Boost This Site", u"css javascript zap customize style",
              "zap", kCommandBoostSite);
    }
    command(u"Bookmark Page", u"star favorite", "star",
            IDC_BOOKMARK_THIS_TAB);
    command(u"Split View", u"side by side split screen", "columns-2",
            IDC_NEW_SPLIT_TAB);
    command(u"Close Tab", u"", "x", IDC_CLOSE_TAB);
    command(u"Move Tab to New Window", u"window split off", "app-window",
            IDC_MOVE_TAB_TO_NEW_WINDOW);
  }
  return out;
}

}  // namespace

std::vector<CommandItem> MatchCommands(BrowserWindowInterface* browser,
                                       std::u16string_view query,
                                       size_t limit) {
  const std::vector<std::u16string> query_words = Words(query);
  if (query_words.empty()) {
    return {};
  }
  // Title matches rank above keyword matches; the list order breaks ties.
  std::vector<CommandItem> by_title;
  std::vector<CommandItem> by_keyword;
  for (Candidate& candidate : Candidates(browser)) {
    const std::vector<std::u16string> title = Words(candidate.item.title);
    std::vector<std::u16string> all = title;
    std::ranges::move(Words(candidate.keywords), std::back_inserter(all));
    if (EveryWordStartsOneOf(query_words, title)) {
      by_title.push_back(std::move(candidate.item));
    } else if (EveryWordStartsOneOf(query_words, all)) {
      by_keyword.push_back(std::move(candidate.item));
    }
  }
  std::ranges::move(by_keyword, std::back_inserter(by_title));
  if (by_title.size() > limit) {
    by_title.resize(limit);
  }
  return by_title;
}

}  // namespace mori
