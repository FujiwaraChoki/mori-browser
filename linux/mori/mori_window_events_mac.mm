#include "chrome/browser/ui/views/mori/mori_window_events.h"

#import <Cocoa/Cocoa.h>

#include "ui/gfx/native_ui_types.h"
#include "ui/views/widget/widget.h"

namespace mori {

void SetWindowButtonsVisible(views::Widget* widget, bool visible) {
  if (!widget || widget->IsFullscreen()) {
    return;
  }
  NSWindow* window = widget->GetNativeWindow().GetNativeNSWindow();
  if (!window) {
    return;
  }
  for (NSWindowButton type :
       {NSWindowCloseButton, NSWindowMiniaturizeButton, NSWindowZoomButton}) {
    [[window standardWindowButton:type] setHidden:!visible];
  }
}

}  // namespace mori
