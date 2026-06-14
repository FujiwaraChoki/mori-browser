// Seam between MoriBrowserWindow / chrome internals and the Mori bridge
// (mori_chrome_bridge.mm). ObjC++-includable; pure-C++ callers use only the
// non-ObjC declarations.

#ifndef CHROME_BROWSER_UI_MORI_MORI_CHROME_HOOKS_H_
#define CHROME_BROWSER_UI_MORI_MORI_CHROME_HOOKS_H_

#ifdef __OBJC__
#import <Cocoa/Cocoa.h>
#endif

class Browser;

namespace mori {

// First normal Browser becomes the Mori browser; the SwiftUI chrome attaches
// to it. Called from MoriBrowserWindow's ctor/dtor and Show().
void OnBrowserWindowCreated(Browser* browser);
void OnBrowserWindowDestroyed(Browser* browser);
void EnsureMoriUIStarted(Browser* browser);

// The Browser instance the Mori UI drives (null before startup).
Browser* MoriBrowser();

#ifdef __OBJC__
// The shared Mori main window (used for GetNativeWindow / dialog parenting).
NSWindow* MoriMainWindow();
#endif

}  // namespace mori

#endif  // CHROME_BROWSER_UI_MORI_MORI_CHROME_HOOKS_H_
