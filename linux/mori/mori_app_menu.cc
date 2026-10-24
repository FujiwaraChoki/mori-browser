#include "chrome/browser/ui/views/mori/mori_app_menu.h"

#include "chrome/app/chrome_command_ids.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/navigator/browser_navigator.h"
#include "chrome/browser/ui/navigator/browser_navigator_params.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/mori/mori_assistant.h"
#include "chrome/browser/ui/views/mori/mori_command_ids.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_sidebar.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/web_contents.h"
#include "ui/base/accelerators/accelerator.h"
#include "ui/base/models/image_model.h"
#include "ui/menus/simple_menu_model.h"
#include "ui/events/event_constants.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "url/gurl.h"

namespace mori {
namespace {

ui::ImageModel Icon(std::string_view lucide) {
  return ui::ImageModel::FromVectorIcon(LucideIcon(lucide),
                                        ui::kColorMenuIcon, 15);
}

}  // namespace

void BuildAppMenu(ui::SimpleMenuModel* model, BrowserWindowInterface* browser) {
  // Mori's order.
  model->AddItemWithIcon(IDC_NEW_TAB, u"New Tab", Icon("plus"));
  model->AddItemWithIcon(kCommandNewFolder, u"New Folder", Icon("folder-plus"));
  model->AddItemWithIcon(IDC_NEW_WINDOW, u"New Window", Icon("app-window"));
  model->AddItemWithIcon(IDC_NEW_INCOGNITO_WINDOW, u"Private Window",
                         Icon("hat-glasses"));
  model->AddSeparator(ui::NORMAL_SEPARATOR);
  model->AddItemWithIcon(IDC_RESTORE_TAB, u"Reopen Closed Tab",
                         Icon("rotate-ccw"));
  model->AddItemWithIcon(kCommandSleepBackgroundTabs, u"Sleep Background Tabs",
                         Icon("moon"));
  model->AddItemWithIcon(kCommandArchiveUnpinnedTabs, u"Archive Unpinned Tabs",
                         Icon("archive"));
  model->AddItemWithIcon(kCommandPeekClipboard, u"Peek a Link", Icon("eye"));
  model->AddSeparator(ui::NORMAL_SEPARATOR);
  model->AddItemWithIcon(kCommandToggleAssistant,
                         IsAssistantOpen(browser) ? u"Hide Assistant"
                                                  : u"Show Assistant",
                         Icon("sparkles"));
  model->AddItemWithIcon(IDC_TOGGLE_VERTICAL_TABS,
                         IsSidebarHiddenIn(browser) ? u"Show Sidebar"
                                                  : u"Hide Sidebar",
                         Icon("panel-left-close"));
  model->AddItemWithIcon(kCommandEditTheme, u"Edit Theme…", Icon("palette"));
  model->AddSeparator(ui::NORMAL_SEPARATOR);
  model->AddItemWithIcon(IDC_SHOW_DOWNLOADS, u"Downloads", Icon("download"));
  model->AddItemWithIcon(IDC_MANAGE_EXTENSIONS, u"Extensions", Icon("puzzle"));
  model->AddSeparator(ui::NORMAL_SEPARATOR);
  model->AddItemWithIcon(IDC_OPTIONS, u"Settings", Icon("settings"));
}

}  // namespace mori
