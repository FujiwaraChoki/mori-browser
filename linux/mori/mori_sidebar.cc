#include "chrome/browser/ui/views/mori/mori_sidebar.h"

#include <algorithm>
#include <cmath>
#include <iterator>
#include <memory>
#include <string_view>
#include <vector>

#include "base/memory/raw_ptr.h"
#include "base/timer/timer.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/vertical_tab_strip_state_controller.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/frame/top_container_view.h"
#include "chrome/browser/ui/views/mori/mori_window_events.h"
#include "ui/compositor/layer.h"
#include "ui/gfx/animation/linear_animation.h"
#include "ui/gfx/geometry/cubic_bezier.h"
#include "ui/gfx/geometry/rounded_corners_f.h"
#include "ui/gfx/geometry/transform.h"
#include "ui/base/class_property.h"
#include "ui/events/event.h"
#include "ui/events/event_constants.h"
#include "ui/events/event_observer.h"
#include "ui/gfx/geometry/point.h"
#include "ui/views/animation/animation_delegate_views.h"
#include "ui/views/event_monitor.h"
#include "ui/views/widget/widget.h"

namespace mori {
namespace {

// The floating sidebar follows Zen Browser's compact mode
// (ZenCompactMode.mjs, zen-compact-mode.css, and Zen's default prefs).
//
// The edge that reveals the sidebar: Zen leaves a few pixels of the hidden
// sidebar at the window's edge. Here it's 6px from the window's edge, and
// the page starts about 7px outside the visible window, past the frame's
// shadow inset.
constexpr int kRevealEdge = 13;
// The hover area reaches past the card by Zen's padding around it
// (--zen-compact-float, the element separation).
constexpr int kHoverPadding = 8;
// How long the sidebar stays after the pointer leaves it
// (zen.view.compact.sidebar-keep-hover.duration).
constexpr base::TimeDelta kKeepHover = base::Milliseconds(150);
// Leaving the window this close to an edge counts as crossing it
// (_getCrossedEdge), along the sidebar give or take this much
// (_positionInBounds).
constexpr int kCrossedEdgeDistance = 10;
constexpr int kCrossedEdgeError = 7;
// Crossing the sidebar's edge shows it this long, as Zen does where it
// can't follow the pointer outside the window (Linux)
// (zen.view.compact.toolbar-hide-after-hover.duration).
constexpr base::TimeDelta kShowAfterLeaving = base::Milliseconds(1000);
// The slide in, on Zen's spring (--zen-compact-mode-func), and out, on CSS
// `ease`.
constexpr base::TimeDelta kSlideIn = base::Milliseconds(250);
constexpr base::TimeDelta kSlideOut = base::Milliseconds(150);
// --zen-compact-mode-func every 5%: a spring that overshoots by 1%.
constexpr float kSpring[] = {
    0.f,      0.058151f, 0.188662f, 0.344288f, 0.497f,    0.632052f,
    0.743381f, 0.830233f, 0.894782f, 0.940547f, 0.971397f, 0.990984f,
    1.002456f, 1.008359f, 1.010649f, 1.010757f, 1.009681f, 1.008083f,
    1.006372f, 1.004782f, 1.003423f};
// Mori 3's floating sidebar corners (metrics::RADIUS).
constexpr float kFloatingRadius = 10;

double Spring(double t) {
  const double at = std::clamp(t, 0.0, 1.0) * (std::size(kSpring) - 1);
  const size_t i = std::min(static_cast<size_t>(at), std::size(kSpring) - 2);
  return kSpring[i] + (kSpring[i + 1] - kSpring[i]) * (at - i);
}

enum class Edge { kNone, kTop, kBottom, kLeft, kRight };

class SidebarState : public ui::EventObserver,
                     public views::AnimationDelegateViews {
 public:
  explicit SidebarState(BrowserView* browser_view)
      : views::AnimationDelegateViews(browser_view),
        browser_view_(browser_view),
        slide_(this) {}

  bool hidden() const { return hidden_; }
  bool peeking() const { return peeking_; }

  void Toggle() {
    hidden_ = !hidden_;
    peeking_ = false;
    showing_ = false;
    leaving_window_ = false;
    keep_timer_.Stop();
    show_timer_.Stop();
    slide_.Stop();
    // Docked, the sidebar sits where the layout puts it.
    shown_ = 1;
    ApplySlide();
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
    const bool exited = event.type() == ui::EventType::kMouseExited;
    // Aura sends synthesized exits as the pointer crosses between the
    // window's own parts (the page, the floating sidebar); only an exit
    // from the compositor means it left the window.
    if (exited && (event.flags() & ui::EF_IS_SYNTHESIZED)) {
      return;
    }
    gfx::Point point =
        gfx::ToFlooredPoint(event.AsLocatedEvent()->root_location_f());
    views::View::ConvertPointFromWidget(browser_view_, &point);
    if (exited) {
      OnLeftWindow(point);
      return;
    }
    const bool over = point.x() <= kRevealEdge || IsOverSidebar(point);
    // Back in the window after crossing the sidebar's edge: the first move
    // anywhere else closes it at once.
    if (leaving_window_) {
      leaving_window_ = false;
      if (!over) {
        Hide();
        return;
      }
    }
    if (over) {
      Show();
    } else if (showing_ && !keep_timer_.IsRunning() &&
               !show_timer_.IsRunning()) {
      keep_timer_.Start(FROM_HERE, kKeepHover, this, &SidebarState::Hide);
    }
  }

  // gfx::AnimationDelegate:
  void AnimationProgressed(const gfx::Animation* animation) override {
    const double t = slide_.GetCurrentValue();
    const double curve = showing_ ? Spring(t) : ease_.Solve(t);
    shown_ = slide_from_ + ((showing_ ? 1 : 0) - slide_from_) * curve;
    ApplySlide();
  }
  void AnimationEnded(const gfx::Animation* animation) override {
    FinishSlide();
  }
  void AnimationCanceled(const gfx::Animation* animation) override {}

 private:
  int SidebarWidth() const {
    auto* state =
        tabs::VerticalTabStripStateController::From(browser_view_->browser());
    return state ? state->GetUncollapsedWidth() : 186;
  }

  // Wayland gives no position for the exit: `point` is where the pointer
  // was last seen inside the window.
  void OnLeftWindow(const gfx::Point& point) {
    const gfx::Rect card = CardBounds();
    if (CrossedEdge(point) == Edge::kLeft &&
        card.y() - kCrossedEdgeError < point.y() &&
        point.y() < card.bottom() + kCrossedEdgeError) {
      Show();
      leaving_window_ = true;
      show_timer_.Start(FROM_HERE, kShowAfterLeaving, this,
                        &SidebarState::Hide);
      return;
    }
    if (!showing_) {
      return;
    }
    if (IsOverSidebar(point)) {
      // Into a menu opened from the sidebar: it stays.
      keep_timer_.Stop();
    } else if (!keep_timer_.IsRunning() && !show_timer_.IsRunning()) {
      keep_timer_.Start(FROM_HERE, kKeepHover, this, &SidebarState::Hide);
    }
  }

  // The window edge within kCrossedEdgeDistance of `point`, checked in
  // Zen's order.
  Edge CrossedEdge(const gfx::Point& point) const {
    const gfx::Rect bounds = browser_view_->GetLocalBounds();
    const int x = std::clamp(point.x(), bounds.x(), bounds.right());
    const int y = std::clamp(point.y(), bounds.y(), bounds.bottom());
    if (std::abs(y - bounds.y()) <= kCrossedEdgeDistance) {
      return Edge::kTop;
    }
    if (std::abs(y - bounds.bottom()) <= kCrossedEdgeDistance) {
      return Edge::kBottom;
    }
    if (std::abs(x - bounds.x()) <= kCrossedEdgeDistance) {
      return Edge::kLeft;
    }
    if (std::abs(x - bounds.right()) <= kCrossedEdgeDistance) {
      return Edge::kRight;
    }
    return Edge::kNone;
  }

  // The floating card, where the layout puts it once it's in.
  gfx::Rect CardBounds() const {
    gfx::Rect card;
    if (peeking_) {
      for (views::View* view : PeekViews()) {
        card.Union(view->bounds());
      }
    }
    if (card.IsEmpty()) {
      const gfx::Rect bounds = browser_view_->GetLocalBounds();
      card = gfx::Rect(bounds.x() + kSidebarPeekInset,
                       bounds.y() + kSidebarPeekInset, SidebarWidth(),
                       bounds.height() - 2 * kSidebarPeekInset);
    }
    return card;
  }

  // Whether `point` is on the card (with Zen's padding), where it is now.
  bool IsOverSidebar(const gfx::Point& point) const {
    if (!peeking_) {
      return false;
    }
    gfx::Rect area = CardBounds();
    area.Inset(gfx::Insets::TLBR(0, 0, 0, -kHoverPadding));
    area.Offset(SlideOffset(), 0);
    return point.x() < area.right() && area.y() <= point.y() &&
           point.y() < area.bottom();
  }

  // Slide the floating sidebar in, from wherever it is.
  void Show() {
    keep_timer_.Stop();
    show_timer_.Stop();
    if (!peeking_) {
      peeking_ = true;
      Relayout();
      // It starts out of sight to the left.
      shown_ = 0;
      ApplySlide();
    }
    if (!showing_) {
      showing_ = true;
      StartSlide();
    }
  }

  // Slide the floating sidebar out, then take it out of the layout.
  void Hide() {
    keep_timer_.Stop();
    show_timer_.Stop();
    if (!peeking_ || !showing_) {
      return;
    }
    showing_ = false;
    StartSlide();
  }

  // A slide that reverses halfway carries on from where the card is, as a
  // CSS transition does.
  void StartSlide() {
    slide_.Stop();
    slide_from_ = shown_;
    const double distance = std::abs((showing_ ? 1 : 0) - shown_);
    if (distance < 0.01) {
      FinishSlide();
      return;
    }
    slide_.SetDuration((showing_ ? kSlideIn : kSlideOut) * distance);
    slide_.Start();
  }

  void FinishSlide() {
    shown_ = showing_ ? 1 : 0;
    ApplySlide();
    if (!showing_ && peeking_) {
      peeking_ = false;
      Relayout();
    }
  }

  // How far left of its place the card is now.
  int SlideOffset() const {
    return std::lround(-(SidebarWidth() + 2 * kSidebarPeekInset) *
                       (1 - shown_));
  }

  void ApplySlide() {
    gfx::Transform transform;
    transform.Translate(SlideOffset(), 0);
    for (views::View* view : PeekViews()) {
      if (view->layer()) {
        view->layer()->SetTransform(transform);
      }
    }
  }

  views::View* TabsRegion() const {
    for (views::View* child : browser_view_->children()) {
      if (std::string_view(child->GetClassName()) ==
          "VerticalTabStripRegionView") {
        return child;
      }
    }
    return nullptr;
  }

  // The views that make up the floating sidebar: the toolbar's container
  // and the tab list.
  std::vector<views::View*> PeekViews() const {
    std::vector<views::View*> views;
    views::View* top = browser_view_->top_container();
    for (views::View* view : {top, TabsRegion()}) {
      if (view) {
        views.push_back(view);
      }
    }
    return views;
  }

  void Relayout() {
    // Peeking, the toolbar's container floats over the page with the tab
    // list (which already paints to its own layer). It needs a layer of its
    // own, above the page's. Together they make Mori 3's floating card, with
    // its rounded corners.
    views::View* top = browser_view_->top_container();
    views::View* tabs = TabsRegion();
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
  // Peeking, whether it's (sliding) in, and how far in it is (0 to 1).
  bool showing_ = false;
  double shown_ = 1;
  double slide_from_ = 0;
  // The pointer left the window across the sidebar's edge.
  bool leaving_window_ = false;
  std::unique_ptr<views::EventMonitor> monitor_;
  base::OneShotTimer keep_timer_;
  base::OneShotTimer show_timer_;
  gfx::LinearAnimation slide_;
  const gfx::CubicBezier ease_{0.25, 0.1, 0.25, 1};
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
