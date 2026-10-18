#include "chrome/browser/ui/views/mori/mori_command_ids.h"

#include <algorithm>
#include <array>

#include "chrome/app/chrome_command_ids.h"
#include "chrome/browser/resource_coordinator/lifecycle_unit_state.mojom.h"
#include "chrome/browser/resource_coordinator/tab_lifecycle_unit_external.h"
#include "chrome/browser/ui/accelerator_table.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/navigator/browser_navigator.h"
#include "chrome/browser/ui/navigator/browser_navigator_params.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/mori/mori_spaces.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/web_contents.h"
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

constexpr int kCtrl = ui::EF_CONTROL_DOWN;
constexpr int kShift = ui::EF_SHIFT_DOWN;
constexpr int kAlt = ui::EF_ALT_DOWN;

// Mori 3's keymap (crates/mori/src/actions.rs) where Chromium differs.
constexpr AcceleratorMapping kMoriAccelerators[] = {
    // Arc: hide the sidebar; Zen/Mori 3 compact mode does the same here.
    {ui::VKEY_S, kCtrl, IDC_TOGGLE_VERTICAL_TABS},
    {ui::VKEY_C, kCtrl | kAlt, IDC_TOGGLE_VERTICAL_TABS},
    // Spaces.
    {ui::VKEY_RIGHT, kCtrl | kAlt, IDC_FOCUS_NEXT_TAB_GROUP},
    {ui::VKEY_LEFT, kCtrl | kAlt, IDC_FOCUS_PREV_TAB_GROUP},
    {ui::VKEY_1, kAlt, kCommandSelectSpace1},
    {ui::VKEY_2, kAlt, kCommandSelectSpace1 + 1},
    {ui::VKEY_3, kAlt, kCommandSelectSpace1 + 2},
    {ui::VKEY_4, kAlt, kCommandSelectSpace1 + 3},
    {ui::VKEY_5, kAlt, kCommandSelectSpace1 + 4},
    {ui::VKEY_6, kAlt, kCommandSelectSpace1 + 5},
    {ui::VKEY_7, kAlt, kCommandSelectSpace1 + 6},
    {ui::VKEY_8, kAlt, kCommandSelectSpace1 + 7},
    {ui::VKEY_9, kAlt, kCommandSelectSpace9},
    // Tabs.
    {ui::VKEY_DOWN, kCtrl | kAlt, IDC_SELECT_NEXT_TAB},
    {ui::VKEY_UP, kCtrl | kAlt, IDC_SELECT_PREVIOUS_TAB},
    {ui::VKEY_P, kCtrl | kShift, kCommandTogglePin},
    {ui::VKEY_C, kCtrl | kShift, IDC_COPY_URL},
    {ui::VKEY_D, kCtrl | kShift, IDC_DUPLICATE_TAB},
    {ui::VKEY_S, kCtrl | kAlt, kCommandSleepBackgroundTabs},
    {ui::VKEY_R, kCtrl | kAlt, IDC_SHOW_READING_MODE_SIDE_PANEL},
    // Places.
    {ui::VKEY_OEM_COMMA, kCtrl, IDC_OPTIONS},
    {ui::VKEY_Y, kCtrl, IDC_SHOW_HISTORY},
    {ui::VKEY_J, kCtrl | kShift, IDC_SHOW_DOWNLOADS},
};

}  // namespace

bool IsMoriCommand(int command_id) {
  return command_id >= kCommandFirst && command_id <= kCommandLast;
}

void ExecuteMoriCommand(BrowserWindowInterface* browser, int command_id) {
  if (command_id >= kCommandSelectSpace1 &&
      command_id <= kCommandSelectSpace9) {
    SelectSpace(browser, command_id - kCommandSelectSpace1);
    return;
  }
  switch (command_id) {
    case kCommandNewSpace:
      NewSpace(browser);
      break;
    case kCommandSleepBackgroundTabs:
      SleepBackgroundTabs(browser);
      break;
    case kCommandTogglePin:
      TogglePin(browser);
      break;
    case kCommandEditTheme: {
      NavigateParams params(browser, GURL("chrome://settings/appearance"),
                            ui::PAGE_TRANSITION_AUTO_TOPLEVEL);
      params.disposition = WindowOpenDisposition::SINGLETON_TAB;
      Navigate(&params);
      break;
    }
  }
}

void ApplyMoriAccelerators(std::vector<AcceleratorMapping>* accelerators) {
  for (const AcceleratorMapping& mori : kMoriAccelerators) {
    std::erase_if(*accelerators, [&](const AcceleratorMapping& existing) {
      return existing.keycode == mori.keycode &&
             existing.modifiers == mori.modifiers;
    });
    accelerators->push_back(mori);
  }
}

}  // namespace mori
