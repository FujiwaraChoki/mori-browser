// The space editor (Mori 3's): name, symbol (a Lucide icon or an emoji)
// and gradient theme of a space, shown when a space is made and from its
// label. Changes apply as they're made.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_SPACE_EDITOR_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_SPACE_EDITOR_H_

class BrowserWindowInterface;

namespace mori {

// Edits the space on screen in `browser`.
void EditSpace(BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_SPACE_EDITOR_H_
