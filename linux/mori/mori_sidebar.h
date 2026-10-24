// Arc's Ctrl+S (and Mori's compact mode): the sidebar hides, the page takes
// the whole window, and touching the window's left edge slides the sidebar
// in over the page until the pointer moves away.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_SIDEBAR_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_SIDEBAR_H_

class BrowserWindowInterface;

namespace views {
class View;
}

namespace mori {

// Hides or shows the sidebar of `browser`'s window.
void ToggleSidebarHidden(BrowserWindowInterface* browser);

// Whether `browser`'s sidebar is hidden.
bool IsSidebarHiddenIn(BrowserWindowInterface* browser);

// Read by the browser layout (`browser_view` is the BrowserView).
bool IsSidebarHidden(const views::View* browser_view);
// The hidden sidebar is showing over the page.
bool IsSidebarPeeking(const views::View* browser_view);

// The floating sidebar's distance from the window edges.
inline constexpr int kSidebarPeekInset = 4;

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_SIDEBAR_H_
