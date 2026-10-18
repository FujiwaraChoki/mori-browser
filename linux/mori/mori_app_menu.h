// Mori's "..." menu (Mori 3's sidebar menu) in place of Chromium's app menu.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_APP_MENU_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_APP_MENU_H_

class BrowserWindowInterface;

namespace ui {
class SimpleMenuModel;
}  // namespace ui

namespace mori {

// Fills the app menu's (empty) `model` with Mori's items.
void BuildAppMenu(ui::SimpleMenuModel* model, BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_APP_MENU_H_
