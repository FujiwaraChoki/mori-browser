// The Chrome Web Store in Mori: its buttons say "Add to Mori" and "Remove
// from Mori" (the store names the browser Chrome whatever it is), and it
// doesn't ask to switch to Chrome.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_WEB_STORE_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_WEB_STORE_H_

namespace content {
class WebContents;
}

namespace mori {

// Fixes up the store in `contents` (a tab); called as tab helpers attach.
void AttachWebStore(content::WebContents* contents);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_WEB_STORE_H_
