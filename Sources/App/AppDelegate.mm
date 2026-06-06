#import "AppDelegate.h"

#include "include/cef_app.h"
#include "include/wrapper/cef_helpers.h"

// Generated from the Swift @objc interface (SWIFT_OBJC_INTERFACE_HEADER_NAME).
#import "Mori-Swift.h"

@interface AppDelegate ()
@property(nonatomic, strong) NSWindow* mainWindow;
@end

@implementation AppDelegate

- (void)applicationDidFinishLaunching:(NSNotification*)notification {
  [self buildMainMenu];

  NSRect frame = NSMakeRect(0, 0, 1280, 820);
  NSWindowStyleMask style = NSWindowStyleMaskTitled |
                            NSWindowStyleMaskClosable |
                            NSWindowStyleMaskMiniaturizable |
                            NSWindowStyleMaskResizable |
                            NSWindowStyleMaskFullSizeContentView;

  self.mainWindow = [[NSWindow alloc] initWithContentRect:frame
                                                styleMask:style
                                                  backing:NSBackingStoreBuffered
                                                    defer:NO];
  self.mainWindow.title = @"Mori";
  self.mainWindow.titlebarAppearsTransparent = YES;
  self.mainWindow.titleVisibility = NSWindowTitleHidden;
  self.mainWindow.movableByWindowBackground = NO;
  // Let behind-window glass (sidebar/panels) sample the desktop for real
  // translucency / Liquid Glass vibrancy.
  self.mainWindow.opaque = NO;
  self.mainWindow.backgroundColor = NSColor.clearColor;
  [self.mainWindow setFrameAutosaveName:@"MoriMainWindow"];
  self.mainWindow.minSize = NSMakeSize(720, 480);

  // Build the SwiftUI root (sidebar + omnibox + web content) and host it.
  NSViewController* root = [MoriRoot makeRootViewController];
  self.mainWindow.contentViewController = root;

  [self.mainWindow center];
  [self.mainWindow makeKeyAndOrderFront:nil];
  [NSApp activateIgnoringOtherApps:YES];
}

