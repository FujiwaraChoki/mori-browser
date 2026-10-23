#include "chrome/browser/ui/views/mori/mori_command_ids.h"

#include <algorithm>
#include <array>

#include "chrome/app/chrome_command_ids.h"
#include "chrome/browser/resource_coordinator/lifecycle_unit_state.mojom.h"
#include "chrome/browser/resource_coordinator/tab_lifecycle_unit_external.h"
#include "chrome/browser/ui/accelerator_table.h"
#include "chrome/browser/ui/browser_commands.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/navigator/browser_navigator.h"
#include "chrome/browser/ui/navigator/browser_navigator_params.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "base/functional/bind.h"
#include "build/build_config.h"
#include "base/strings/string_util.h"
#include "chrome/browser/ui/views/mori/mori_assistant.h"
#include "chrome/browser/ui/views/mori/mori_boosts.h"
#include "chrome/browser/ui/views/mori/mori_cheatsheet.h"
#include "chrome/browser/ui/views/mori/mori_peek.h"
#include "chrome/browser/ui/views/mori/mori_folders.h"
#include "chrome/browser/ui/views/mori/mori_settings.h"
#include "chrome/browser/ui/views/mori/mori_tab_list.h"
#include "chrome/browser/ui/views/mori/mori_tab_switcher.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/web_contents.h"
#include "ui/base/clipboard/clipboard.h"
#include "ui/events/event_constants.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "url/gurl.h"

