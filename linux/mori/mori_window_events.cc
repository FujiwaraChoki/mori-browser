#include "chrome/browser/ui/views/mori/mori_window_events.h"

#include "build/build_config.h"
#include "ui/events/event.h"
#include "ui/views/event_monitor.h"
#include "ui/views/widget/widget.h"

#if defined(USE_AURA)
#include "ui/aura/window.h"
#endif

namespace mori {

std::unique_ptr<views::EventMonitor> MonitorWindow(
    ui::EventObserver* observer,
    views::Widget* widget,
    const std::set<ui::EventType>& types) {
#if defined(USE_AURA)
  // A window monitor misses events over the page, a window of its own.
  return views::EventMonitor::CreateApplicationMonitor(
      observer, widget->GetNativeWindow(), types);
#else
  return views::EventMonitor::CreateWindowMonitor(
      observer, widget->GetNativeWindow(), types);
#endif
}

bool IsEventInWindow(const ui::Event& event, const views::Widget* widget) {
  if (!widget) {
    return false;
  }
#if defined(USE_AURA)
  auto* target = static_cast<aura::Window*>(event.target());
  return target && target->GetRootWindow() ==
                       widget->GetNativeWindow()->GetRootWindow();
#else
  // The window monitor only sees this window.
  return true;
#endif
}

bool IsEventInChildWidget(const ui::Event& event, const views::Widget* child) {
  if (!child) {
    return false;
  }
#if defined(USE_AURA)
  auto* target = static_cast<aura::Window*>(event.target());
  return target && child->GetNativeView()->Contains(target);
#else
  return false;
#endif
}

#if !BUILDFLAG(IS_MAC)
void SetWindowButtonsVisible(views::Widget* widget, bool visible) {}
#endif

}  // namespace mori
