// A floating panel over the page in a browser window (the Ctrl+Tab
// switcher, the shortcut cheatsheet): a child widget, as Chromium's find
// bar, which leaves the window active.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_OVERLAY_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_OVERLAY_H_

#include <memory>

#include "base/memory/raw_ptr.h"
#include "base/scoped_observation.h"
#include "ui/gfx/geometry/rect.h"
#include "ui/views/view_tracker.h"
#include "ui/views/widget/widget_observer.h"

class BrowserView;

namespace gfx {
class Canvas;
}

namespace views {
class View;
class Widget;
}  // namespace views

namespace mori {

// Place a TYPE_CONTROL child widget using bounds in BrowserView coordinates.
// These widgets take parent-relative bounds, unlike top-level/popup widgets.
void PlaceChildWidget(BrowserView* browser_view,
                      views::Widget* widget,
                      const gfx::Rect& bounds);

class Overlay : public views::WidgetObserver {
 public:
  // Page content and sheets share the browser compositor below floating chrome.
  enum class Layer { kWindow, kPageContent, kPageSheet };

  Overlay(BrowserView* browser_view,
          std::unique_ptr<views::View> contents,
          const char* name,
          Layer layer = Layer::kWindow);
  ~Overlay() override;

  views::View* contents() { return contents_.view(); }
  bool IsVisible() const;
  // `bounds` in the BrowserView's coordinates.
  void Show(const gfx::Rect& bounds);
  void Hide();

  // views::WidgetObserver:
  void OnWidgetDestroying(views::Widget* widget) override;

 private:
  raw_ptr<BrowserView> browser_view_;
  std::unique_ptr<views::Widget> widget_;
  views::ViewTracker contents_;
  views::ViewTracker page_host_;
  base::ScopedObservation<views::Widget, views::WidgetObserver> observation_{
      this};
};

// Mori's popover surface in `bounds` (leave room around it for the
// shadow): #181818 (white in light mode) with a 1px edge, 12px corners and
// a soft shadow.
void PaintPopover(const views::View* view,
                  gfx::Canvas* canvas,
                  const gfx::Rect& bounds);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_OVERLAY_H_
