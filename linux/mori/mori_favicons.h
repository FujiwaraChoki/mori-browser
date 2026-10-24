// Icons for tabs that haven't loaded (imported or saved ones), as Mori
// fetches them: the favicon database first, then the site's /favicon.ico,
// kept as an on-demand favicon until the page itself is visited.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_FAVICONS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_FAVICONS_H_

namespace content {
class WebContents;
}

namespace mori {

void FetchIconForUnloadedTab(content::WebContents* contents);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_FAVICONS_H_
