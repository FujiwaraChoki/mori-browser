#include "chrome/browser/ui/views/mori/mori_toolbar_layout.h"

#include "build/build_config.h"
#include <algorithm>
#include <cmath>
#include <optional>
#include <string_view>
#include <vector>

#include "base/functional/bind.h"
#include "base/scoped_observation.h"
#include "base/memory/weak_ptr.h"
#include "base/task/single_thread_task_runner.h"
#include "base/timer/timer.h"
#include "chrome/app/chrome_command_ids.h"
#include "chrome/app/vector_icons/vector_icons.h"
#include "chrome/browser/ui/browser_commands.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/views/toolbar/reload_button.h"
#include "chrome/browser/ui/views/extensions/extensions_toolbar_desktop.h"
#include "chrome/browser/ui/views/mori/mori_window_events.h"
#include "chrome/browser/ui/views/toolbar/toolbar_button.h"
#include "chrome/browser/ui/views/toolbar/toolbar_view.h"
#include "ui/compositor/layer.h"
#include "ui/display/screen.h"
#include "ui/events/event.h"
#include "ui/events/event_observer.h"
#include "ui/gfx/animation/animation_delegate.h"
#include "ui/gfx/animation/slide_animation.h"
#include "ui/gfx/geometry/point_conversions.h"
#include "ui/gfx/geometry/rect.h"
#include "ui/gfx/geometry/size.h"
#include "ui/views/layout/flex_layout.h"
#include "ui/views/layout/proposed_layout.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/event_monitor.h"
#include "ui/views/focus/focus_manager.h"
#include "ui/views/widget/widget.h"
#include "ui/views/view.h"
#include "ui/views/view_utils.h"
#include "ui/views/view_class_properties.h"
#include "ui/views/view_observer.h"

DEFINE_UI_CLASS_PROPERTY_KEY(int, kMoriSidebarToolbarLeadingInset, -1)
DEFINE_UI_CLASS_PROPERTY_KEY(int, kMoriLocationBarTrailingInset, 0)

namespace mori {

const ui::ClassProperty<int>* const kSidebarToolbarLeadingInsetKey =
    kMoriSidebarToolbarLeadingInset;
const ui::ClassProperty<int>* const kLocationBarTrailingInsetKey =
    kMoriLocationBarTrailingInset;

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
  // The menu, downloads and other pinned browser actions: first row,
  // from the start (Mori 3 has its "..." menu there).
  kLeading,
  // Back, forward and reload: first row, at the end.
  kNavigation,
  // The address field: the whole second row.
  kAddress,
  // Extension actions and their pinning menu: inside the address pill.
  kExtensions,
  // Everything else (profile button, home, overflow, ...) is hidden.
  kHidden,
};

Slot SlotFor(const ToolbarView* toolbar, const views::View* child) {
  if (child == toolbar->location_bar_view()) {
    return Slot::kAddress;
  }
  // Most of these members are private to ToolbarView; match them by class.
  const std::string_view name = child->GetClassName();
  if (name == "BackForwardButton" || child == toolbar->reload_button()) {
    return Slot::kNavigation;
  }
  if (name == "ExtensionsToolbarDesktop") {
    return Slot::kExtensions;
  }
  if (name == "PinnedToolbarActionsContainer" || name == "BrowserAppMenuButton" ||
      name == "MoriSidebarToggle") {
    return Slot::kLeading;
  }
  return Slot::kHidden;
}

ExtensionsToolbarDesktop* ExtensionsFor(ToolbarView* toolbar) {
  for (views::View* child : toolbar->children()) {
    if (auto* extensions = views::AsViewClass<ExtensionsToolbarDesktop>(child)) {
      return extensions;
    }
  }
  return nullptr;
}

