// The single bridging header handed to swiftc (`bridge_header` in GN): the
// pure-ObjC surface Mori's Swift sources compile against — identical to the
// Xcode app's Mori-Bridging-Header.h.
#import "MoriBrowserView.h"
#import "MoriPrivacy.h"

// Chrome-layer extensions (real Chromium extension service; chrome fork only).
// Every method must be called on the main (browser UI) thread.
#import <Cocoa/Cocoa.h>

NS_ASSUME_NONNULL_BEGIN

/// Swift-facing surface over Chrome's real extension system. Mori renders the
/// UI (toolbar buttons, menu, settings rows); Chrome owns everything else —
/// install state, content scripts, service workers, chrome.* APIs, popups,
/// side panels, badges and keyboard commands.
///
/// Listing dictionaries contain:
///   id, name, shortName, description, version (NSString)
///   enabled, pinned, mayDisable, hasOptionsPage, hasPopup (NSNumber bool)
///   badgeText, actionTitle (NSString; active-tab values, may be empty)
///   badgeBackgroundColor, badgeTextColor (NSArray<NSNumber*> RGBA 0-255)
///   icon (NSImage; absent until its async load completes)
///   homepageURL, webStoreURL, installType (NSString)
/// Command dictionaries contain:
///   extensionId, extensionName, commandName, description (NSString)
///   shortcut (NSString, "Ctrl+Shift+K" portable form; Ctrl ⇒ ⌘ on macOS)
///   isAction (NSNumber bool; true for _execute_action and MV2 equivalents)
///
/// Posts NSNotification "MoriChromeExtensionsChanged" whenever installed
/// extensions, action state (badge/icon/title), or pin state change;
/// "MoriChromeExtensionSidePanelChanged" when a side panel opens or closes;
/// and "MoriChromeExtensionInstallFinished" (userInfo {ok, error}) when a CRX
/// install completes.
@interface MoriChromeExtensions : NSObject

/// All installed (enabled and disabled) extensions the user can manage,
/// sorted by name. Apps, themes and component extensions are excluded.
+ (NSArray<NSDictionary*>*)installedExtensions NS_SWIFT_NAME(installedExtensions());

/// Run the extension's toolbar action exactly like clicking it in Chrome:
/// grants activeTab, runs blocked actions, fires action.onClicked, opens the
/// popup, or toggles the extension side panel — whichever the extension and
/// page state call for. The popup is anchored to `anchor` (screen
/// coordinates; pass NSZeroRect for the window's top-right).
+ (void)runActionWithId:(NSString*)extensionId
             anchorRect:(NSRect)anchor NS_SWIFT_NAME(runAction(id:anchor:));

/// Enable or disable an extension (chrome://extensions toggle equivalent).
+ (void)setExtensionWithId:(NSString*)extensionId
                   enabled:(BOOL)enabled NS_SWIFT_NAME(setExtension(id:enabled:));

/// Uninstall as a user action. The caller is responsible for confirmation UI.
/// Returns NO if the extension cannot be uninstalled (e.g. policy-installed).
+ (BOOL)uninstallExtensionWithId:(NSString*)extensionId NS_SWIFT_NAME(uninstall(id:));

/// Pin/unpin the extension's action in the toolbar (persisted in profile
/// prefs by Chrome's ToolbarActionsModel).
+ (void)setExtensionWithId:(NSString*)extensionId
                    pinned:(BOOL)pinned NS_SWIFT_NAME(setExtension(id:pinned:));

/// Open the extension's options page the way Chrome would (embedded in
/// chrome://extensions or as a tab, per the manifest).
+ (BOOL)openOptionsPageForId:(NSString*)extensionId NS_SWIFT_NAME(openOptionsPage(id:));

/// Open chrome://extensions in a new tab.
+ (void)openExtensionsPage NS_SWIFT_NAME(openExtensionsPage());
/// Open chrome://extensions focused on one extension's details.
+ (void)openExtensionsPageForId:(NSString*)extensionId NS_SWIFT_NAME(openExtensionsPage(id:));

/// Install a packed .crx through Chrome's CrxInstaller. Asynchronous; posts
/// MoriChromeExtensionInstallFinished and MoriChromeExtensionsChanged when
/// done. Returns NO only if the extension service isn't ready.
+ (BOOL)installCRXAtPath:(NSString*)path
              expectedId:(nullable NSString*)extensionId
    NS_SWIFT_NAME(installCRX(atPath:expectedId:));

/// Load an unpacked extension directory (chrome://extensions developer-mode
/// "Load unpacked" equivalent).
+ (BOOL)loadUnpackedExtensionAtPath:(NSString*)path
    NS_SWIFT_NAME(loadUnpacked(atPath:));

/// Keyboard commands of all enabled extensions (named commands plus the
/// action command). See the dictionary shape above.
+ (NSArray<NSDictionary*>*)commands NS_SWIFT_NAME(commands());

/// Fire a named command exactly like Chrome's keybinding registry: grants
/// activeTab and dispatches commands.onCommand to the extension. Action
/// commands (`isAction`) should instead be routed to runActionWithId.
+ (void)dispatchCommand:(NSString*)commandName
         forExtensionId:(NSString*)extensionId
    NS_SWIFT_NAME(dispatchCommand(_:extensionId:));

/// The native view of the currently open extension side panel, if any, with
/// its owning extension's id and name. Hosted by Mori's side panel chrome.
+ (nullable NSView*)sidePanelView;
+ (nullable NSString*)sidePanelExtensionId;
+ (nullable NSString*)sidePanelTitle;
/// Toggle the side panel for an extension that has one for the active tab.
/// Returns NO if the extension has no side panel for this tab.
+ (BOOL)toggleSidePanelForId:(NSString*)extensionId NS_SWIFT_NAME(toggleSidePanel(id:));
+ (void)closeSidePanel;

/// Close the action popup if one is showing.
+ (void)closePopup;

@end

NS_ASSUME_NONNULL_END
