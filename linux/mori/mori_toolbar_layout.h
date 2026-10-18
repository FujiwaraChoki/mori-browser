// The toolbar at the top of Mori's sidebar (Arc and Zen's single toolbar):
// window buttons and navigation in the first row, the address field below.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_TOOLBAR_LAYOUT_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_TOOLBAR_LAYOUT_H_

#include <memory>

#include "ui/base/class_property.h"

class BrowserWindowInterface;
class ToolbarView;

namespace views {
class FlexLayout;
class View;
}  // namespace views

namespace mori {

// Set on the ToolbarView by the browser layout. -1 (the default) keeps
// Chromium's one-row toolbar. Otherwise the toolbar sits at the top of the
// sidebar in two rows, and the value is the width taken by the window
// buttons at the start of the first row.
extern const ui::ClassProperty<int>* const kSidebarToolbarLeadingInsetKey;

// Height of each of the sidebar toolbar's two rows.
inline constexpr int kSidebarToolbarRowHeight = 36;

// The toolbar's layout: Chromium's FlexLayout, switching to two rows while
// kSidebarToolbarLeadingInsetKey is set.
std::unique_ptr<views::FlexLayout> CreateToolbarLayout(ToolbarView* toolbar);

// The button next to the menu that collapses the sidebar (Mori 3's sidebar
// toggle). Its class name is "MoriSidebarToggle".
std::unique_ptr<views::View> CreateSidebarToggle(
    BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_TOOLBAR_LAYOUT_H_