// Build a standard macOS menu bar. The Edit menu's Cut/Copy/Paste/Select-All
// route to the first responder, which is how clipboard works inside CEF web
// text fields.
- (void)buildMainMenu {
  NSMenu* mainMenu = [[NSMenu alloc] init];

  // App menu.
  NSMenuItem* appItem = [[NSMenuItem alloc] init];
  [mainMenu addItem:appItem];
  NSMenu* appMenu = [[NSMenu alloc] init];
  [appMenu addItemWithTitle:@"About Mori"
                     action:@selector(orderFrontStandardAboutPanel:)
              keyEquivalent:@""];
  [appMenu addItem:[NSMenuItem separatorItem]];
  [appMenu addItemWithTitle:@"Settings…"
                     action:@selector(openSettings:)
              keyEquivalent:@","];
  [appMenu addItem:[NSMenuItem separatorItem]];
  [appMenu addItemWithTitle:@"Hide Mori"
                     action:@selector(hide:)
              keyEquivalent:@"h"];
  [appMenu addItemWithTitle:@"Quit Mori"
                     action:@selector(terminate:)
              keyEquivalent:@"q"];
  appItem.submenu = appMenu;

  // File menu.
  NSMenuItem* fileItem = [[NSMenuItem alloc] init];
  [mainMenu addItem:fileItem];
  NSMenu* fileMenu = [[NSMenu alloc] initWithTitle:@"File"];
  [fileMenu addItemWithTitle:@"New Tab"
                      action:@selector(newTab:)
               keyEquivalent:@"t"];
  // Capital "T" → Cmd-Shift-T (AppKit folds the Shift in automatically).
  [fileMenu addItemWithTitle:@"Reopen Closed Tab"
                      action:@selector(reopenTab:)
               keyEquivalent:@"T"];
  [fileMenu addItem:[NSMenuItem separatorItem]];
  [fileMenu addItemWithTitle:@"Open Location…"
                      action:@selector(openLocation:)
               keyEquivalent:@"l"];
  [fileMenu addItem:[NSMenuItem separatorItem]];
  [fileMenu addItemWithTitle:@"Close Tab"
                      action:@selector(closeTab:)
               keyEquivalent:@"w"];
  [fileMenu addItem:[NSMenuItem separatorItem]];
  [fileMenu addItemWithTitle:@"Print…"
                      action:@selector(printPage:)
               keyEquivalent:@"p"];
  fileItem.submenu = fileMenu;

  // Edit menu — standard responder actions (clipboard for web fields).
  NSMenuItem* editItem = [[NSMenuItem alloc] init];
  [mainMenu addItem:editItem];
  NSMenu* editMenu = [[NSMenu alloc] initWithTitle:@"Edit"];
  [editMenu addItemWithTitle:@"Undo" action:@selector(undo:) keyEquivalent:@"z"];
  [editMenu addItemWithTitle:@"Redo" action:@selector(redo:) keyEquivalent:@"Z"];
  [editMenu addItem:[NSMenuItem separatorItem]];
  [editMenu addItemWithTitle:@"Cut" action:@selector(cut:) keyEquivalent:@"x"];
  [editMenu addItemWithTitle:@"Copy" action:@selector(copy:) keyEquivalent:@"c"];
  [editMenu addItemWithTitle:@"Paste" action:@selector(paste:) keyEquivalent:@"v"];
  [editMenu addItemWithTitle:@"Select All"
                      action:@selector(selectAll:)
               keyEquivalent:@"a"];
  [editMenu addItem:[NSMenuItem separatorItem]];
  [editMenu addItemWithTitle:@"Find…"
                      action:@selector(findInPage:)
               keyEquivalent:@"f"];
  [editMenu addItemWithTitle:@"Find Next"
                      action:@selector(findNext:)
               keyEquivalent:@"g"];
  // Capital "G" → Cmd-Shift-G (find previous).
  [editMenu addItemWithTitle:@"Find Previous"
                      action:@selector(findPrevious:)
               keyEquivalent:@"G"];
  editItem.submenu = editMenu;

  // View menu.
  NSMenuItem* viewItem = [[NSMenuItem alloc] init];
  [mainMenu addItem:viewItem];
  NSMenu* viewMenu = [[NSMenu alloc] initWithTitle:@"View"];
  [viewMenu addItemWithTitle:@"Reload"
                      action:@selector(reloadPage:)
               keyEquivalent:@"r"];
  // Capital "R" → Cmd-Shift-R (reload, bypassing the cache).
  [viewMenu addItemWithTitle:@"Force Reload"
                      action:@selector(forceReload:)
               keyEquivalent:@"R"];
  [viewMenu addItemWithTitle:@"Stop"
                      action:@selector(stopPage:)
               keyEquivalent:@"."];
  [viewMenu addItem:[NSMenuItem separatorItem]];
  // Bound to "=" (not "+") so the bare Cmd-= that users actually press fires
  // it, without requiring Shift. Displays as ⌘=.
  [viewMenu addItemWithTitle:@"Zoom In"
                      action:@selector(zoomIn:)
               keyEquivalent:@"="];
  [viewMenu addItemWithTitle:@"Zoom Out"
                      action:@selector(zoomOut:)
               keyEquivalent:@"-"];
  [viewMenu addItemWithTitle:@"Actual Size"
                      action:@selector(resetZoom:)
               keyEquivalent:@"0"];
  [viewMenu addItem:[NSMenuItem separatorItem]];
  // Cmd-S toggles the tab sidebar (the default modifier mask is Command).
  // NB: the action is intentionally NOT named toggleSidebar: — that selector
  // collides with AppKit's built-in NSSplitViewController.toggleSidebar:, which
  // a responder in the SwiftUI/CEF chain claims and validates as disabled,
  // greying out the item. A unique name routes it straight to us.
  NSMenuItem* sidebarItem =
      [viewMenu addItemWithTitle:@"Toggle Sidebar"
                          action:@selector(moriToggleSidebar:)
                   keyEquivalent:@"s"];
  [viewMenu addItemWithTitle:@"Toggle AI Assistant"
                      action:@selector(toggleAI:)
               keyEquivalent:@"k"];
  [viewMenu addItem:[NSMenuItem separatorItem]];
  // Cmd-Opt-I → Developer Tools, matching Chrome/Safari's Inspect shortcut.
  NSMenuItem* devItem =
      [[NSMenuItem alloc] initWithTitle:@"Developer Tools"
                                 action:@selector(toggleDevTools:)
                          keyEquivalent:@"i"];
  devItem.keyEquivalentModifierMask =
      NSEventModifierFlagCommand | NSEventModifierFlagOption;
  [viewMenu addItem:devItem];
  viewItem.submenu = viewMenu;

  // History menu.
  NSMenuItem* historyItem = [[NSMenuItem alloc] init];
  [mainMenu addItem:historyItem];
  NSMenu* historyMenu = [[NSMenu alloc] initWithTitle:@"History"];
  [historyMenu addItemWithTitle:@"Back"
                         action:@selector(goBack:)
                  keyEquivalent:@"["];
  [historyMenu addItemWithTitle:@"Forward"
                         action:@selector(goForward:)
                  keyEquivalent:@"]"];
  [historyMenu addItem:[NSMenuItem separatorItem]];
  // Capital "H" → Cmd-Shift-H (avoids clobbering the app-level Hide on Cmd-H).
  [historyMenu addItemWithTitle:@"Home"
                         action:@selector(goHome:)
                  keyEquivalent:@"H"];
  historyItem.submenu = historyMenu;

  // Window menu.
  NSMenuItem* windowItem = [[NSMenuItem alloc] init];
  [mainMenu addItem:windowItem];
  NSMenu* windowMenu = [[NSMenu alloc] initWithTitle:@"Window"];
  [windowMenu addItemWithTitle:@"Minimize"
                        action:@selector(performMiniaturize:)
                 keyEquivalent:@"m"];
  [windowMenu addItemWithTitle:@"Zoom"
                        action:@selector(performZoom:)
                 keyEquivalent:@""];
  NSMenuItem* fullscreenItem =
      [[NSMenuItem alloc] initWithTitle:@"Enter Full Screen"
                                 action:@selector(toggleFullScreen:)
                          keyEquivalent:@"f"];
  fullscreenItem.keyEquivalentModifierMask =
      NSEventModifierFlagCommand | NSEventModifierFlagControl;
  [windowMenu addItem:fullscreenItem];
  [windowMenu addItem:[NSMenuItem separatorItem]];
  NSMenuItem* previousTabItem =
      [[NSMenuItem alloc] initWithTitle:@"Show Previous Tab"
                                 action:@selector(selectPreviousTab:)
                          keyEquivalent:@"["];
  previousTabItem.keyEquivalentModifierMask =
      NSEventModifierFlagCommand | NSEventModifierFlagShift;
  [windowMenu addItem:previousTabItem];
  NSMenuItem* nextTabItem =
      [[NSMenuItem alloc] initWithTitle:@"Show Next Tab"
                                 action:@selector(selectNextTab:)
                          keyEquivalent:@"]"];
  nextTabItem.keyEquivalentModifierMask =
      NSEventModifierFlagCommand | NSEventModifierFlagShift;
  [windowMenu addItem:nextTabItem];
  [windowMenu addItem:[NSMenuItem separatorItem]];
  [windowMenu addItemWithTitle:@"Bring All to Front"
                        action:@selector(arrangeInFront:)
                 keyEquivalent:@""];
  windowItem.submenu = windowMenu;
  NSApp.windowsMenu = windowMenu;

  [self targetMoriMenuItemsInMenu:mainMenu];
  NSApp.mainMenu = mainMenu;
}

