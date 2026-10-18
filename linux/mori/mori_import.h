// Importing spaces from other browsers (Mori 3's, Arc's onboarding): Zen's
// workspaces and essentials, Firefox's windows, and Arc's sidebar (a copy
// of StorableSidebar.json in the home or Downloads folder, as Arc has no
// Linux build). Each workspace or window becomes a space, essentials and
// Arc's favorites become favorites. Tabs come in unloaded, as restored
// tabs do. Bookmarks and history import through Chromium's importer.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_IMPORT_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_IMPORT_H_

class BrowserWindowInterface;

namespace mori {

enum class ImportSource { kZen, kFirefox, kArc };

// Reads the source in the background, then adds its spaces to `browser`.
void ImportSpaces(BrowserWindowInterface* browser, ImportSource source);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_IMPORT_H_
