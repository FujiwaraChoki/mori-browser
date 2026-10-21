// Sizes for Mori's window layout, shared by the patched Chromium layout code.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_LAYOUT_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_LAYOUT_H_

namespace mori {

// With tabs in the sidebar the page is a card: inset from the window's right
// and bottom edges and rounded, like Zen Browser.
inline constexpr int kContentCardMargin = 8;
inline constexpr float kContentCardRadius = 10.0f;

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_LAYOUT_H_
