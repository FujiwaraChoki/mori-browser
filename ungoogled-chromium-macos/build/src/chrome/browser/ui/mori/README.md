# Mori Overlay

This directory contains Mori's first-party browser chrome. It is compiled into
Chromium's `chrome` target through `chrome/browser/ui/BUILD.gn`.

## Boundaries

- Swift owns Mori UI, app state, settings, shortcuts, and product behavior.
- Objective-C++ owns direct interaction with Chromium browser objects.
- Chromium owns tabs, navigation, renderers, downloads, extensions,
  permissions, and web text input.

Keep the bridge thin. Put browser-object code in Objective-C++ and product/UI
state in Swift.

## File Groups

- Root and shell: `MoriRoot.swift`, `RootView.swift`, `Sidebar.swift`,
  `Toolbar.swift`, `TabRow.swift`, `WebContainerView.swift`.
- State: `BrowserStore.swift`, `BrowserStore+DragDrop.swift`,
  `BrowserTab.swift`, `BrowserContext.swift`, `Contexts.swift`,
  `TabFolder.swift`, `BrowserSettings.swift`.
- Stores: `ArchiveStore.swift`, `BookmarkStore.swift`, `BoostStore.swift`,
  `DownloadStore.swift`, `ExtensionStore.swift`, `HistoryStore.swift`,
  `RouteStore.swift`.
- Features: `AIPanel.swift`, `AirTrafficControl.swift`, `Boosts.swift`,
  `BrowserAutomation.swift`, `DownloadsPanel.swift`, `ExtensionsMenu.swift`,
  `FindBar.swift`, `LibraryPanel.swift`, `Peek.swift`, `Reader.swift`,
  `Screenshot.swift`, `SettingsView.swift`, `TabMaintenance.swift`.
- Theming and UI helpers: `Components.swift`, `FaviconKit.swift`,
  `FontRegistry.swift`, `Glass.swift`, `GradientEngine.swift`,
  `GradientTheme.swift`, `Icon.swift`, `MorphingFolderIcon.swift`,
  `OKLCH.swift`, `Theme.swift`, `ThemePicker.swift`, `ThemePresets.swift`,
  `ToastCenter.swift`, `ToastOverlay.swift`.
- Media: `MediaAgentScripts.swift`, `MediaController.swift`,
  `MediaPlayer.swift`, `MediaPolling.swift`, `PiPWindowStyler.swift`.
- Security and auth: `PasskeyAuthenticator.swift`, `PasskeySupport.swift`,
  `MoriPrivacy.h`.
- Bridge: `mori_bridge.h`, `mori_chrome_bridge.mm`,
  `mori_browser_window.mm`, `mori_browser_window.h`,
  `mori_chrome_extensions.mm`, `mori_chrome_hooks.h`,
  `mori_permission_prompt.mm`, `mori_permission_prompt.h`,
  `MoriBrowserView.h`.

## Adding Files

When adding, moving, or deleting Swift files, update the `mori_ui_swift`
sources list in:

```text
chrome/browser/ui/BUILD.gn
```

For larger architectural context, see the repository-level
`docs/ARCHITECTURE.md` and `docs/SOURCE_LAYOUT.md`.
