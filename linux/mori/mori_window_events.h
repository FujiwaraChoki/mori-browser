// Watching the input of one browser window, the same way on Linux (Aura)
// and macOS. On Aura the page is a window of its own, so Mori watches the
// whole application and keeps the events whose window belongs to this one;
// on macOS the page is part of the window, which a window monitor covers.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_WINDOW_EVENTS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_WINDOW_EVENTS_H_

#include <memory>
#include <set>

#include "ui/events/types/event_type.h"

namespace ui {
class Event;
class EventObserver;
}  // namespace ui

namespace views {
class EventMonitor;
class Widget;
}  // namespace views

namespace mori {

// Events of `types` in `widget`'s window, page included, go to `observer`.
// On Aura they may come from other windows too: check IsEventInWindow().
std::unique_ptr<views::EventMonitor> MonitorWindow(
    ui::EventObserver* observer,
    views::Widget* widget,
    const std::set<ui::EventType>& types);

// Whether `event`, from MonitorWindow(), happened in `widget`'s window.
// Located events from it are then in the window's coordinates.
bool IsEventInWindow(const ui::Event& event, const views::Widget* widget);

// Whether `event`, from MonitorWindow() on the browser window, happened in
// `child`, a widget over it (an overlay). On macOS a child widget is a
// window of its own, whose events the browser window's monitor never sees.
bool IsEventInChildWidget(const ui::Event& event, const views::Widget* child);

// macOS: shows or hides the window's traffic lights (Arc hides them with
// the sidebar and shows them when it slides out). Elsewhere, nothing.
void SetWindowButtonsVisible(views::Widget* widget, bool visible);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_WINDOW_EVENTS_H_
