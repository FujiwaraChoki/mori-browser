#include "chrome/browser/ui/views/mori/mori_sidebar.h"

#include <memory>
#include <string_view>

#include "base/memory/raw_ptr.h"
#include "base/timer/timer.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/vertical_tab_strip_state_controller.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/frame/top_container_view.h"
#include "chrome/browser/ui/views/mori/mori_window_events.h"
#include "ui/compositor/layer.h"
#include "ui/gfx/geometry/rounded_corners_f.h"
#include "ui/base/class_property.h"
#include "ui/events/event.h"
#include "ui/events/event_observer.h"
#include "ui/gfx/geometry/point.h"
#include "ui/views/event_monitor.h"
#include "ui/views/widget/widget.h"

namespace mori {
namespace {

// As Mori 3: the edge that reveals the sidebar (6px from the window's edge,
// and the page starts about 7px outside the visible window, past the frame's
// shadow inset), and how far past the sidebar the pointer may stray before
// it hides again.
constexpr int kRevealEdge = 13;
constexpr int kHideMargin = 24;
constexpr base::TimeDelta kHideDelay = base::Milliseconds(250);
// Mori 3's floating sidebar corners (metrics::RADIUS).
constexpr float kFloatingRadius = 10;

class SidebarState : public ui::EventObserver {
 public:
  explicit SidebarState(BrowserView* browser_view)
      : browser_view_(browser_view) {}

  bool hidden() const { return hidden_; }
  bool peeking() const { return peeking_; }

  void Toggle() {
    hidden_ = !hidden_;
    peeking_ = false;
    hide_timer_.Stop();
    if (hidden_ && browser_view_->GetWidget()) {
      monitor_ = MonitorWindow(
          this, browser_view_->GetWidget(),
          {ui::EventType::kMouseMoved, ui::EventType::kMouseDragged,
           ui::EventType::kMouseExited});
    } else {
      monitor_.reset();
    }
    Relayout();
  }

  // ui::EventObserver:
  void OnEvent(const ui::Event& event) override {
    if (!hidden_) {
      return;
    }
    if (!event.IsLocatedEvent()) {
      return;
    }
    // Only this window's events; their root location is in its coordinates.
    if (!IsEventInWindow(event, browser_view_->GetWidget())) {
      return;
    }
    gfx::Point point =
        gfx::ToFlooredPoint(event.AsLocatedEvent()->root_location_f());
    views::View::ConvertPointFromWidget(browser_view_, &point);
    // An exit also comes when the pointer only crosses from the page onto
    // the floating sidebar; it counts as leaving only outside the window.
    const bool left_window =
        event.type() == ui::EventType::kMouseExited &&
        !browser_view_->GetLocalBounds().Contains(point);
    if (point.x() <= kRevealEdge && !left_window) {
      hide_timer_.Stop();
      if (!peeking_) {
        peeking_ = true;
        Relayout();
      }
      return;
    }
    if (!peeking_) {
      return;
    }
    if (!left_window &&
        point.x() <= SidebarWidth() + kSidebarPeekInset + kHideMargin) {
      hide_timer_.Stop();
    } else if (!hide_timer_.IsRunning()) {
      hide_timer_.Start(FROM_HERE, kHideDelay, this, &SidebarState::Hide);
    }
  }

 private:
  int SidebarWidth() const {
    auto* state =
        tabs::VerticalTabStripStateController::From(browser_view_->browser());
    return state ? state->GetUncollapsedWidth() : 186;
  }

  void Hide() {
    peeking_ = false;
    Relayout();
  }

  void Relayout() {
    // Peeking, the toolbar's container floats over the page with the tab
    // list (which already paints to its own layer). It needs a layer of its
    // own, above the page's. Together they make Mori 3's floating card, with
    // its rounded corners.
    views::View* top = browser_view_->top_container();
    views::View* tabs = nullptr;
    for (views::View* child : browser_view_->children()) {
      if (std::string_view(child->GetClassName()) ==
          "VerticalTabStripRegionView") {
        tabs = child;
      }
    }
    if (peeking_) {
      if (top && !top->layer()) {
        top->SetPaintToLayer();
        top->layer()->SetFillsBoundsOpaquely(false);
      }
      if (top && top->parent() == browser_view_) {
        browser_view_->ReorderChildView(top, browser_view_->children().size());
      }
    }
    const float radius = peeking_ ? kFloatingRadius : 0;
    if (top && top->layer()) {
      top->layer()->SetRoundedCornerRadius(
          gfx::RoundedCornersF(radius, radius, 0, 0));
    }
    if (tabs && tabs->layer()) {
      tabs->layer()->SetRoundedCornerRadius(
          gfx::RoundedCornersF(0, 0, radius, radius));
    }
    // macOS: the traffic lights go with the sidebar, as in Arc.
    SetWindowButtonsVisible(browser_view_->GetWidget(), !hidden_ || peeking_);
    browser_view_->InvalidateLayout();
    browser_view_->SchedulePaint();
  }

  raw_ptr<BrowserView> browser_view_;
  bool hidden_ = false;
  bool peeking_ = false;
  std::unique_ptr<views::EventMonitor> monitor_;
  base::OneShotTimer hide_timer_;
};

}  // namespace
}  // namespace mori

DEFINE_UI_CLASS_PROPERTY_TYPE(mori::SidebarState*)

namespace mori {
namespace {

DEFINE_OWNED_UI_CLASS_PROPERTY_KEY(SidebarState, kSidebarStateKey)

const SidebarState* StateOf(const views::View* browser_view) {
  return browser_view ? browser_view->GetProperty(kSidebarStateKey) : nullptr;
}

}  // namespace

void ToggleSidebarHidden(BrowserWindowInterface* browser) {
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view) {
    return;
  }
  SidebarState* state = browser_view->GetProperty(kSidebarStateKey);
  if (!state) {
    state = browser_view->SetProperty(
        kSidebarStateKey, std::make_unique<SidebarState>(browser_view));
  }
  state->Toggle();
}

bool IsSidebarHiddenIn(BrowserWindowInterface* browser) {
  return IsSidebarHidden(BrowserView::GetBrowserViewForBrowser(browser));
}

bool IsSidebarHidden(const views::View* browser_view) {
  const SidebarState* state = StateOf(browser_view);
  return state && state->hidden();
}

bool IsSidebarPeeking(const views::View* browser_view) {
  const SidebarState* state = StateOf(browser_view);
  return state && state->hidden() && state->peeking();
}

}  // namespace mori
