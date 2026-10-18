#include "chrome/browser/ui/views/mori/mori_commands.h"

#include <algorithm>
#include <iterator>

#include "base/functional/bind.h"
#include "base/strings/string_split.h"
#include "base/strings/string_util.h"
#include "chrome/app/chrome_command_ids.h"
#include "chrome/browser/ui/browser_commands.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/tab_group_model.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_command_ids.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_spaces.h"
#include "chrome/browser/ui/views/mori/mori_tab_names.h"
#include "components/tab_groups/tab_group_visual_data.h"
#include "components/tabs/public/tab_group.h"
#include "components/tabs/public/tab_interface.h"
#include "ui/base/accelerators/accelerator.h"

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
  command(u"Peek Link on Clipboard", u"preview glance paste", "eye",
          kCommandPeekClipboard);
  command(u"Capture", u"screenshot snip image area", "camera",
          IDC_SHARING_HUB_SCREENSHOT);
  command(u"Toggle Sidebar", u"hide show compact mode zen", "panel-left",
          IDC_TOGGLE_VERTICAL_TABS);
  command(u"Keyboard Shortcuts", u"keys hotkeys cheatsheet help", "keyboard",
          kCommandShowShortcuts);
  command(u"Settings", u"preferences options", "settings", IDC_OPTIONS);
  command(u"History & Bookmarks", u"library archive", "library",
          IDC_SHOW_HISTORY);
  command(u"Downloads", u"files", "download", IDC_SHOW_DOWNLOADS);
  command(u"Extensions", u"add-ons plugins", "puzzle",
          IDC_MANAGE_EXTENSIONS);
  command(u"New Space", u"context create", "layers", kCommandNewSpace);
  const std::vector<tab_groups::TabGroupId> spaces =
      model->SupportsTabGroups() ? model->group_model()->ListTabGroups()
                                 : std::vector<tab_groups::TabGroupId>();
  if (spaces.size() > 1) {
    command(u"Next Space", u"switch context right", "arrow-right",
            IDC_FOCUS_NEXT_TAB_GROUP);
    command(u"Previous Space", u"switch context left", "arrow-left",
            IDC_FOCUS_PREV_TAB_GROUP);
    action(u"Delete Space", u"remove context close", "trash",
           base::BindRepeating(&DeleteSpace, browser));
  }
  if (GetCurrentSpace(browser)) {
    action(u"Rename Space", u"context color colour", "pencil",
           base::BindRepeating(
               [](BrowserWindowInterface* b) {
                 if (auto space = GetCurrentSpace(b)) {
                   b->GetTabStripModel()->OpenTabGroupEditor(*space);
                 }
               },
               browser));
  }
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
    command(u"Bookmark Page", u"star favorite", "star",
            IDC_BOOKMARK_THIS_TAB);
    command(u"Split View", u"side by side split screen", "columns-2",
            IDC_NEW_SPLIT_TAB);
    command(u"Close Tab", u"", "x", IDC_CLOSE_TAB);
    command(u"Move Tab to New Window", u"window split off", "app-window",
            IDC_MOVE_TAB_TO_NEW_WINDOW);
    // One entry per other space.
    for (const tab_groups::TabGroupId& id : spaces) {
      if (tab->GetGroup() == id) {
        continue;
      }
      const std::u16string name =
          model->group_model()->GetTabGroup(id)->visual_data()->title();
      action(u"Move Tab to " + name, u"space context", "arrow-right-left",
             base::BindRepeating(
                 [](BrowserWindowInterface* b, tab_groups::TabGroupId space) {
                   TabStripModel* m = b->GetTabStripModel();
                   if (!m->group_model()->ContainsTabGroup(space) ||
                       m->active_index() == TabStripModel::kNoTab) {
                     return;
                   }
                   const int index = m->active_index();
                   if (m->IsTabPinned(index)) {
                     m->SetTabPinned(index, false);
                   }
                   m->AddToExistingGroup({m->active_index()}, space);
                   m->SetFocusedGroup(space);
                 },
                 browser, id));
    }
  }
  for (const tab_groups::TabGroupId& id : spaces) {
    if (GetCurrentSpace(browser) == id) {
      continue;
    }
    const std::u16string name =
        model->group_model()->GetTabGroup(id)->visual_data()->title();
    action(u"Switch to " + name, u"space context", "layers",
           base::BindRepeating(
               [](BrowserWindowInterface* b, tab_groups::TabGroupId space) {
                 if (b->GetTabStripModel()->group_model()->ContainsTabGroup(
                         space)) {
                   b->GetTabStripModel()->SetFocusedGroup(space);
                 }
               },
               browser, id));
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
