// Mori's toasts (Zen's): a short note in a pill at the top right of the
// page, such as "Opened in Work" or "Imported 3 spaces from Zen". It goes
// away after a few seconds or when clicked.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_TOAST_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_TOAST_H_

#include <string>

class BrowserWindowInterface;

namespace mori {

void ShowToast(BrowserWindowInterface* browser, const std::u16string& text);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_TOAST_H_