namespace mori {
namespace {

// Frees the memory of every tab but the one on screen and any playing
// sound (Chromium reloads a tab when it is selected again).
void SleepBackgroundTabs(BrowserWindowInterface* browser) {
  TabStripModel* model = browser->GetTabStripModel();
  for (int i = 0; i < model->count(); ++i) {
    tabs::TabInterface* tab = model->GetTabAtIndex(i);
    if (i == model->active_index() || !tab->GetContents() ||
        tab->GetContents()->IsCurrentlyAudible()) {
      continue;
    }
    if (auto* lifecycle =
            resource_coordinator::TabLifecycleUnitExternal::FromWebContents(
                tab->GetContents())) {
      lifecycle->DiscardTab(mojom::LifecycleUnitDiscardReason::EXTERNAL);
    }
  }
}

// Ctrl+Shift+P: the current tab becomes a favorite, or stops being one.
void TogglePin(BrowserWindowInterface* browser) {
  TabStripModel* model = browser->GetTabStripModel();
  const int index = model->active_index();
  if (index != TabStripModel::kNoTab) {
    model->SetTabPinned(index, !model->IsTabPinned(index));
  }
}

// Ctrl+Shift+O: the web address on the clipboard, in Peek (Mori 3).
void PeekClipboard(BrowserWindowInterface* browser) {
  ui::Clipboard::GetForCurrentThread()->ReadText(
      ui::ClipboardBuffer::kCopyPaste, /*data_dst=*/std::nullopt,
      base::BindOnce(
          [](base::WeakPtr<BrowserWindowInterface> browser,
             std::u16string text) {
            const GURL url(base::TrimWhitespace(text, base::TRIM_ALL));
            if (browser && url.is_valid() && url.SchemeIsHTTPOrHTTPS()) {
              ShowPeek(browser.get(), url);
            }
          },
          browser->GetWeakPtr()));
}

#if !BUILDFLAG(IS_MAC)
constexpr int kCtrl = ui::EF_CONTROL_DOWN;
constexpr int kShift = ui::EF_SHIFT_DOWN;
constexpr int kAlt = ui::EF_ALT_DOWN;

// Mori 3's keymap (crates/mori/src/actions.rs) where Chromium differs.
constexpr AcceleratorMapping kMoriAccelerators[] = {
    // Arc: hide the sidebar; Zen/Mori 3 compact mode does the same here.
    {ui::VKEY_S, kCtrl, IDC_TOGGLE_VERTICAL_TABS},
#if !BUILDFLAG(IS_WIN)
    // Alt+1-9 picks a tab, as Ctrl+1-9 does (Chromium's Linux keys).
    {ui::VKEY_1, kAlt, IDC_SELECT_TAB_0},
    {ui::VKEY_2, kAlt, IDC_SELECT_TAB_1},
    {ui::VKEY_3, kAlt, IDC_SELECT_TAB_2},
    {ui::VKEY_4, kAlt, IDC_SELECT_TAB_3},
    {ui::VKEY_5, kAlt, IDC_SELECT_TAB_4},
    {ui::VKEY_6, kAlt, IDC_SELECT_TAB_5},
    {ui::VKEY_7, kAlt, IDC_SELECT_TAB_6},
    {ui::VKEY_8, kAlt, IDC_SELECT_TAB_7},
    {ui::VKEY_9, kAlt, IDC_SELECT_LAST_TAB},
#endif
    // Tabs: Ctrl+N shares the new-tab flow with Ctrl+T.
    {ui::VKEY_N, kCtrl, IDC_NEW_TAB},
    {ui::VKEY_DOWN, kCtrl | kAlt, IDC_SELECT_NEXT_TAB},
    {ui::VKEY_UP, kCtrl | kAlt, IDC_SELECT_PREVIOUS_TAB},
    {ui::VKEY_P, kCtrl | kShift, kCommandTogglePin},
    {ui::VKEY_C, kCtrl | kShift, IDC_COPY_URL},
    {ui::VKEY_D, kCtrl | kShift, IDC_DUPLICATE_TAB},
#if !BUILDFLAG(IS_WIN)
    // Not on Windows, where Ctrl+Alt is AltGr and types characters (AltGr+S
    // is "ś" on Polish keyboards); the menu and command bar still have them.
    {ui::VKEY_S, kCtrl | kAlt, kCommandSleepBackgroundTabs},
    {ui::VKEY_R, kCtrl | kAlt, IDC_SHOW_READING_MODE_SIDE_PANEL},
#endif
    // Arc: Ctrl+Tab goes through recent tabs, not the strip's order.
    {ui::VKEY_TAB, kCtrl, kCommandSwitchRecentTab},
    {ui::VKEY_TAB, kCtrl | kShift, kCommandSwitchRecentTabBack},
    {ui::VKEY_OEM_2, kCtrl, kCommandShowShortcuts},
    {ui::VKEY_O, kCtrl | kShift, kCommandPeekClipboard},
    // Arc's Capture: drag over the page to capture an area.
    {ui::VKEY_X, kCtrl | kShift, IDC_SHARING_HUB_SCREENSHOT},
    // Arc: Boost this site (Chromium's bookmarks bar toggle otherwise).
    {ui::VKEY_B, kCtrl | kShift, kCommandBoostSite},
    {ui::VKEY_K, kCtrl | kShift, kCommandToggleAssistant},
    // Places.
    {ui::VKEY_OEM_COMMA, kCtrl, IDC_OPTIONS},
    {ui::VKEY_Y, kCtrl, IDC_SHOW_HISTORY},
    {ui::VKEY_J, kCtrl | kShift, IDC_SHOW_DOWNLOADS},
};
#endif  // !BUILDFLAG(IS_MAC)

}  // namespace

bool IsMoriCommand(int command_id) {
  return command_id >= kCommandFirst && command_id <= kCommandLast;
}

bool HandlePinnedTabClose(BrowserWindowInterface* browser) {
  if (browser->GetType() != BrowserWindowInterface::TYPE_NORMAL) {
    return false;
  }
  TabStripModel* model = browser->GetTabStripModel();
  const int index = model->active_index();
  if (index == TabStripModel::kNoTab) {
    return false;
  }

  if (model->IsTabPinned(index)) {
    const tabs::TabHandle pin = model->GetTabAtIndex(index)->GetHandle();
    // Select first: Chromium cannot discard the foreground tab. Use a real
    // blank tab for the last pin, rather than Mori's new-tab launcher overlay.
    if (model->count() == 1) {
      chrome::NewTab(browser, NewTabTypes::kNewTabCommand);
    } else {
      model->ActivateTabAt((index + 1) % model->count(),
                          TabStripUserGestureDetails(
                              TabStripUserGestureDetails::GestureType::kKeyboard));
    }
    if (tabs::TabInterface* tab = pin.Get()) {
      if (auto* lifecycle =
              resource_coordinator::TabLifecycleUnitExternal::FromWebContents(
                  tab->GetContents())) {
        lifecycle->DiscardTab(mojom::LifecycleUnitDiscardReason::EXTERNAL);
      }
    }
    return true;
  }

  // A mixed multi-selection must not let Chromium's CloseSelectedTabs remove
  // pins alongside ordinary tabs. Snapshot handles before any close mutates it.
  bool has_pin = false;
  std::vector<tabs::TabHandle> unpinned;
  for (const tabs::TabInterface* tab : model->selection_model().selected_tabs()) {
    if (tab->IsPinned()) {
      has_pin = true;
    } else {
      unpinned.push_back(tab->GetHandle());
    }
  }
  if (!has_pin) {
    return false;
  }
  for (const tabs::TabHandle& handle : unpinned) {
    if (tabs::TabInterface* tab = handle.Get()) {
      const int tab_index = model->GetIndexOfTab(tab);
      if (tab_index != TabStripModel::kNoTab && !model->IsTabPinned(tab_index)) {
        model->CloseWebContentsAt(tab_index,
                                 TabCloseTypes::CLOSE_USER_GESTURE |
                                     TabCloseTypes::CLOSE_CREATE_HISTORICAL_TAB);
      }
    }
  }
  return true;
}

void ExecuteMoriCommand(BrowserWindowInterface* browser, int command_id) {
  switch (command_id) {
    case kCommandNewFolder:
      NewFolder(browser);
      break;
    case kCommandSleepBackgroundTabs:
      SleepBackgroundTabs(browser);
      break;
    case kCommandArchiveUnpinnedTabs:
      ArchiveUnpinnedTabs(browser);
      break;
    case kCommandTogglePin:
      TogglePin(browser);
      break;
    case kCommandSwitchRecentTab:
      StepTabSwitcher(browser, /*forward=*/true);
      break;
    case kCommandSwitchRecentTabBack:
      StepTabSwitcher(browser, /*forward=*/false);
      break;
    case kCommandShowShortcuts:
      ToggleCheatsheet(browser);
      break;
    case kCommandPeekClipboard:
      PeekClipboard(browser);
      break;
    case kCommandBoostSite:
      EditBoost(browser);
      break;
    case kCommandToggleAssistant:
      ToggleAssistant(browser);
      break;
    case kCommandEditTheme:
      // Mori 3's "Edit Theme…": the theme is in Settings.
      ShowThemeSettings(browser);
      break;
  }
}

void ApplyMoriAccelerators(std::vector<AcceleratorMapping>* accelerators) {
#if BUILDFLAG(IS_MAC)
  // macOS takes its shortcuts from the menu bar and
  // global_keyboard_shortcuts_mac.mm, where Mori's are, with Command for
  // Control.
  return;
#else
  for (const AcceleratorMapping& mori : kMoriAccelerators) {
    std::erase_if(*accelerators, [&](const AcceleratorMapping& existing) {
      return existing.keycode == mori.keycode &&
             existing.modifiers == mori.modifiers;
    });
    accelerators->push_back(mori);
  }
#endif
}

}  // namespace mori