int ExtensionWidth(int toolbar_width, int preferred_width) {
  // Leave at least half of the address field for the URL and page actions.
  // The native extension layout keeps its menu when pinned icons overflow.
  return std::min(preferred_width,
                  std::max(0, (toolbar_width - 2 * kHorizontalPadding) / 2));
}

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
    views::View* extensions = nullptr;
    for (views::View* child : host_view()->children()) {
      if (!IsChildIncludedInLayout(child)) {
        continue;
      }
      switch (SlotFor(toolbar_, child)) {
        case Slot::kNavigation:
          navigation.push_back(child);
          break;
        case Slot::kLeading:
          leading.push_back(child);
          break;
        case Slot::kExtensions:
          extensions = child;
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
    if (extensions) {
      const int available_width = ExtensionWidth(width, width);
      const gfx::Size preferred = extensions->GetPreferredSize();
      const gfx::Size size(std::min(available_width, preferred.width()),
                           std::min(kSidebarToolbarRowHeight,
                                    preferred.height()));
      const int extension_x = width - kHorizontalPadding - 4 - size.width();
      // A toolbar sibling drawn over the field keeps Chromium's native
      // extension ownership, pinning, drag-and-drop and popup anchors intact.
      // Advertise the full capacity, not the current animated width, so
      // pinning an extension can grow the container to the left of its menu.
      layout.child_layouts.push_back(
          {extensions, !size.IsEmpty(), RowRect(extension_x, second_row_y, size),
           views::SizeBounds(available_width, kSidebarToolbarRowHeight)});
    }
    return layout;
  }

 private:
  static gfx::Rect RowRect(int x, int row_y, const gfx::Size& size) {
    const int height = std::min(kSidebarToolbarRowHeight, size.height());
    return gfx::Rect(x, row_y + (kSidebarToolbarRowHeight - height) / 2,
                     size.width(), height);
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

// Observes toolbar input and focus to reveal address-field actions. The top
// row uses bare buttons, with no shared background around either group.
class MoriToolbarController : public views::View,
                              public views::ViewObserver,
                              public ui::EventObserver,
                              public views::FocusChangeListener,
                              public gfx::AnimationDelegate {
  METADATA_HEADER(MoriToolbarController, views::View)

 public:
  using views::View::OnEvent;

  explicit MoriToolbarController(ToolbarView* toolbar) : toolbar_(toolbar) {
    SetProperty(views::kViewIgnoredByLayoutKey, true);
    SetCanProcessEventsWithinSubtree(false);
    action_animation_.SetSlideDuration(base::Milliseconds(160));
    action_animation_.SetTweenType(gfx::Tween::EASE_IN_OUT);
    observation_.Observe(toolbar);
    StartMonitoring();
  }

  ~MoriToolbarController() override { StopMonitoring(); }

  // views::ViewObserver:
  void OnViewBoundsChanged(views::View* observed) override {
    SetBoundsRect(gfx::Rect(observed->size()));
    UpdateAddressActions();
  }
  void OnViewAddedToWidget(views::View*) override { StartMonitoring(); }
  void OnViewRemovedFromWidget(views::View*) override { StopMonitoring(); }
  void OnViewPropertyChanged(views::View*, const void* key, int64_t) override {
    if (key == kSidebarToolbarLeadingInsetKey) {
      UpdateAddressActions();
    }
  }
  void OnViewIsDeleting(views::View* observed) override {
    StopMonitoring();
    observation_.Reset();
  }

  // Observe the whole native window so leaving the sibling extensions view
  // hides the controls too, rather than leaving them stuck after a hover.
  void OnEvent(const ui::Event& event) override {
    if (IsEventInWindow(event, toolbar_->GetWidget())) {
      if (event.IsLocatedEvent()) {
        if (event.type() == ui::EventType::kMouseExited) {
          // Child-view exits can be synthesized while crossing the pill.
          if (!(event.flags() & ui::EF_IS_SYNTHESIZED)) {
            pointer_exited_ = true;
          }
        } else {
          pointer_in_widget_ = gfx::ToFlooredPoint(
              event.AsLocatedEvent()->root_location_f());
          pointer_exited_ = false;
        }
      }
      UpdateAddressActions();
      // Button actions and focus changes run after input observers.
      base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
          FROM_HERE, base::BindOnce(&MoriToolbarController::UpdateAddressActions,
                                   weak_factory_.GetWeakPtr()));
    }
  }
  void OnDidChangeFocus(views::View*, views::View*) override {
    UpdateAddressActions();
  }

  // gfx::AnimationDelegate:
  void AnimationProgressed(const gfx::Animation*) override {
    ApplyAddressActions();
  }
  void AnimationEnded(const gfx::Animation*) override {
    ApplyAddressActions();
  }


 private:
  void StartMonitoring() {
    if (!toolbar_->GetWidget() || monitor_) {
      return;
    }
    monitor_ = MonitorWindow(
        this, toolbar_->GetWidget(),
        {ui::EventType::kMouseMoved, ui::EventType::kMouseDragged,
         ui::EventType::kMouseExited, ui::EventType::kMousePressed, ui::EventType::kMouseReleased,
         ui::EventType::kKeyPressed});
    focus_manager_ = toolbar_->GetFocusManager();
    if (focus_manager_) {
      focus_manager_->AddFocusChangeListener(this);
    }
    UpdateAddressActions();
  }

  void StopMonitoring() {
    weak_factory_.InvalidateWeakPtrs();
    popup_timer_.Stop();
    action_animation_.Reset(action_animation_.GetCurrentValue());
    pointer_in_widget_.reset();
    pointer_exited_ = false;
    monitor_.reset();
    if (focus_manager_) {
      focus_manager_->RemoveFocusChangeListener(this);
      focus_manager_ = nullptr;
    }
  }

  void UpdateAddressActions() {
    if (!observation_.IsObserving() || !toolbar_->GetWidget()) {
      return;
    }
    auto* extensions = ExtensionsFor(toolbar_);
    views::View* address = toolbar_->location_bar_view();
    if (!extensions || !address) {
      return;
    }
    const bool sidebar = toolbar_->GetProperty(kSidebarToolbarLeadingInsetKey) >= 0;
    const bool popup = extensions->HasActivePopup();
    const views::View* focused = focus_manager_
                                    ? focus_manager_->GetFocusedView() : nullptr;
    const bool keyboard_focus = toolbar_->GetWidget()->IsActive() && focused &&
        (address->Contains(focused) || extensions->Contains(focused));
    bool over_address = false;
    if (pointer_in_widget_) {
      gfx::Point point = *pointer_in_widget_;
      views::View::ConvertPointFromWidget(address, &point);
      over_address = address->GetLocalBounds().Contains(point);
    } else if (display::Screen::Get()) {
      // Before the first input event, preserve hover when a window relayouts.
      over_address = address->GetBoundsInScreen().Contains(
          display::Screen::Get()->GetCursorScreenPoint());
    }
    const bool hover = toolbar_->GetWidget()->IsActive() && toolbar_->IsDrawn() &&
                       !pointer_exited_ && over_address;
    const bool reveal = !sidebar || hover || keyboard_focus || popup;
    // Reverse from the current value when the pointer crosses the edge again.
    // Native menus keep their anchor fully visible, and reduced motion skips
    // both the opacity transition and the address field's changing inset.
    revealed_ = reveal;
    if (!sidebar || popup || !gfx::Animation::ShouldRenderRichAnimation()) {
      action_animation_.Reset(reveal ? 1.0 : 0.0);
    } else if (reveal) {
      action_animation_.Show();
    } else {
      action_animation_.Hide();
    }
    ApplyAddressActions();
    // An open native popup may stop delivering events to the browser window.
    // Check only while it is open, so it hides again immediately on dismissal.
    if (popup && !popup_timer_.IsRunning()) {
      popup_timer_.Start(FROM_HERE, base::Milliseconds(100), this,
                         &MoriToolbarController::UpdateAddressActions);
    } else if (!popup) {
      popup_timer_.Stop();
    }
  }

  void ApplyAddressActions() {
    if (!observation_.IsObserving() || !toolbar_->GetWidget()) {
      return;
    }
    auto* extensions = ExtensionsFor(toolbar_);
    views::View* address = toolbar_->location_bar_view();
    if (!extensions || !address) {
      return;
    }
    if (!extensions->layer()) {
      extensions->SetPaintToLayer();
      extensions->layer()->SetFillsBoundsOpaquely(false);
    }
    const double progress = action_animation_.GetCurrentValue();
    extensions->layer()->SetOpacity(static_cast<float>(progress));
    // Fading controls must not intercept clicks after the pointer leaves.
    extensions->SetCanProcessEventsWithinSubtree(revealed_);
    const bool sidebar =
        toolbar_->GetProperty(kSidebarToolbarLeadingInsetKey) >= 0;
    const int full_inset = sidebar && extensions->GetVisible()
                               ? ExtensionWidth(
                                     toolbar_->width(),
                                     extensions->GetPreferredSize().width()) + 8
                               : 0;
    const int inset = static_cast<int>(std::lround(full_inset * progress));
    if (address->GetProperty(kLocationBarTrailingInsetKey) != inset) {
      address->SetProperty(kLocationBarTrailingInsetKey, inset);
      address->InvalidateLayout();
    }
  }

  raw_ptr<ToolbarView> toolbar_;
  raw_ptr<views::FocusManager> focus_manager_ = nullptr;
  std::unique_ptr<views::EventMonitor> monitor_;
  base::RepeatingTimer popup_timer_;
  bool revealed_ = false;
  bool pointer_exited_ = false;
  std::optional<gfx::Point> pointer_in_widget_;
  gfx::SlideAnimation action_animation_{this};
  base::ScopedObservation<views::View, views::ViewObserver> observation_{this};
  base::WeakPtrFactory<MoriToolbarController> weak_factory_{this};
};

BEGIN_METADATA(MoriToolbarController)
END_METADATA

}  // namespace

std::unique_ptr<views::View> CreateSidebarToggle(
    BrowserWindowInterface* browser) {
  return std::make_unique<MoriSidebarToggle>(browser);
}

std::unique_ptr<views::View> CreateToolbarController(ToolbarView* toolbar) {
  return std::make_unique<MoriToolbarController>(toolbar);
}

std::unique_ptr<views::FlexLayout> CreateToolbarLayout(ToolbarView* toolbar) {
  return std::make_unique<SidebarToolbarLayout>(toolbar);
}

}  // namespace mori
