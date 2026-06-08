# Working in this repo

Mori is a native macOS browser: SwiftUI/AppKit chrome over a real Chromium
engine via CEF. Read `README.md` for the architecture, build, and layout. This
file captures the non-obvious things an agent needs to be productive here —
especially the Chrome-extension subsystem, which has sharp edges.

## Build & run

```bash
./run.sh                                   # generate project, build Debug, launch
./script/build_and_run.sh build            # build only (no launch)
./script/build_and_run.sh --verify-extension-smoke   # extension API smoke gate
```

Requirements: macOS 26+, Xcode 26+, `xcodegen` + `cmake` (Homebrew). The first
build compiles `libcef_dll_wrapper` from the bundled CEF distribution.

## Extension subsystem map

Mori implements the Chrome extension surface itself (no `--load-extension`); the
moving parts:

- **`Sources/App/BrowserClient.mm`** — the big one. Contains `ExtensionRuntimeShim`
  (the injected `chrome.*` / `browser.*` JS), content-script injection
  (`InjectExtensionContentScripts`, fired from `OnLoadStart`/`OnLoadEnd` for
  document_start/end/idle), `chrome.scripting.executeScript` emulation, the
  runtime-messaging bridge (`runtime.sendMessage`/`tabs.sendMessage`/ports), and
  the `OnConsoleMessage` IPC channel (`console.info('__MORI_…__'+json)`).
- **`Sources/App/CefAppImpl.mm`** — the `mori-extension://` scheme handler
  (`MoriExtensionSchemeHandlerFactory`): serves extension files, background page,
  and gates `web_accessible_resources`.
- **`Sources/Bridge/MoriBrowserView.mm`** — per-tab `NSView` over one CEF
  browser; `broadcastExtensionJavaScript` / `dispatchExtensionBridgeResponse` /
  `executeExtensionJavaScript` deliver bridge traffic into page frames.
- **`Sources/UI/Models/BrowserStore.swift`** — `handleExtensionScripting`,
  tab/extension-tab mapping, the Swift side of the bridge.

There is **no isolated world**: CEF's `frame->ExecuteJavaScript` only runs in the
page's main world, so content scripts AND `executeScript`-injected "isolated"
scripts share the page's single `globalThis.chrome`. Much of the subtlety below
comes from that.

## Extension gotchas (hard-won — change with care)

These were each a multi-hour debugging session. Test against **Proton Pass** (a
demanding MV3 extension: anti-tamper proxy, ML field detection, iframe-based
autofill UI, port-multiplexed store sync) before declaring extension work done.

1. **Document focus must be forwarded to CEF.** `MoriBrowserView` calls
   `host->SetFocus()` from `_syncBrowserFocus` on window-key changes / visibility
   / attach. Without it `document.hasFocus()` stays `false` on a freshly-loaded
   or refocused page, and extensions that gate UI on focus (Proton Pass hides its
   in-field autofill icon) silently do nothing. Don't remove the
   `NSWindowDidBecomeKey/ResignKey` observers.

2. **Keep the `runtime.messageNoResponse` grace SHORT (~1s).** It sits on hot
   init paths — Proton's orchestrator does `await sendMessage(UNLOAD_CONTENT_SCRIPT)`
   before loading its autofill client, and a long grace (it was briefly 30s)
   stalls the autofill icon by that much on every page. A real async reply still
   wins because its `messageResponse` settles + clears the request the instant it
   lands. See the comment in the `runtime.messageNoResponse` handler.

3. **`web_accessible_resources` initiator must resolve a real origin.** A
   freshly-created subframe reports `frame->GetURL()` == `about:blank`, which
   matches no `https://*/*` pattern → the resource 403s. `InitiatorURLForRequest`
   walks up to the parent frame's origin (about:blank inherits its embedder's
   origin) before falling back to the referrer. This is what lets Proton's
   `dropdown.html` autofill iframe load when you click the field icon.

4. **`globalThis.chrome` is hardened against replacement.** The shim installs
   `chrome`/`browser` as accessor properties whose setter rejects any object
   lacking `runtime.id`, plus a restore guard. Proton replaces `globalThis.chrome`
   with an anti-tamper `Proxy`; this keeps the real shim in place. The benign
   `[Extension::Error] extension API is protected` log is just that rejected proxy
   being probed — not a failure.

5. **chrome.storage values persist as JSON-encoded `NSData`, not plist dicts.**
   Extension storage can contain `null` (NSNull), which `NSUserDefaults` rejects,
   throwing away the whole write. (This repeatedly signed Proton out.)

6. **Internal `runtime.sendMessage` must reach extension contexts only, never
   content scripts.** Only `tabs.sendMessage` targets a tab's content scripts.

## Debugging extensions

Mori is windowed CEF, so it honors `--remote-debugging-port`. Drive it via CDP:

```bash
# Launch the BUILT bundle frontmost (open => real window-key, which #1 needs);
# launching the raw binary via nohup leaves it non-key so hasFocus stays false.
open "$(pwd)/build/dd/Build/Products/Debug/Mori.app" \
  --args --remote-debugging-port=9222 --remote-allow-origins='*'
curl -s http://127.0.0.1:9222/json        # list page/iframe/worker targets
```

Then connect a CDP websocket to a target and `Runtime.evaluate` / collect
`Runtime.consoleAPICalled` (the `__MORI_EXTENSION__…` lines are the bridge IPC).
Pitfalls:

- `/Applications/Mori.app` is usually a stale older build — `pkill -9 -f
  "Mori.app/Contents/MacOS/Mori"` first; it conflicts on the shared user-data-dir
  singleton and `osascript "tell app Mori to activate"` relaunches the registered
  bundle **without** your CDP flags.
- Proton lazy-loads its autofill client only for visible/focused tabs, and tears
  its UI down on blur — a headless session that can't hold window focus makes
  results look flaky. Use `Emulation.setFocusEmulationEnabled` to simulate a
  focused window when you only need to measure the extension pipeline.
- Installed extensions live under
  `~/Library/Application Support/MoriBrowser/Extensions/<UUID>/`.

## Conventions

- Swift talks only to the pure-ObjC `MoriBrowserView` header; all C++/CEF stays
  in `.mm`. Don't leak CEF types into Swift-visible headers.
- The injected `chrome.*` shim is one giant `NSString` of JS — match its existing
  `var`/`function` style and `||`-guard every definition so re-injection is safe.
- Commit only when asked. If you patch an installed extension's files for
  debugging, revert them.
