#import "MoriApplication.h"

// Generated from the Swift @objc interface (SWIFT_OBJC_INTERFACE_HEADER_NAME).
#import "Mori-Swift.h"

@implementation MoriApplication {
  BOOL _handlingSendEvent;
}

- (BOOL)isHandlingSendEvent {
  return _handlingSendEvent;
}

- (void)setHandlingSendEvent:(BOOL)handlingSendEvent {
  _handlingSendEvent = handlingSendEvent;
}

- (void)sendEvent:(NSEvent*)event {
  if (event.type == NSEventTypeKeyDown) {
    BOOL handled = [MoriRoot handleShortcutEvent:event];
    if (handled) return;
  } else if (event.type == NSEventTypeKeyUp) {
    [MoriRoot releaseShortcutEvent:event];
  }

  CefScopedSendingEvent sendingEventScoper;
  [super sendEvent:event];
}

// A browser is "active" for the purposes of the standard Cmd-key shortcuts.
// Terminate cleanly when the user chooses Quit.
- (void)terminate:(id)sender {
  [super terminate:sender];
}

@end
