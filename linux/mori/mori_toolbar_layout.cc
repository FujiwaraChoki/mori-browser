#include "chrome/browser/ui/views/mori/mori_toolbar_layout.h"

#include <algorithm>
#include <string_view>
#include <vector>

#include "chrome/browser/ui/views/toolbar/reload_button.h"
#include "chrome/browser/ui/views/toolbar/toolbar_view.h"
#include "ui/gfx/geometry/rect.h"
#include "ui/gfx/geometry/size.h"
#include "ui/views/layout/flex_layout.h"
#include "ui/views/layout/proposed_layout.h"
#include "ui/views/view.h"

DEFINE_UI_CLASS_PROPERTY_KEY(int, kMoriSidebarToolbarLeadingInset, -1)

namespace mori {

const ui::ClassProperty<int>* const kSidebarToolbarLeadingInsetKey =
    kMoriSidebarToolbarLeadingInset;

namespace {

// Matches the sidebar's tab rows.
constexpr int kHorizontalPadding = 10;
constexpr int kTopPadding = 4;
constexpr int kRowGap = 2;
constexpr int kBottomPadding = 8;
constexpr int kButtonGap = 2;
// Used when the toolbar is asked for its preferred width.
constexpr int kDefaultWidth = 240;

enum class Slot {
  // Back, forward and reload: first row, after the window buttons.
  kNavigation,
  // Downloads and other pinned actions, extensions, the menu: first row,
  // at the end.
  kTrailing,
  // The address field: the whole second row.
  kAddress,
  // Everything else (profile button, home, overflow, ...) is hidden.
  kHidden,
};

class SidebarToolbarLayout : public views::FlexLayout {
 public:
  explicit SidebarToolbarLayout(ToolbarView* toolbar) : toolbar_(toolbar) {}

  // The first row is the least the sidebar toolbar needs. The frame sizes
  // the window buttons from this, which keeps them in the first row.
  gfx::Size GetMinimumSize(const views::View* host) const override {
    if (toolbar_->GetProperty(kSidebarToolbarLeadingInsetKey) < 0) {
      return views::FlexLayout::GetMinimumSize(host);
    }
    return gfx::Size(0, kTopPadding + kSidebarToolbarRowHeight);
  }

 protected:
  views::ProposedLayout CalculateProposedLayout(
      const views::SizeBounds& size_bounds) const override {
    const int leading_inset =
        toolbar_->GetProperty(kSidebarToolbarLeadingInsetKey);
    if (leading_inset < 0) {
      return views::FlexLayout::CalculateProposedLayout(size_bounds);
    }

    const int width = size_bounds.width().is_bounded()
                          ? size_bounds.width().value()
                          : kDefaultWidth;
    const int first_row_y = kTopPadding;
    const int second_row_y = first_row_y + kSidebarToolbarRowHeight + kRowGap;
    views::ProposedLayout layout;
    layout.host_size = gfx::Size(
        width, second_row_y + kSidebarToolbarRowHeight + kBottomPadding);

    std::vector<views::View*> navigation;
    std::vector<views::View*> trailing;
    for (views::View* child : host_view()->children()) {
      if (!IsChildIncludedInLayout(child)) {
        continue;
      }
      switch (SlotFor(child)) {
        case Slot::kNavigation:
          navigation.push_back(child);
          break;
        case Slot::kTrailing:
          trailing.push_back(child);
          break;
        case Slot::kAddress: {
          const int height = std::min(kSidebarToolbarRowHeight,
                                      child->GetPreferredSize().height());
          layout.child_layouts.push_back(
              {child, true,
               gfx::Rect(kHorizontalPadding,
                         second_row_y + (kSidebarToolbarRowHeight - height) / 2,
                         std::max(0, width - 2 * kHorizontalPadding), height),
               views::SizeBounds(width, kSidebarToolbarRowHeight)});
          break;
        }
        case Slot::kHidden:
          layout.child_layouts.push_back({child, false});
          break;
      }
    }

    // Navigation buttons from the start of the first row.
    int x = kHorizontalPadding + leading_inset;
    for (views::View* child : navigation) {
      const gfx::Size size = child->GetPreferredSize();
      layout.child_layouts.push_back({child, true, RowRect(x, first_row_y, size),
                                      views::SizeBounds(size)});
      x += size.width() + kButtonGap;
    }

    // Trailing buttons from the end, as long as they fit.
    int right = width - kHorizontalPadding;
    for (auto it = trailing.rbegin(); it != trailing.rend(); ++it) {
      views::View* child = *it;
      const gfx::Size size = child->GetPreferredSize();
      const bool fits = right - size.width() >= x;
      if (fits) {
        right -= size.width();
      }
      layout.child_layouts.push_back(
          {child, fits && size.width() > 0, RowRect(right, first_row_y, size),
           views::SizeBounds(size)});
      if (fits) {
        right -= kButtonGap;
      }
    }
    return layout;
  }

 private:
  static gfx::Rect RowRect(int x, int row_y, const gfx::Size& size) {
    const int height = std::min(kSidebarToolbarRowHeight, size.height());
    return gfx::Rect(x, row_y + (kSidebarToolbarRowHeight - height) / 2,
                     size.width(), height);
  }

  Slot SlotFor(const views::View* child) const {
    if (child == toolbar_->location_bar_view()) {
      return Slot::kAddress;
    }
    // Most of these members are private to ToolbarView; match them by class.
    const std::string_view name = child->GetClassName();
    if (name == "BackForwardButton" || child == toolbar_->reload_button()) {
      return Slot::kNavigation;
    }
    if (name == "PinnedToolbarActionsContainer" ||
        name == "ExtensionsToolbarDesktop" ||
        name == "BrowserAppMenuButton") {
      return Slot::kTrailing;
    }
    return Slot::kHidden;
  }

  raw_ptr<ToolbarView> toolbar_;
};

}  // namespace

std::unique_ptr<views::FlexLayout> CreateToolbarLayout(ToolbarView* toolbar) {
  return std::make_unique<SidebarToolbarLayout>(toolbar);
}

}  // namespace mori
