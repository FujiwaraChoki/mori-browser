# Mori on Linux (Chromium build)

Mori's Linux browser built into ungoogled-chromium. See
`docs/DECISIONS-linux-chromium.md` for why and how.

- `mori/`: Mori's C++ (Views) sources. `build.sh` links this folder into the
  Chromium tree as `chrome/browser/ui/views/mori`.
- `patches/`: changes to Chromium files, applied by `build.sh`: the hooks that
  call into `mori/`, and build fixes for ungoogled-chromium.
- `fetch.sh`, `build.sh`: download and build. They expect a clone of
  [ungoogled-chromium-portablelinux](https://github.com/ungoogled-software/ungoogled-chromium-portablelinux)
  (with submodules) in `../ungoogled-chromium-linux` (git-ignored).
- `harness/`: headless sway for driving the browser in tests without touching
  the desktop. State goes to `../ungoogled-chromium-linux/harness-run`.

```sh
linux/fetch.sh                      # once per Chromium version
JOBS=12 linux/build.sh              # build out/Default/chrome
```

## Default browser

Open **Settings → General → Default browser → Make Default** to use Mori for
links from other apps. Confirm the change in the system dialog if prompted.
The status refreshes while General settings are open, including after a macOS
confirmation. If the desktop cannot report the current default, the action
remains available; unsuccessful or cancelled changes can be retried.

## Little Mori

External links open in a compact window with a single header: back, forward,
reload, the current domain, and **Open in Mori**. Click the domain or press
Cmd/Ctrl+L to edit the address in Mori's command bar. Searches replace the
current Little Mori page.

**Open in Mori** (Cmd/Ctrl+O) moves the live tab into the last active Mori
window, preserving page state and navigation history. The arrow beside it
lists other windows in the same profile and offers **New Mori Window**.
Empty header space drags the window. On macOS, the traffic lights share the
header instead of occupying a separate title bar.

## Folders and imports

Use **New Folder** in the sidebar menu or command bar to create a folder
with a blank tab. A tab's menu also offers **New Folder with Tab** and
**Add to …**. Folder headers offer rename, collapse, icons and deletion.
Folder tabs stay out of auto-archive and Archive Unpinned Tabs.

Importing Arc or Zen tabs keeps folder names and membership, including
separate folders with the same name. Folders persist through session restore.
Arc and Zen imports preserve the original parent/child hierarchy, including
parents with no direct tabs. Use **New Subfolder** or **Move Folder Into** in
a folder's menu to organize the tree. Collapsing a parent hides its descendants.
All folders and their tabs appear above **Add Tab**; ordinary tabs appear below
it. Empty leaf folders receive a blank tab so they persist in the session.

The import parser regressions use the Chromium build's dependencies:

```sh
cd ungoogled-chromium-linux/build/src
out/Default/gn gen out/Default --root-target=//chrome/browser/ui/views/mori/tests
ninja -C out/Default mori_import_tests
out/Default/mori_import_tests
```

On macOS, use `ungoogled-chromium-mac/build/src` instead.

## Experimental automatic 1Password passkeys

The full Chromium assistant can confirm the official stable 1Password
extension's displayed **Use passkey** prompt. This is opt-in per HTTPS origin
and account. Launch a fresh browser process with, for example:

```sh
out/Default/chrome \
  --mori-auto-passkey-accounts='{"https://example.com":"alice@example.com"}'
```

Use the exact account text shown inside the prompt. If you don't know the
label, `"@default"` authorizes the single displayed default passkey instead:

```sh
out/Default/chrome \
  --mori-auto-passkey-accounts='{"https://my.1password.com":"@default","https://start.1password.com":"@default"}'
```

On macOS, pass the same flag via `open -n /path/to/Mori.app --args ...`.
Quit the existing browser first: Chromium's process singleton does not apply
startup switches to an already running process. Start a new assistant chat so
its tool definitions include `mori_use_1password_passkey`.

For the staged macOS vault prototype, run
`mac/package.sh dist/passkey-prototype/Mori.app`, quit Mori, and run
`mac/run-passkey-prototype.sh`. This launcher uses the existing Mori profile,
authorizes the two US vault origins above with `@default`, and refuses to run
while another Mori instance is open. Other account regions or custom account
domains need their own exact origin in the startup mapping.

Ask the assistant to open the site's passkey sign-in flow and use the tool
with `origin` (the exact HTTPS origin) and optional `tabId`. The tool has standing
authorization from the startup mapping, so it adds no confirmation card.
Ordinary browser actions retain their existing approval behavior. The helper
only focuses the matching 1Password control and sends native Enter; it never
reads passwords, exports passkeys, or exposes the extension's general UI to
the model. It cancels if the turn stops, the active tab changes, or the page
navigates before activation. It also checks the prompt's own tab ID.

`confirmation_sent` means input was sent, **not** that the site authenticated.
The assistant must verify the site's resulting page. A locked vault or fresh
biometric/PIN verification still needs the user. Unsupported prompt layouts,
multiple matching controls, and unavailable extensions fail without clicking.
This prototype targets the English universal-sign-in prompt in official
extension 8.12.42.36, uses the currently displayed account only, and does not
expand account lists or handle other extension versions/layouts automatically.

Regression checks (use the macOS Chromium source directory on macOS):

```sh
cd ungoogled-chromium-linux/build/src
out/Default/gn gen out/Default --root-target=//chrome/browser/ui/views/mori/tests
ninja -C out/Default mori_passkey_tests
out/Default/mori_passkey_tests
```

From the repository root, `python3 linux/tests/mori_passkey_prompt_test.py`
tests the actual production prompt probe with fake accounts through
Playwright CLI. It covers native Enter, exact/default account selection,
ambiguity, hidden/disabled controls, locked prompts, and unsupported labels.
Use `--cli 'node /path/to/playwright-cli.js'` and `--config /path/to/config.json`
if Playwright CLI isn't on PATH or to choose a specific browser executable.
