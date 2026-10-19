#include "chrome/browser/ui/views/mori/mori_toolbar_layout.h"

#include "build/build_config.h"
#include <algorithm>
#include <string_view>
#include <vector>

#include "base/functional/bind.h"
#include "chrome/app/chrome_command_ids.h"
#include "chrome/app/vector_icons/vector_icons.h"
#include "chrome/browser/ui/browser_commands.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/views/toolbar/reload_button.h"
#include "chrome/browser/ui/views/toolbar/toolbar_button.h"
#include "chrome/browser/ui/views/toolbar/toolbar_view.h"
#include "ui/gfx/geometry/rect.h"
#include "ui/gfx/geometry/size.h"
#include "ui/views/layout/flex_layout.h"
#include "ui/views/layout/proposed_layout.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/view.h"

DEFINE_UI_CLASS_PROPERTY_KEY(int, kMoriSidebarToolbarLeadingInset, -1)

namespace mori {

const ui::ClassProperty<int>* const kSidebarToolbarLeadingInsetKey =
    kMoriSidebarToolbarLeadingInset;

namespace {

// Mori 3's sidebar inset.
constexpr int kHorizontalPadding = 7;
constexpr int kTopPadding = 4;
constexpr int kRowGap = 2;
constexpr int kBottomPadding = 3;
constexpr int kButtonGap = 2;
// Used when the toolbar is asked for its preferred width.
constexpr int kDefaultWidth = 240;

enum class Slot {
  // The menu, extensions, downloads and other pinned actions: first row,
  // from the start (Mori 3 has its "..." menu there).
  kLeading,
  // Back, forward and reload: first row, at the end.
  kNavigation,
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

    std::vector<views::View*> leading;
    std::vector<views::View*> navigation;
    for (views::View* child : host_view()->children()) {
      if (!IsChildIncludedInLayout(child)) {
        continue;
      }
      switch (SlotFor(child)) {
        case Slot::kNavigation:
          navigation.push_back(child);
          break;
        case Slot::kLeading:
          leading.push_back(child);
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

    // Navigation buttons at the end of the first row.
    int right = width - kHorizontalPadding;
    for (auto it = navigation.rbegin(); it != navigation.rend(); ++it) {
      views::View* child = *it;
      const gfx::Size size = child->GetPreferredSize();
      right -= size.width();
      layout.child_layouts.push_back({child, true,
                                      RowRect(right, first_row_y, size),
                                      views::SizeBounds(size)});
      right -= kButtonGap;
    }

    // The menu and friends from the start, as long as they fit.
    int x = kHorizontalPadding + leading_inset;
    // Mori 3's order: the menu, the sidebar toggle, then the rest.
    std::ranges::stable_sort(leading, {}, [](const views::View* v) {
      const std::string_view name = v->GetClassName();
      return name == "BrowserAppMenuButton" ? 0
             : name == "MoriSidebarToggle"  ? 1
                                            : 2;
    });
    for (views::View* child : leading) {
      const gfx::Size size = child->GetPreferredSize();
      const bool fits = x + size.width() <= right;
      layout.child_layouts.push_back(
          {child, fits && size.width() > 0, RowRect(x, first_row_y, size),
           views::SizeBounds(size)});
      if (fits) {
        x += size.width() + kButtonGap;
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
        name == "BrowserAppMenuButton" || name == "MoriSidebarToggle") {
      return Slot::kLeading;
    }
    return Slot::kHidden;
  }

  raw_ptr<ToolbarView> toolbar_;
};

class MoriSidebarToggle : public ToolbarButton {
  METADATA_HEADER(MoriSidebarToggle, ToolbarButton)

 public:
  explicit MoriSidebarToggle(BrowserWindowInterface* browser)
      : ToolbarButton(base::BindRepeating(
            [](BrowserWindowInterface* browser) {
              // Hides the sidebar (Ctrl+S), as in Mori 3 and Arc.
              chrome::ExecuteCommand(browser, IDC_TOGGLE_VERTICAL_TABS);
            },
            browser)) {
    SetVectorIcon(kLeftPanelCloseFlippableIcon);
#if BUILDFLAG(IS_MAC)
    SetTooltipText(u"Hide sidebar (⌘S)");
#else
    SetTooltipText(u"Hide sidebar (Ctrl+S)");
#endif
    GetViewAccessibility().SetName(u"Toggle sidebar");
  }
};

BEGIN_METADATA(MoriSidebarToggle)
END_METADATA

}  // namespace

std::unique_ptr<views::View> CreateSidebarToggle(
    BrowserWindowInterface* browser) {
  return std::make_unique<MoriSidebarToggle>(browser);
}

std::unique_ptr<views::FlexLayout> CreateToolbarLayout(ToolbarView* toolbar) {
  return std::make_unique<SidebarToolbarLayout>(toolbar);
}

}  // namespace mori