- (void)targetMoriMenuItemsInMenu:(NSMenu*)menu {
  for (NSMenuItem* item in menu.itemArray) {
    if (item.submenu) {
      [self targetMoriMenuItemsInMenu:item.submenu];
    }
    SEL action = item.action;
    if (action && [self respondsToSelector:action]) {
      item.target = self;
    }
  }
}

// Menu actions that drive the SwiftUI store.
- (void)newTab:(id)sender { [MoriRoot newTab]; }
- (void)reopenTab:(id)sender { [MoriRoot reopenClosedTab]; }
- (void)openLocation:(id)sender { [MoriRoot focusOmnibox]; }
- (void)closeTab:(id)sender { [MoriRoot closeCurrentTab]; }
- (void)reloadPage:(id)sender { [MoriRoot reload]; }
- (void)forceReload:(id)sender { [MoriRoot forceReload]; }
- (void)stopPage:(id)sender { [MoriRoot stop]; }
- (void)goBack:(id)sender { [MoriRoot goBack]; }
- (void)goForward:(id)sender { [MoriRoot goForward]; }
- (void)goHome:(id)sender { [MoriRoot goHome]; }
- (void)moriToggleSidebar:(id)sender { [MoriRoot toggleSidebar]; }
- (void)toggleAI:(id)sender { [MoriRoot toggleAIPanel]; }
- (void)openSettings:(id)sender { [MoriRoot openSettings]; }
- (void)zoomIn:(id)sender { [MoriRoot zoomIn]; }
- (void)zoomOut:(id)sender { [MoriRoot zoomOut]; }
- (void)resetZoom:(id)sender { [MoriRoot resetZoom]; }
- (void)findInPage:(id)sender { [MoriRoot toggleFindBar]; }
- (void)findNext:(id)sender { [MoriRoot findNext]; }
- (void)findPrevious:(id)sender { [MoriRoot findPrevious]; }
- (void)printPage:(id)sender { [MoriRoot printPage]; }
- (void)toggleDevTools:(id)sender { [MoriRoot toggleDevTools]; }
- (void)selectNextTab:(id)sender { [MoriRoot selectNextTab]; }
- (void)selectPreviousTab:(id)sender { [MoriRoot selectPreviousTab]; }

- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication*)sender {
  return YES;
}

- (NSApplicationTerminateReply)applicationShouldTerminate:
    (NSApplication*)sender {
  // Let SwiftUI tear down browsers, then quit the CEF message loop.
  [MoriRoot prepareForTermination];
  CefQuitMessageLoop();
  return NSTerminateNow;
}

@end
