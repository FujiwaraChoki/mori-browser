// The toolbar at the top of Mori's sidebar (Arc and Zen's single toolbar):
// window buttons and navigation in the first row, the address field below,
// with pinned extensions always visible and menu controls shown on hover.

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
// sidebar, and the value is the width taken by the window
// buttons at the start of the first row.
extern const ui::ClassProperty<int>* const kSidebarToolbarLeadingInsetKey;

// Extra space reserved inside the address field for its hover actions.
extern const ui::ClassProperty<int>* const kLocationBarTrailingInsetKey;

// Height of each sidebar toolbar row.
inline constexpr int kSidebarToolbarRowHeight = 36;

// The toolbar's layout: Chromium's FlexLayout, switching to sidebar rows while
// kSidebarToolbarLeadingInsetKey is set.
std::unique_ptr<views::FlexLayout> CreateToolbarLayout(ToolbarView* toolbar);

// Non-painting toolbar observer that manages address-field hover actions.
std::unique_ptr<views::View> CreateToolbarController(ToolbarView* toolbar);

// The button next to the menu that collapses the sidebar (Mori's sidebar
// toggle). Its class name is "MoriSidebarToggle".
std::unique_ptr<views::View> CreateSidebarToggle(
    BrowserWindowInterface* browser);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_TOOLBAR_LAYOUT_H_
