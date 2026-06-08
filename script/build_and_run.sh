#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-run}"
APP_NAME="Mori"
PROJECT="Mori.xcodeproj"
SCHEME="Mori"
CONFIGURATION="Debug"
DERIVED_DATA_PATH="build/dd"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_BUNDLE="$ROOT_DIR/$DERIVED_DATA_PATH/Build/Products/$CONFIGURATION/$APP_NAME.app"
EXTERNAL_CHROME_BEFORE=""
SMOKE_ENGINE_AUDIT_PATH=""
SMOKE_NATIVE_MESSAGING_HOSTS_DIR=""
SMOKE_CRX_EXTENSION_ID=""
SMOKE_CRX_PATH=""
SMOKE_CRX_URL=""
SMOKE_EXTERNAL_PAGE_URL=""
SMOKE_CORS_URL=""
cd "$ROOT_DIR"

external_chrome_pids() {
  /bin/ps ax -o pid=,args= \
    | /usr/bin/awk '/Google Chrome[.]app|\/Chrome[.]app/ && $0 !~ /Mori[.]app/ { print $1 }' \
    | /usr/bin/sort -n
}

record_external_chrome_state() {
  EXTERNAL_CHROME_BEFORE="$(external_chrome_pids)"
}

audit_no_external_chrome_spawned() {
  local after new_pids pid details

  after="$(external_chrome_pids)"
  new_pids=""
  while IFS= read -r pid; do
    [[ -z "$pid" ]] && continue
    if ! printf '%s\n' "$EXTERNAL_CHROME_BEFORE" | /usr/bin/grep -Fx "$pid" >/dev/null; then
      new_pids+="$pid"$'\n'
    fi
  done <<<"$after"

  if [[ -n "$new_pids" ]]; then
    details="$(/bin/ps -o pid=,args= -p "$(printf '%s\n' "$new_pids" | /usr/bin/paste -sd, -)" 2>/dev/null || true)"
    echo "verify failed: launching Mori spawned external Chrome processes:" >&2
    printf '%s\n' "$details" >&2
    exit 1
  fi
}

stop_app() {
  /usr/bin/osascript -e 'tell application "Mori" to quit' >/dev/null 2>&1 || true
  sleep 0.5
  /usr/bin/pkill -x "$APP_NAME" >/dev/null 2>&1 || true
  /usr/bin/pkill -f "/Mori.app/Contents/MacOS/Mori" >/dev/null 2>&1 || true
  /usr/bin/pkill -f "/Mori.app/Contents/Frameworks/Mori Helper" >/dev/null 2>&1 || true
  sleep 0.5
}

ensure_cef_wrapper() {
  local wrapper="$ROOT_DIR/third_party/cef/build/libcef_dll_wrapper/libcef_dll_wrapper.a"
  if [[ -f "$wrapper" ]]; then
    return
  fi

  echo "Building libcef_dll_wrapper..."
  (
    cd "$ROOT_DIR/third_party/cef"
    /bin/mkdir -p build
    cd build
    cmake -G "Unix Makefiles" -DCMAKE_BUILD_TYPE=Release -DPROJECT_ARCH=arm64 .. >/dev/null
    make libcef_dll_wrapper -j"$(/usr/sbin/sysctl -n hw.ncpu)" >/dev/null
  )
}

generate_project() {
  xcodegen generate >/dev/null
}

build_app() {
  ensure_cef_wrapper
  generate_project
  /usr/bin/xcodebuild \
    -project "$PROJECT" \
    -scheme "$SCHEME" \
    -configuration "$CONFIGURATION" \
    -derivedDataPath "$DERIVED_DATA_PATH" \
    build
}

open_app() {
  /usr/bin/open -n "$APP_BUNDLE"
}

json_escape() {
  /usr/bin/sed 's/\\/\\\\/g; s/"/\\"/g' <<<"$1"
}

create_extension_smoke_fixture() {
  local run_id="$1"
  local smoke_id="mori-smoke-extension"
  local smoke_companion_id="mori-smoke-companion"
  local smoke_dir="$ROOT_DIR/build/extension-smoke/$smoke_id"
  local smoke_companion_dir="$ROOT_DIR/build/extension-smoke/$smoke_companion_id"
  local smoke_page="$ROOT_DIR/build/extension-smoke/smoke-page.html"
  local smoke_capture_page="$ROOT_DIR/build/extension-smoke/capture-page.html"
  local smoke_auth_page="$ROOT_DIR/build/extension-smoke/identity-auth.html"
  local native_hosts_dir="$ROOT_DIR/build/extension-smoke/native-hosts"
  local native_host_script="$ROOT_DIR/build/extension-smoke/native-host.py"
  local crx_fixture_dir="$ROOT_DIR/build/extension-smoke/crx-fixture"
  local escaped_dir escaped_companion_dir chrome_extension_origin

  /bin/mkdir -p "$smoke_dir"
  /bin/mkdir -p "$smoke_companion_dir"
  /bin/mkdir -p "$native_hosts_dir"
  /bin/mkdir -p "$crx_fixture_dir"
  cat >"$native_host_script" <<'PY'
#!/usr/bin/env python3
import json
import struct
import sys

def write_message(response):
    payload = json.dumps(response, separators=(",", ":")).encode("utf-8")
    sys.stdout.buffer.write(struct.pack("<I", len(payload)))
    sys.stdout.buffer.write(payload)
    sys.stdout.buffer.flush()

while True:
    raw_length = sys.stdin.buffer.read(4)
    if not raw_length:
        break
    if len(raw_length) != 4:
        sys.exit(1)
    length = struct.unpack("<I", raw_length)[0]
    message = json.loads(sys.stdin.buffer.read(length).decode("utf-8"))
    write_message({
        "ok": True,
        "echo": message,
        "origin": sys.argv[1] if len(sys.argv) > 1 else "",
        "host": "com.mori.smoke",
        "mode": message.get("mode", "single")
    })
PY
  /bin/chmod +x "$native_host_script"
  cat >"$native_hosts_dir/com.mori.smoke.json" <<JSON
{
  "name": "com.mori.smoke",
  "description": "Mori native messaging smoke host",
  "path": "$native_host_script",
  "type": "stdio",
  "allowed_origins": [
    "mori-extension://$smoke_id/"
  ]
}
JSON
  chrome_extension_origin="chrome-"
  chrome_extension_origin+="extension://$smoke_id/"
  cat >"$native_hosts_dir/com.mori.smoke_chrome.json" <<JSON
{
  "name": "com.mori.smoke_chrome",
  "description": "Mori Chrome-compatible native messaging smoke host",
  "path": "$native_host_script",
  "type": "stdio",
  "allowed_origins": [
    "$chrome_extension_origin"
  ]
}
JSON
  cat >"$smoke_page" <<HTML
<!doctype html>
<meta charset="utf-8">
<title>Mori Smoke Page</title>
<body data-mori-smoke-page="$run_id">
  <form id="mori-smoke-login">
    <label for="mori-smoke-input">Email</label>
    <input id="mori-smoke-input" name="email" type="email" autocomplete="username">
  </form>
  <iframe id="mori-smoke-child-frame" src="$SMOKE_FRAME_URL"></iframe>
</body>
HTML

  cat >"$smoke_capture_page" <<HTML
<!doctype html>
<meta charset="utf-8">
<title>Mori Capture Smoke</title>
<style>
html, body {
  margin: 0;
  width: 100%;
  height: 100%;
  background: rgb(42, 171, 91);
}
body::before {
  content: "capture-$run_id";
  display: block;
  padding: 32px;
  color: white;
  font: 32px system-ui;
}
</style>
HTML

  cat >"$smoke_auth_page" <<HTML
<!doctype html>
<meta charset="utf-8">
<title>Mori Identity Smoke</title>
<body>Redirecting identity smoke...</body>
<script>
setTimeout(() => {
  location.href = "https://$smoke_id.chromiumapp.org/oauth-smoke?code=$run_id";
}, 25);
</script>
HTML

  cat >"$crx_fixture_dir/manifest.json" <<JSON
{
  "manifest_version": 3,
  "name": "Mori CRX Download Smoke",
  "version": "1.0.0",
  "description": "Installed through Mori's CRX download pipeline for $run_id",
  "action": {
    "default_popup": "popup.html"
  }
}
JSON

  cat >"$crx_fixture_dir/popup.html" <<HTML
<!doctype html>
<meta charset="utf-8">
<title>Mori CRX Download Smoke</title>
<body>CRX install smoke for $run_id</body>
HTML

  python3 - "$crx_fixture_dir" "$SMOKE_CRX_PATH" "$SMOKE_CRX_EXTENSION_ID" <<'PY'
import io
import os
import struct
import sys
import zipfile

source, target, extension_id = sys.argv[1:4]

def varint(value):
    out = bytearray()
    while value >= 0x80:
        out.append((value & 0x7f) | 0x80)
        value >>= 7
    out.append(value)
    return bytes(out)

def length_delimited(field, payload):
    return varint((field << 3) | 2) + varint(len(payload)) + payload

def raw_crx_id(value):
    if len(value) != 32 or any(ch < "a" or ch > "p" for ch in value):
        raise SystemExit("invalid smoke extension id")
    return bytes((ord(value[i]) - 97) << 4 | (ord(value[i + 1]) - 97)
                 for i in range(0, 32, 2))

zip_bytes = io.BytesIO()
with zipfile.ZipFile(zip_bytes, "w", zipfile.ZIP_DEFLATED) as archive:
    for root, _, files in os.walk(source):
        for name in sorted(files):
            path = os.path.join(root, name)
            archive.write(path, os.path.relpath(path, source))

signed_data = length_delimited(1, raw_crx_id(extension_id))
header = length_delimited(10000, signed_data)
with open(target, "wb") as handle:
    handle.write(b"Cr24")
    handle.write(struct.pack("<II", 3, len(header)))
    handle.write(header)
    handle.write(zip_bytes.getvalue())
PY

  cat >"$smoke_dir/manifest.json" <<'JSON'
{
  "manifest_version": 3,
  "name": "Mori Extension Smoke",
  "version": "1.0.0",
  "description": "Mori extension runtime smoke test.",
  "background": {
    "service_worker": "background.js"
  },
  "content_scripts": [
    {
      "matches": [
        "<all_urls>"
      ],
      "js": [
        "main-world.js"
      ],
      "run_at": "document_start",
      "world": "MAIN"
    },
    {
      "matches": [
        "<all_urls>"
      ],
      "js": [
        "start.js"
      ],
      "run_at": "document_start"
    },
    {
      "matches": [
        "<all_urls>"
      ],
      "js": [
        "content.js"
      ],
      "run_at": "document_end"
    },
    {
      "matches": [
        "<all_urls>"
      ],
      "js": [
        "idle.js"
      ],
      "run_at": "document_idle"
    }
  ],
  "action": {
    "default_popup": "popup.html"
  },
  "side_panel": {
    "default_path": "sidepanel.html"
  },
  "web_accessible_resources": [
    {
      "resources": [
        "public.txt"
      ],
      "matches": [
        "<all_urls>"
      ]
    }
  ],
  "commands": {
    "_execute_action": {
      "suggested_key": {
        "default": "Command+Shift+Y"
      },
      "description": "Open the Mori smoke popup"
    },
    "mori-smoke-command": {
      "suggested_key": {
        "default": "Command+Shift+U"
      },
      "description": "Dispatch a Mori smoke command event"
    }
  },
  "host_permissions": [
    "<all_urls>"
  ],
  "externally_connectable": {
    "matches": [
      "http://127.0.0.1/*"
    ]
  },
  "permissions": [
    "alarms",
    "bookmarks",
    "browsingData",
    "clipboardRead",
    "clipboardWrite",
    "contentSettings",
    "contextMenus",
    "cookies",
    "declarativeNetRequest",
    "dns",
    "downloads",
    "history",
    "idle",
	    "identity",
	    "management",
	    "nativeMessaging",
	    "offscreen",
	    "power",
	    "privacy",
	    "proxy",
	    "sessions",
	    "search",
    "sidePanel",
    "scripting",
    "storage",
    "system.cpu",
    "system.display",
    "system.memory",
    "system.storage",
    "topSites",
    "userScripts",
    "webNavigation",
    "webRequest"
  ],
  "optional_permissions": [
    "notifications"
  ],
  "optional_host_permissions": [
    "https://example.com/*"
  ]
}
JSON

  cat >"$smoke_companion_dir/manifest.json" <<'JSON'
{
  "manifest_version": 3,
  "name": "Mori Smoke Companion",
  "version": "1.0.0",
  "description": "Second extension for management API smoke coverage.",
  "background": {
    "service_worker": "background.js"
  },
  "permissions": [
    "management"
  ]
}
JSON

  cat >"$smoke_companion_dir/background.js" <<'JS'
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message && message.type === "lifecycle-smoke") {
    chrome.runtime.setUninstallURL(message.uninstallURL || "").then(() => {
      sendResponse({ uninstallURLSet: true, runtimeId: chrome.runtime.id });
      setTimeout(() => {
        chrome.management.uninstallSelf({ showConfirmDialog: false });
      }, 0);
    }, (error) => {
      sendResponse({
        uninstallURLSet: false,
        error: error && error.message || String(error)
      });
    });
    return true;
  }
  return false;
});
JS

  cat >"$smoke_dir/start.js" <<'JS'
document.documentElement.dataset.moriStartContentScriptSmoke = "loaded";
JS

  cat >"$smoke_dir/main-world.js" <<'JS'
document.documentElement.dataset.moriMainWorldManifestSmoke =
  (globalThis.chrome && globalThis.chrome.runtime &&
    globalThis.chrome.runtime.id) ? "runtime" : "page";
JS

  cat >"$smoke_dir/content.js" <<'JS'
document.documentElement.dataset.moriContentScriptSmoke = "loaded";

function moriSenderSnapshot(sender) {
  return {
    id: sender && sender.id,
    url: sender && sender.url,
    origin: sender && sender.origin,
    frameId: sender && sender.frameId,
    tab: sender && sender.tab ? {
      id: sender.tab.id,
      url: sender.tab.url,
      active: sender.tab.active
    } : null
  };
}

async function moriContentPortRoundTrip(runId) {
  return new Promise((resolve, reject) => {
    const port = chrome.runtime.connect(chrome.runtime.id, {
      name: "content-focus-" + runId
    });
    const timer = setTimeout(() => {
      try { port.disconnect(); } catch (_) {}
      reject(new Error("content port timeout"));
    }, 1000);
    port.onMessage.addListener((message) => {
      clearTimeout(timer);
      resolve(message);
      port.disconnect();
    });
    port.postMessage({ type: "smoke-port-ping", runId, source: "content" });
  });
}

document.addEventListener("focusin", (event) => {
  const input = event.target;
  if (!(input instanceof HTMLInputElement)) return;
  const runId = document.body && document.body.dataset.moriSmokePage || "";
  input.dataset.moriExtensionFocusSmoke = runId;
  input.value = "focused-" + runId;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  const overlay = document.createElement("button");
  overlay.type = "button";
  overlay.dataset.moriExtensionInputOverlay = runId;
  overlay.textContent = "extension overlay";
  input.insertAdjacentElement("afterend", overlay);
  Promise.all([
    chrome.runtime.sendMessage(chrome.runtime.id, {
      type: "content-focus-smoke",
      runId,
      fieldId: input.id
    }),
    moriContentPortRoundTrip(runId)
  ]).then(([background, port]) => {
    window.__moriContentFocusBackground = background;
    window.__moriContentFocusPort = port;
    document.documentElement.dataset.moriContentFocusSmoke = "done";
  }).catch((error) => {
    window.__moriContentFocusError = error && error.message || String(error);
    document.documentElement.dataset.moriContentFocusSmoke = "error";
  });
}, true);

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message && message.type === "content-smoke-ping") {
    const input = document.getElementById("mori-smoke-input");
    const overlay = document.querySelector("[data-mori-extension-input-overlay]");
    sendResponse({
      contentPong: true,
      href: location.href,
      runtimeId: chrome.runtime.id,
      startLoaded: document.documentElement.dataset.moriStartContentScriptSmoke,
      endLoaded: document.documentElement.dataset.moriContentScriptSmoke,
      idleLoaded: document.documentElement.dataset.moriIdleContentScriptSmoke,
      mainWorldLoaded: document.documentElement.dataset.moriMainWorldManifestSmoke,
      sender: moriSenderSnapshot(sender),
      marker: document.body && document.body.dataset.moriSmokePage,
      focus: {
        done: document.documentElement.dataset.moriContentFocusSmoke,
        error: window.__moriContentFocusError || "",
        activeId: document.activeElement && document.activeElement.id,
        inputValue: input && input.value,
        inputDataset: input && input.dataset.moriExtensionFocusSmoke,
        overlayRunId: overlay && overlay.dataset.moriExtensionInputOverlay,
        background: window.__moriContentFocusBackground || null,
        port: window.__moriContentFocusPort || null
      }
    });
  }
  return true;
});
JS

  cat >"$smoke_dir/frame-target.js" <<'JS'
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message && message.type === "frame-target-smoke") {
    const context = chrome.runtime.getExtensionContext &&
      chrome.runtime.getExtensionContext();
    sendResponse({
      frameTargetPong: true,
      href: location.href,
      frameId: chrome.runtime.getFrameId(),
      documentId: context && context.documentId,
      context,
      marker: document.body && (
        document.body.dataset.moriSmokePage ||
        document.body.dataset.moriSmokeFrame
      ),
      sender: {
        id: sender && sender.id,
        url: sender && sender.url,
        frameId: sender && sender.frameId,
        documentId: sender && sender.documentId
      }
    });
  }
  return true;
});
JS

  cat >"$smoke_dir/idle.js" <<'JS'
document.documentElement.dataset.moriIdleContentScriptSmoke = "loaded";
JS

  cat >"$smoke_dir/protected-late.js" <<'JS'
if (!(globalThis.chrome &&
    globalThis.chrome.runtime &&
    globalThis.chrome.runtime.id)) {
  throw new Error("late dynamic script missing chrome.runtime.id");
}
document.documentElement.dataset.moriProtectedLateRuntime =
  globalThis.chrome.runtime.id;
JS

  cat >"$smoke_dir/dynamic.js" <<'JS'
document.documentElement.dataset.moriDynamicContentScriptSmoke = "loaded";

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message && message.type === "dynamic-smoke-ping") {
    const url = new URL(location.href);
    sendResponse({
      dynamicPong: true,
      href: location.href,
      runtimeId: chrome.runtime.id,
      marker: url.searchParams.get("dynamic"),
      sender: {
        id: sender && sender.id,
        url: sender && sender.url,
        origin: sender && sender.origin,
        frameId: sender && sender.frameId
      }
    });
  }
  return true;
});
JS

  cat >"$smoke_dir/worker-extra.js" <<JS
globalThis.__moriImportScriptsSmoke = "loaded-" + "$run_id";
JS

  cat >"$smoke_dir/background.js" <<'JS'
importScripts("worker-extra.js");

const commandEvents = [];
const notificationClosedEvents = [];
const alarmEvents = [];
const externalMessages = [];

chrome.commands.onCommand.addListener((command) => {
  commandEvents.push(command);
});

chrome.notifications.onClosed.addListener((notificationId, byUser) => {
  notificationClosedEvents.push({ notificationId, byUser });
});

chrome.alarms.onAlarm.addListener((alarm) => {
  alarmEvents.push(alarm);
});

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message && message.type === "smoke-ping") {
    sendResponse({
      pong: true,
      importScripts: globalThis.__moriImportScriptsSmoke || null,
      sender: {
        id: sender && sender.id,
        url: sender && sender.url,
        origin: sender && sender.origin,
        frameId: sender && sender.frameId
      }
    });
  }
  if (message && message.type === "content-focus-smoke") {
    sendResponse({
      focusPong: true,
      message,
      sender: {
        id: sender && sender.id,
        url: sender && sender.url,
        origin: sender && sender.origin,
        frameId: sender && sender.frameId,
        tab: sender && sender.tab ? {
          id: sender.tab.id,
          url: sender.tab.url,
          active: sender.tab.active
        } : null
      }
    });
  }
  if (message && message.type === "command-smoke-state") {
    sendResponse({
      commandEvents: commandEvents.slice()
    });
  }
  if (message && message.type === "notification-smoke-state") {
    chrome.notifications.getAll().then((all) => {
      sendResponse({
        all,
        closedEvents: notificationClosedEvents.slice()
      });
    });
  }
  if (message && message.type === "alarm-smoke-start") {
    const name = message.name || "mori-alarm-smoke";
    alarmEvents.length = 0;
    chrome.alarms.clearAll().then(() => {
      chrome.alarms.create(name, { delayInMinutes: 0.001 });
      Promise.all([
        chrome.alarms.get(name),
        chrome.alarms.getAll()
      ]).then(([created, all]) => {
        sendResponse({ created, all });
      });
    });
  }
  if (message && message.type === "alarm-smoke-state") {
    sendResponse({ events: alarmEvents.slice() });
  }
  if (message && message.type === "legacy-extension-views-smoke") {
    const allViews = chrome.extension.getViews();
    const popupViews = chrome.extension.getViews({ type: "popup" });
    const tabViews = chrome.extension.getViews({ type: "tab" });
    const extensionTabs = chrome.extension.getExtensionTabs();
    Promise.resolve(chrome.runtime.getBackgroundPage()).then((backgroundPage) => {
      sendResponse({
        namespace: !!chrome.extension,
        inIncognitoContext: chrome.extension.inIncognitoContext,
        backgroundPageIsSelf: chrome.extension.getBackgroundPage() === globalThis,
        runtimeBackgroundPageIsSelf: backgroundPage === globalThis,
        allViewsIncludeSelf: allViews.includes(globalThis),
        allViewsLength: allViews.length,
        popupViewsLength: popupViews.length,
        tabViewsLength: tabViews.length,
        extensionTabsLength: extensionTabs.length,
        getURL: chrome.extension.getURL("popup.html")
      });
    });
  }
  return true;
});

chrome.runtime.onMessageExternal.addListener((message, sender, sendResponse) => {
  const externalSender = {
    id: sender && sender.id,
    url: sender && sender.url,
    origin: sender && sender.origin,
    frameId: sender && sender.frameId,
    tab: sender && sender.tab ? {
      id: sender.tab.id,
      url: sender.tab.url,
      active: sender.tab.active
    } : null
  };
  externalMessages.push({ message, sender: externalSender });
  if (message && message.type === "fork") {
    sendResponse({
      externalPong: true,
      runtimeId: chrome.runtime.id,
      message,
      sender: externalSender,
      count: externalMessages.length
    });
    return true;
  }
  return false;
});

chrome.runtime.onConnect.addListener((port) => {
  port.onMessage.addListener((message) => {
    if (message && message.type === "smoke-port-ping") {
      port.postMessage({
        pong: true,
        sender: port.sender || null,
        name: port.name
      });
    }
  });
});
JS

  cat >"$smoke_dir/popup.html" <<HTML
<!doctype html>
<meta charset="utf-8">
<title>Mori Extension Smoke</title>
<body>Starting Mori extension smoke...</body>
<script>
window.__moriEarlyNavigatorOnline = navigator.onLine === true;
</script>
<script src="popup.js"></script>
HTML

  cat >"$smoke_dir/public.txt" <<TXT
public-web-accessible-$run_id
TXT

  cat >"$smoke_dir/private.txt" <<TXT
private-extension-resource-$run_id
TXT

  cat >"$smoke_dir/sidepanel.html" <<HTML
<!doctype html>
<meta charset="utf-8">
<title>Mori Side Panel Smoke</title>
<body>Mori side panel smoke</body>
<script src="sidepanel.js"></script>
HTML

  cat >"$smoke_dir/sidepanel.js" <<JS
document.body.dataset.moriSidePanelSmoke = "$run_id";
chrome.storage.local.set({
  sidePanelLoaded: "$run_id",
  sidePanelRuntimeId: chrome.runtime.id,
  sidePanelURL: location.href
});
JS

  cat >"$smoke_dir/offscreen.html" <<HTML
<!doctype html>
<meta charset="utf-8">
<title>Mori Offscreen Smoke</title>
<body>Mori offscreen smoke</body>
<script src="offscreen.js"></script>
HTML

  cat >"$smoke_dir/offscreen.js" <<JS
document.body.dataset.moriOffscreenSmoke = "$run_id";
const clipboardTarget = document.createElement("textarea");
clipboardTarget.id = "mori-offscreen-clipboard-target";
document.body.appendChild(clipboardTarget);

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (!message || message.type !== "clipboard-smoke") {
    return false;
  }
  (async () => {
    const value = String(message.value || "");
    const hasPermissions = await chrome.permissions.contains({
      permissions: ["clipboardRead", "clipboardWrite"]
    });
    let original = null;
    try {
      original = await navigator.clipboard.readText();
    } catch (_) {
      original = null;
    }

    await navigator.clipboard.writeText(value);
    const asyncRead = await navigator.clipboard.readText();

    clipboardTarget.value = value;
    clipboardTarget.focus();
    clipboardTarget.select();
    const copyCommand = document.execCommand("copy");

    clipboardTarget.value = "";
    clipboardTarget.focus();
    const pasteCommand = document.execCommand("paste");
    const pasted = clipboardTarget.value;
    const finalRead = await navigator.clipboard.readText();

    if (original !== null) {
      await navigator.clipboard.writeText(original);
    }

    sendResponse({
      hasPermissions,
      hasClipboardApi: !!(navigator.clipboard &&
        navigator.clipboard.writeText &&
        navigator.clipboard.readText),
      execCommandWrapped: String(document.execCommand).includes("__moriClipboard"),
      asyncRead,
      copyCommand,
      pasteCommand,
      pasted,
      finalRead,
      restoredOriginal: original !== null
    });
  })().catch((error) => {
    sendResponse({
      error: error && error.message || String(error)
    });
  });
  return true;
});

chrome.storage.local.set({
  offscreenLoaded: "$run_id",
  offscreenRuntimeId: chrome.runtime.id,
  offscreenURL: location.href,
  offscreenDocumentHidden: document.hidden
});
JS

  cat >"$smoke_dir/popup.js" <<JS
(async () => {
  const runId = "$run_id";
  const extensionId = "$smoke_id";
  const companionExtensionId = "$smoke_companion_id";
  const smokePageURL = "file://$smoke_page";
  const capturePageURL = "file://$smoke_capture_page";
  const authPageURL = "file://$smoke_auth_page";
  const slowDownloadURL = "$SMOKE_DOWNLOAD_URL";
  const crxURL = "$SMOKE_CRX_URL";
  const externalPageURL = "$SMOKE_EXTERNAL_PAGE_URL";
  const corsURL = "$SMOKE_CORS_URL";
  const crxExtensionId = "$SMOKE_CRX_EXTENSION_ID";
  const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const primaryRunnerKey = "mori-smoke-primary-runner-" + runId;
  if (localStorage.getItem(primaryRunnerKey)) {
    document.body.textContent = "Mori extension smoke popup opened";
    return;
  }
  localStorage.setItem(primaryRunnerKey, "1");
  const withTimeout = (promise, ms) => Promise.race([
    promise,
    new Promise((_, reject) => setTimeout(() => reject(new Error("timeout")), ms))
  ]);

  async function retry(label, task) {
    let lastError = null;
    for (let i = 0; i < 25; i += 1) {
      try {
        return await withTimeout(task(), 500);
      } catch (error) {
        lastError = error;
        await delay(150);
      }
    }
    throw new Error(label + ": " + (lastError && lastError.message || "no response"));
  }

  async function runtimeIntrospectionRoundTrip() {
    const platformPromise = await chrome.runtime.getPlatformInfo();
    const platformCallback = await new Promise((resolve) => {
      chrome.runtime.getPlatformInfo((platform) => resolve(platform));
    });
    const updatePromise = await chrome.runtime.requestUpdateCheck();
    const updateCallback = await new Promise((resolve) => {
      chrome.runtime.requestUpdateCheck((status, details) => {
        resolve({
          status,
          details: details || null
        });
      });
    });
    let restartAfterDelayCallback = false;
    const restartAfterDelayPromise = chrome.runtime.restartAfterDelay(1, () => {
      restartAfterDelayCallback = true;
    });
    await restartAfterDelayPromise;
    chrome.runtime.restart();
    const reloadResult = chrome.runtime.reload();
    return {
      version: chrome.runtime.getVersion(),
      manifestVersion: chrome.runtime.getManifest().version,
      platformPromise,
      platformCallback,
      updatePromise,
      updateCallback,
      restartAfterDelayCallback,
      reloadResultType: typeof reloadResult
    };
  }

  async function powerRoundTrip() {
    const initial = await chrome.power.__moriSmokeState();
    const systemReturn = await chrome.power.requestKeepAwake(
      chrome.power.Level.SYSTEM);
    const afterSystem = await retry("power system keep-awake", async () => {
      const state = await chrome.power.__moriSmokeState();
      if (!state || state.level !== "system") {
        throw new Error("system assertion not active");
      }
      return state;
    });
    const displayReturn = await chrome.power.requestKeepAwake(
      chrome.power.Level.DISPLAY);
    const afterDisplay = await retry("power display keep-awake", async () => {
      const state = await chrome.power.__moriSmokeState();
      if (!state || state.level !== "display") {
        throw new Error("display assertion not active");
      }
      return state;
    });
    const releaseReturn = await chrome.power.releaseKeepAwake();
    const afterRelease = await retry("power release", async () => {
      const state = await chrome.power.__moriSmokeState();
      if (state !== null) {
        throw new Error("power assertion still active");
      }
      return state;
    });
    let invalidError = "";
    try {
      await chrome.power.requestKeepAwake("screen");
    } catch (error) {
      invalidError = error && error.message || String(error);
    }
    return {
      initial,
      levels: chrome.power.Level,
      systemReturn,
      afterSystem,
      displayReturn,
      afterDisplay,
      releaseReturn,
      afterRelease,
      invalidError
    };
  }

  async function idleRoundTrip() {
    const validStates = ["active", "idle", "locked"];
    const events = [];
    chrome.idle.onStateChanged.addListener((state) => {
      events.push(state);
    });
    const callbackState = await new Promise((resolve) => {
      chrome.idle.queryState(1, (state) => resolve(state));
    });
    const promiseState = await chrome.idle.queryState(1);
    const setReturn = await chrome.idle.setDetectionInterval(1);
    await delay(1100);
    const browserState = await browser.idle.queryState(1);
    const autoLockDelay = await chrome.idle.getAutoLockDelay();
    const autoLockDelayCallback = await new Promise((resolve) => {
      chrome.idle.getAutoLockDelay((delay) => resolve(delay));
    });
    return {
      namespace: !!(chrome.idle &&
        chrome.idle.queryState &&
        chrome.idle.setDetectionInterval &&
        chrome.idle.getAutoLockDelay &&
        chrome.idle.onStateChanged),
      browserNamespace: !!(browser.idle &&
        browser.idle.queryState &&
        browser.idle.setDetectionInterval &&
        browser.idle.getAutoLockDelay),
      idleStateEnum: chrome.idle.IdleState,
      callbackState,
      promiseState,
      browserState,
      statesValid: validStates.includes(callbackState) &&
        validStates.includes(promiseState) &&
        validStates.includes(browserState) &&
        events.every((state) => validStates.includes(state)),
      setReturn,
      autoLockDelay,
      autoLockDelayCallback,
      events
    };
  }

  async function systemRoundTrip() {
    const cpu = await chrome.system.cpu.getInfo();
    const cpuCallback = await new Promise((resolve) => {
      chrome.system.cpu.getInfo((info) => resolve(info));
    });
    const memory = await chrome.system.memory.getInfo();
    const memoryCallback = await new Promise((resolve) => {
      chrome.system.memory.getInfo((info) => resolve(info));
    });
    const displays = await chrome.system.display.getInfo();
    const displaysCallback = await new Promise((resolve) => {
      chrome.system.display.getInfo((info) => resolve(info));
    });
    const storageUnits = await chrome.system.storage.getInfo();
    const storageCallback = await new Promise((resolve) => {
      chrome.system.storage.getInfo((info) => resolve(info));
    });
    const firstStorageId =
      storageUnits && storageUnits[0] && storageUnits[0].id || "";
    const available = firstStorageId
      ? await chrome.system.storage.getAvailableCapacity(firstStorageId)
      : null;
    const availableCallback = firstStorageId
      ? await new Promise((resolve) => {
          chrome.system.storage.getAvailableCapacity(firstStorageId,
            (info) => resolve(info));
        })
      : null;
    const ejectMissing = await chrome.system.storage.ejectDevice(
      "missing-" + runId);
    const ejectFixed = firstStorageId
      ? await chrome.system.storage.ejectDevice(firstStorageId)
      : "";
    return {
      hasNamespace: !!(chrome.system &&
        chrome.system.cpu &&
        chrome.system.memory &&
        chrome.system.display &&
        chrome.system.storage),
      browserNamespace: !!(globalThis.browser &&
        browser.system &&
        browser.system.cpu &&
        browser.system.memory &&
        browser.system.display &&
        browser.system.storage),
      storageEvents: {
        attached: !!(chrome.system.storage.onAttached &&
          chrome.system.storage.onAttached.addListener),
        detached: !!(chrome.system.storage.onDetached &&
          chrome.system.storage.onDetached.addListener)
      },
      cpu,
      cpuCallback,
      memory,
      memoryCallback,
      displays,
      displaysCallback,
      storageUnits,
      storageCallback,
      available,
      availableCallback,
      ejectMissing,
      ejectFixed
    };
  }

  function moriEncodedQueryToken(text) {
    return encodeURIComponent(text);
  }

  async function waitForSearchURL(tabId, text) {
    const token = moriEncodedQueryToken(text);
    return retry("search tab " + tabId, async () => {
      const tab = await chrome.tabs.get(tabId);
      if (!tab || !String(tab.url || "").includes(token)) {
        throw new Error("search URL not loaded yet");
      }
      return tab;
    });
  }

  async function searchRoundTrip() {
    const tabText = "mori search tabid " + runId;
    const currentText = "mori search current " + runId;
    const newTabText = "mori search newtab " + runId;
    const browserText = "mori search browser " + runId;

    const tabTarget = await chrome.tabs.create({
      url: "about:blank",
      active: false
    });
    const tabReturn = await chrome.search.query({
      text: tabText,
      tabId: tabTarget.id
    });
    const tabAfter = await waitForSearchURL(tabTarget.id, tabText);

    const currentTarget = await chrome.tabs.create({
      url: "about:blank",
      active: true
    });
    const currentReturn = await chrome.search.query({
      text: currentText
    });
    const currentAfter = await waitForSearchURL(currentTarget.id, currentText);

    const beforeNewTabs = await chrome.tabs.query({});
    const newTabCallback = await new Promise((resolve) => {
      chrome.search.query({
        text: newTabText,
        disposition: chrome.search.Disposition.NEW_TAB
      }, () => {
        resolve({
          lastError: chrome.runtime.lastError &&
            chrome.runtime.lastError.message || ""
        });
      });
    });
    const newTabAfter = await retry("search new tab", async () => {
      const tabs = await chrome.tabs.query({});
      const token = moriEncodedQueryToken(newTabText);
      const tab = tabs.find((item) =>
        item && String(item.url || "").includes(token));
      if (!tab) throw new Error("new search tab not visible");
      return tab;
    });

    const browserReturn = await browser.search.query({
      text: browserText,
      disposition: "NEW_WINDOW"
    });
    const browserAfter = await retry("browser.search new window", async () => {
      const tabs = await chrome.tabs.query({});
      const token = moriEncodedQueryToken(browserText);
      const tab = tabs.find((item) =>
        item && String(item.url || "").includes(token));
      if (!tab) throw new Error("browser.search tab not visible");
      return tab;
    });

    let invalidError = "";
    try {
      await chrome.search.query({
        text: "invalid " + runId,
        tabId: tabTarget.id,
        disposition: "NEW_TAB"
      });
    } catch (error) {
      invalidError = error && error.message || String(error);
    }

    await chrome.tabs.remove([
      tabTarget.id,
      currentTarget.id,
      newTabAfter.id,
      browserAfter.id
    ]).catch(() => null);

    return {
      namespace: {
        chrome: !!(chrome.search && chrome.search.query),
        browser: !!(browser.search && browser.search.query),
        disposition: chrome.search.Disposition
      },
      tabText,
      currentText,
      newTabText,
      browserText,
      tabReturn,
      currentReturn,
      browserReturn,
      tabAfter,
      currentAfter,
      beforeNewTabCount: beforeNewTabs.length,
      newTabCallback,
      newTabAfter,
      browserAfter,
      invalidError
    };
  }

  async function dnsRoundTrip() {
    const promiseResult = await chrome.dns.resolve("localhost");
    const callbackResult = await new Promise((resolve) => {
      chrome.dns.resolve("localhost", (info) => resolve(info));
    });
    const browserResult = await browser.dns.resolve("localhost");
    const invalidResult = await chrome.dns.resolve("https://localhost/");
    return {
      namespace: !!(chrome.dns && chrome.dns.resolve),
      browserNamespace: !!(browser.dns && browser.dns.resolve),
      promiseResult,
      callbackResult,
      browserResult,
      invalidResult
    };
  }

  function portRoundTrip() {
    return new Promise((resolve, reject) => {
      const port = chrome.runtime.connect(chrome.runtime.id, {
        name: "smoke-" + runId
      });
      const timer = setTimeout(() => reject(new Error("port timeout")), 500);
      port.onMessage.addListener((message) => {
        clearTimeout(timer);
        resolve(message);
        port.disconnect();
      });
      port.postMessage({ type: "smoke-port-ping", runId });
    });
  }

  async function externalMessageRoundTrip() {
    const tab = await chrome.tabs.create({ url: externalPageURL, active: true });
    try {
      return await retry("external runtime.sendMessage", async () => {
        const scriptResult = await chrome.scripting.executeScript({
          target: { tabId: tab.id },
          func: () => ({
            title: document.title,
            result: document.body && document.body.dataset.result || "",
            error: document.body && document.body.dataset.error || ""
          })
        });
        const state = scriptResult && scriptResult[0] && scriptResult[0].result;
        if (!state || state.title !== "external-ok" || !state.result) {
          throw new Error(state && state.error || "external page not ready");
        }
        return {
          tab,
          response: JSON.parse(state.result)
        };
      });
    } finally {
      await chrome.tabs.remove(tab.id);
    }
  }

  async function extensionFetchRoundTrip() {
    const cookieName = "mori_fetch_" + runId.replace(/[^A-Za-z0-9_]/g, "_");
    await chrome.cookies.remove({ url: corsURL, name: cookieName }).catch(() => null);
    await chrome.cookies.set({
      url: corsURL,
      name: cookieName,
      value: runId
    });
    const postResponse = await withTimeout(fetch(corsURL + "&phase=post", {
      method: "POST",
      credentials: "include",
      headers: {
        "Content-Type": "application/json",
        "X-Mori-CORS-Smoke": runId
      },
      body: JSON.stringify({
        runId,
        source: "popup"
      })
    }), 3000).catch((error) => {
      throw new Error("extension fetch POST: " +
        (error && error.message || String(error)));
    });
    const postJSON = await withTimeout(postResponse.json(), 1000);
    const getResponse = await withTimeout(fetch(corsURL + "&phase=get", {
      credentials: "include",
      headers: {
        "X-Mori-CORS-Smoke": runId + "-get"
      }
    }), 3000).catch((error) => {
      throw new Error("extension fetch GET: " +
        (error && error.message || String(error)));
    });
    const getJSON = await withTimeout(getResponse.json(), 1000);
    return {
      postStatus: postResponse.status,
      postJSON,
      getStatus: getResponse.status,
      getJSON,
      cookieName,
      cookieValue: runId
    };
  }

  async function contentScriptRoundTrip() {
    const frameScriptId = "mori-frame-target-smoke";
    await chrome.scripting.unregisterContentScripts({ ids: [frameScriptId] })
      .catch(() => null);
    await chrome.scripting.registerContentScripts([{
      id: frameScriptId,
      matches: ["<all_urls>"],
      js: ["frame-target.js"],
      runAt: "document_end",
      allFrames: true
    }]);
    const tab = await chrome.tabs.create({ url: smokePageURL, active: true });
    try {
      await delay(700);
      await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        func: () => {
          const input = document.getElementById("mori-smoke-input");
          if (!input) return false;
          input.focus();
          input.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
          return document.activeElement === input;
        }
      });
      const result = await retry("tabs.sendMessage", () =>
        chrome.tabs.sendMessage(tab.id, { type: "content-smoke-ping", runId })
          .then((result) => {
            if (!result ||
                !result.focus ||
                result.focus.done !== "done") {
              throw new Error(result && result.focus &&
                (result.focus.error || result.focus.done) ||
                "content focus path not ready");
            }
            return result;
          }));
      const protectedSetup = await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        world: "MAIN",
        func: () => {
          const runtimeIdOf = (value) => {
            try {
              return value && value.runtime && value.runtime.id || null;
            } catch (_) {
              return null;
            }
          };
          const installProtected = (name, value) => {
            const beforeRuntime = runtimeIdOf(globalThis[name]);
            let mode = "none";
            let error = "";
            try {
              Object.defineProperty(globalThis, name, {
                configurable: true,
                writable: true,
                value
              });
              mode = "defineProperty";
            } catch (defineError) {
              error = defineError && defineError.message || String(defineError);
              try {
                globalThis[name] = value;
                mode = "assignment";
              } catch (assignError) {
                error += " / " +
                  (assignError && assignError.message || String(assignError));
              }
            }
            return {
              beforeRuntime,
              afterRuntime: runtimeIdOf(globalThis[name]),
              mode,
              error
            };
          };
          const realChrome = globalThis.chrome;
          const realBrowser = globalThis.browser;
          const protectedProxy = new Proxy(realChrome || {}, {
            get(target, prop) {
              if (prop === "app") return target[prop];
              return undefined;
            },
            set() {
              return false;
            }
          });
          const protectedBrowserProxy = new Proxy(realBrowser || protectedProxy, {
            get(target, prop) {
              if (prop === "app") return target[prop];
              return undefined;
            },
            set() {
              return false;
            }
          });
          return {
            chrome: installProtected("chrome", protectedProxy),
            browser: installProtected("browser", protectedBrowserProxy)
          };
        }
      });
      await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        files: ["protected-late.js"]
      });
      const protectedDynamic = await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        func: () => ({
          runtimeId: globalThis.chrome &&
            globalThis.chrome.runtime &&
            globalThis.chrome.runtime.id,
          browserRuntimeId: globalThis.browser &&
            globalThis.browser.runtime &&
            globalThis.browser.runtime.id,
          marker: document.documentElement.dataset.moriProtectedLateRuntime
        })
      });
      const frames = await retry("webNavigation.getAllFrames child", async () => {
        const all = await chrome.webNavigation.getAllFrames({ tabId: tab.id });
        const child = (all || []).find((frame) =>
          frame &&
          frame.frameId !== 0 &&
          String(frame.url || "").includes("frame-smoke.html"));
        if (!child) {
          throw new Error("child frame not observed " + JSON.stringify(all));
        }
        return all;
      });
      const childFrame = frames.find((frame) =>
        frame &&
        frame.frameId !== 0 &&
        String(frame.url || "").includes("frame-smoke.html"));
      if (!childFrame.documentId) {
        throw new Error("child frame missing documentId " +
          JSON.stringify(childFrame));
      }
      const documentFrame = await chrome.webNavigation.getFrame({
        tabId: tab.id,
        documentId: childFrame.documentId
      });
      if (!documentFrame ||
          documentFrame.frameId !== childFrame.frameId ||
          documentFrame.documentId !== childFrame.documentId) {
        throw new Error("documentId getFrame mismatch " + JSON.stringify({
          documentFrame,
          childFrame
        }));
      }
      const targeted = await retry("tabs.sendMessage frameId", () =>
        chrome.tabs.sendMessage(tab.id, {
          type: "frame-target-smoke",
          runId
        }, {
          frameId: childFrame.frameId
        }).then((response) => {
          if (!response ||
              response.frameTargetPong !== true ||
              response.frameId !== childFrame.frameId ||
              !String(response.href || "").includes("frame-smoke.html")) {
            throw new Error("wrong frame response " + JSON.stringify({
              response,
              childFrame
            }));
          }
          return response;
        }));
      const documentTargeted = await retry("tabs.sendMessage documentId", () =>
        chrome.tabs.sendMessage(tab.id, {
          type: "frame-target-smoke",
          runId
        }, {
          documentId: childFrame.documentId
        }).then((response) => {
          if (!response ||
              response.frameTargetPong !== true ||
              response.frameId !== childFrame.frameId ||
              response.documentId !== childFrame.documentId ||
              !response.context ||
              response.context.documentId !== childFrame.documentId ||
              !String(response.href || "").includes("frame-smoke.html")) {
            throw new Error("wrong document response " + JSON.stringify({
              response,
              childFrame
            }));
          }
          return response;
        }));
      return {
        ...result,
        protectedDynamic,
        protectedSetup,
        frameTarget: {
          frames,
          childFrame,
          documentFrame,
          targeted,
          documentTargeted
        }
      };
    } finally {
      await chrome.scripting.unregisterContentScripts({ ids: [frameScriptId] })
        .catch(() => null);
      await chrome.tabs.remove(tab.id).catch(() => null);
    }
  }

  async function executeScriptRoundTrip() {
    const tab = await chrome.tabs.create({
      url: smokePageURL + "?execute=" + encodeURIComponent(runId),
      active: true
    });
    await delay(700);
    const result = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: (value) => {
        document.documentElement.dataset.moriExecuteScriptSmoke = value;
        return {
          value,
          marker: document.body && document.body.dataset.moriSmokePage,
          href: location.href,
          dataset: document.documentElement.dataset.moriExecuteScriptSmoke
        };
      },
      args: [runId]
    });
    await chrome.tabs.remove(tab.id);
    const asyncTab = await chrome.tabs.create({
      url: smokePageURL + "?execute-async=" + encodeURIComponent(runId),
      active: true
    });
    await delay(700);
    const asyncResult = await chrome.scripting.executeScript({
      target: { tabId: asyncTab.id },
      func: async (value) => {
        await new Promise((resolve) => setTimeout(resolve, 25));
        document.documentElement.dataset.moriExecuteScriptAsyncSmoke = value;
        return {
          asyncValue: value,
          marker: document.body && document.body.dataset.moriSmokePage,
          href: location.href,
          dataset: document.documentElement.dataset.moriExecuteScriptAsyncSmoke
        };
      },
      args: [runId]
    });
    await chrome.tabs.remove(asyncTab.id);
    const worldTab = await chrome.tabs.create({
      url: smokePageURL + "?execute-world=" + encodeURIComponent(runId),
      active: true
    });
    await delay(700);
    const mainWorldResult = await chrome.scripting.executeScript({
      target: { tabId: worldTab.id },
      world: "MAIN",
      func: () => {
        const previous = globalThis.chrome || {};
        globalThis.chrome = new Proxy(previous, {
          get(target, prop) {
            if (prop === "app") return target[prop];
            return undefined;
          },
          set() {
            return false;
          }
        });
        return {
          mainHasRuntime: !!(globalThis.chrome &&
            globalThis.chrome.runtime &&
            globalThis.chrome.runtime.id)
        };
      }
    });
    const isolatedAfterMainResult = await chrome.scripting.executeScript({
      target: { tabId: worldTab.id },
      func: () => ({
        runtimeId: chrome.runtime && chrome.runtime.id,
        url: chrome.runtime && chrome.runtime.getURL &&
          chrome.runtime.getURL("/dropdown.html"),
        context: chrome.runtime && chrome.runtime.getExtensionContext &&
          chrome.runtime.getExtensionContext()
      })
    });
    await chrome.tabs.remove(worldTab.id);
    const frameTab = await chrome.tabs.create({
      url: smokePageURL + "?execute-frames=" + encodeURIComponent(runId),
      active: true
    });
    await delay(700);
    const frames = await retry("scripting.executeScript frames", async () => {
      const all = await chrome.webNavigation.getAllFrames({ tabId: frameTab.id });
      const child = (all || []).find((frame) =>
        frame &&
        frame.frameId !== 0 &&
        String(frame.url || "").includes("frame-smoke.html"));
      if (!child) {
        throw new Error("child frame not observed " + JSON.stringify(all));
      }
      return all;
    });
    const childFrame = frames.find((frame) =>
      frame &&
      frame.frameId !== 0 &&
      String(frame.url || "").includes("frame-smoke.html"));
    if (!childFrame.documentId) {
      throw new Error("child frame missing documentId " +
        JSON.stringify(childFrame));
    }
    const documentFrame = await chrome.webNavigation.getFrame({
      tabId: frameTab.id,
      documentId: childFrame.documentId
    });
    if (!documentFrame ||
        documentFrame.frameId !== childFrame.frameId ||
        documentFrame.documentId !== childFrame.documentId) {
      throw new Error("documentId execute getFrame mismatch " +
        JSON.stringify({ documentFrame, childFrame }));
    }
    const allFrameResult = await chrome.scripting.executeScript({
      target: { tabId: frameTab.id, allFrames: true },
      func: (value) => ({
        value,
        href: location.href,
        runtimeFrameId: chrome.runtime && chrome.runtime.getFrameId &&
          chrome.runtime.getFrameId(),
        context: chrome.runtime && chrome.runtime.getExtensionContext &&
          chrome.runtime.getExtensionContext(),
        pageMarker: document.body && document.body.dataset.moriSmokePage,
        frameMarker: document.body && document.body.dataset.moriSmokeFrame
      }),
      args: [runId]
    });
    const targetedFrameResult = await chrome.scripting.executeScript({
      target: { tabId: frameTab.id, frameIds: [childFrame.frameId] },
      func: (value) => ({
        value,
        href: location.href,
        runtimeFrameId: chrome.runtime && chrome.runtime.getFrameId &&
          chrome.runtime.getFrameId(),
        context: chrome.runtime && chrome.runtime.getExtensionContext &&
          chrome.runtime.getExtensionContext(),
        frameMarker: document.body && document.body.dataset.moriSmokeFrame
      }),
      args: [runId]
    });
    const targetedDocumentResult = await chrome.scripting.executeScript({
      target: { tabId: frameTab.id, documentIds: [childFrame.documentId] },
      func: (value) => ({
        value,
        href: location.href,
        runtimeFrameId: chrome.runtime && chrome.runtime.getFrameId &&
          chrome.runtime.getFrameId(),
        context: chrome.runtime && chrome.runtime.getExtensionContext &&
          chrome.runtime.getExtensionContext(),
        frameMarker: document.body && document.body.dataset.moriSmokeFrame
      }),
      args: [runId]
    });
    await chrome.tabs.remove(frameTab.id);
    return {
      sync: result,
      async: asyncResult,
      world: {
        main: mainWorldResult,
        isolatedAfterMain: isolatedAfterMainResult
      },
      frames: {
        observed: frames,
        childFrame,
        documentFrame,
        all: allFrameResult,
        targeted: targetedFrameResult,
        documentTargeted: targetedDocumentResult
      }
    };
  }

  async function cssInjectionRoundTrip() {
    const tab = await chrome.tabs.create({
      url: smokePageURL + "?css=" + encodeURIComponent(runId),
      active: true
    });
    await delay(700);
    await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => {
        document.body.style.backgroundColor = "rgb(1, 2, 3)";
        document.body.style.color = "rgb(4, 5, 6)";
      }
    });
    const css = "body { background-color: rgb(9, 87, 165) !important; }";
    await chrome.scripting.insertCSS({
      target: { tabId: tab.id },
      css
    });
    const afterInsert = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => getComputedStyle(document.body).backgroundColor
    });
    await chrome.scripting.removeCSS({
      target: { tabId: tab.id },
      css
    });
    const afterRemove = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => getComputedStyle(document.body).backgroundColor
    });
    const legacyCSS = "body { color: rgb(201, 77, 33) !important; }";
    await chrome.tabs.insertCSS(tab.id, { code: legacyCSS });
    const legacyAfterInsert = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => getComputedStyle(document.body).color
    });
    await chrome.tabs.removeCSS(tab.id, { code: legacyCSS });
    const legacyAfterRemove = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => getComputedStyle(document.body).color
    });
    await chrome.tabs.remove(tab.id);
    const frameTab = await chrome.tabs.create({
      url: smokePageURL + "?css-frames=" + encodeURIComponent(runId),
      active: true
    });
    await delay(700);
    let frameTargetCSS = null;
    try {
      const frames = await retry("scripting CSS frames", async () => {
        const all = await chrome.webNavigation.getAllFrames({ tabId: frameTab.id });
        const child = (all || []).find((frame) =>
          frame &&
          frame.frameId !== 0 &&
          String(frame.url || "").includes("frame-smoke.html"));
        if (!child) {
          throw new Error("child frame not observed " + JSON.stringify(all));
        }
        return all;
      });
      const childFrame = frames.find((frame) =>
        frame &&
        frame.frameId !== 0 &&
        String(frame.url || "").includes("frame-smoke.html"));
      await chrome.scripting.executeScript({
        target: { tabId: frameTab.id, allFrames: true },
        func: () => {
          document.body.style.backgroundColor = "rgb(11, 22, 33)";
        }
      });
      const frameCSS =
        "body { background-color: rgb(88, 99, 111) !important; }";
      await chrome.scripting.insertCSS({
        target: { tabId: frameTab.id, frameIds: [childFrame.frameId] },
        css: frameCSS
      });
      const frameAfterInsert = await chrome.scripting.executeScript({
        target: { tabId: frameTab.id, allFrames: true },
        func: () => ({
          href: location.href,
          color: getComputedStyle(document.body).backgroundColor
        })
      });
      await chrome.scripting.removeCSS({
        target: { tabId: frameTab.id, frameIds: [childFrame.frameId] },
        css: frameCSS
      });
      const frameAfterRemove = await chrome.scripting.executeScript({
        target: { tabId: frameTab.id, allFrames: true },
        func: () => ({
          href: location.href,
          color: getComputedStyle(document.body).backgroundColor
        })
      });
      frameTargetCSS = {
        frames,
        childFrame,
        afterInsert: frameAfterInsert,
        afterRemove: frameAfterRemove
      };
    } finally {
      await chrome.tabs.remove(frameTab.id).catch(() => null);
    }
    return {
      tab,
      afterInsert,
      afterRemove,
      legacyAfterInsert,
      legacyAfterRemove,
      frameTargetCSS
    };
  }

  async function storageAreasRoundTrip() {
    await chrome.storage.sync.clear();
    await chrome.storage.session.clear();
    await chrome.storage.sync.setAccessLevel({
      accessLevel: "TRUSTED_AND_UNTRUSTED_CONTEXTS"
    });
    await chrome.storage.session.setAccessLevel({
      accessLevel: "TRUSTED_CONTEXTS"
    });

    await chrome.storage.sync.set({
      syncKey: "sync-" + runId,
      syncCount: 2
    });
    const syncStored = await chrome.storage.sync.get([
      "syncKey",
      "syncCount",
      "missingSync"
    ]);
    const syncKeys = await chrome.storage.sync.getKeys();
    const syncBytes = await chrome.storage.sync.getBytesInUse([
      "syncKey",
      "syncCount"
    ]);
    await chrome.storage.sync.remove("syncCount");
    const syncAfterRemove = await chrome.storage.sync.get([
      "syncKey",
      "syncCount"
    ]);

    await chrome.storage.session.set({
      sessionKey: "session-" + runId,
      sessionFlag: true
    });
    const sessionStored = await chrome.storage.session.get([
      "sessionKey",
      "sessionFlag"
    ]);
    const sessionKeys = await chrome.storage.session.getKeys();
    const sessionBytes = await chrome.storage.session.getBytesInUse([
      "sessionKey",
      "sessionFlag"
    ]);
    await chrome.storage.session.clear();
    const sessionAfterClear = await chrome.storage.session.get([
      "sessionKey",
      "sessionFlag"
    ]);

    const managedStored = await chrome.storage.managed.get(null);
    const managedKeys = await chrome.storage.managed.getKeys();
    let managedSetError = "";
    try {
      await chrome.storage.managed.set({ blockedManagedWrite: runId });
    } catch (error) {
      managedSetError = error && error.message || String(error);
    }

    return {
      syncStored,
      syncKeys,
      syncBytes,
      syncAfterRemove,
      sessionStored,
      sessionKeys,
      sessionBytes,
      sessionAfterClear,
      managedStored,
      managedKeys,
      managedSetError
    };
  }

  async function dynamicContentScriptRoundTrip() {
    const scriptId = "mori-dynamic-smoke";
    await chrome.scripting.unregisterContentScripts({ ids: [scriptId] });
    await chrome.scripting.registerContentScripts([{
      id: scriptId,
      matches: ["<all_urls>"],
      js: ["dynamic.js"],
      runAt: "document_end",
      allFrames: false
    }]);
    const registered = await chrome.scripting.getRegisteredContentScripts({
      ids: [scriptId]
    });
    const tab = await chrome.tabs.create({
      url: smokePageURL + "?dynamic=" + encodeURIComponent(runId),
      active: true
    });
    await delay(700);
    const response = await retry("dynamic content script", () =>
      chrome.tabs.sendMessage(tab.id, { type: "dynamic-smoke-ping", runId }));
    await chrome.scripting.updateContentScripts([{
      id: scriptId,
      matches: ["<all_urls>"],
      js: ["dynamic.js"],
      runAt: "document_idle",
      allFrames: false
    }]);
    const updated = await chrome.scripting.getRegisteredContentScripts({
      ids: [scriptId]
    });
    await chrome.scripting.unregisterContentScripts({ ids: [scriptId] });
    const afterUnregister = await chrome.scripting.getRegisteredContentScripts({
      ids: [scriptId]
    });
    await chrome.tabs.remove(tab.id);
    return { registered, response, updated, afterUnregister };
  }

  async function userScriptsRoundTrip() {
    const scriptId = "mori-user-script-smoke";
    const worldId = "moriUserWorld";
    await chrome.userScripts.unregister({ ids: [scriptId] }).catch(() => null);
    await chrome.userScripts.resetWorldConfiguration(worldId).catch(() => null);
    const before = await chrome.userScripts.getScripts({ ids: [scriptId] });

    await chrome.userScripts.configureWorld({
      worldId,
      messaging: true,
      csp: "script-src 'self'"
    });
    const worldConfigurations =
      await chrome.userScripts.getWorldConfigurations();

    const registeredValue = "registered-" + runId;
    await chrome.userScripts.register([{
      id: scriptId,
      matches: ["<all_urls>"],
      includeGlobs: ["*user-script*"],
      excludeGlobs: ["*excluded*"],
      js: [{
        code: "document.documentElement.dataset.moriUserScriptSmoke = " +
          JSON.stringify(registeredValue) + ";"
      }],
      runAt: "document_end",
      allFrames: false,
      world: chrome.userScripts.ExecutionWorld.USER_SCRIPT,
      worldId
    }]);
    const registered = await chrome.userScripts.getScripts({ ids: [scriptId] });
    const tab = await chrome.tabs.create({
      url: smokePageURL + "?user-script=" + encodeURIComponent(runId),
      active: true
    });
    const injected = await retry("userScripts registered injection", async () => {
      const result = await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        func: () => document.documentElement.dataset.moriUserScriptSmoke || ""
      });
      const value = result && result[0] && result[0].result || "";
      if (value !== registeredValue) throw new Error("user script not injected");
      return value;
    });

    const updatedValue = "updated-" + runId;
    await chrome.userScripts.update([{
      id: scriptId,
      js: [{
        code: "document.documentElement.dataset.moriUserScriptSmoke = " +
          JSON.stringify(updatedValue) + ";"
      }]
    }]);
    const updated = await browser.userScripts.getScripts({ ids: [scriptId] });
    const updateTab = await chrome.tabs.create({
      url: smokePageURL + "?user-script-updated=" + encodeURIComponent(runId),
      active: true
    });
    const injectedAfterUpdate = await retry("userScripts updated injection", async () => {
      const result = await chrome.scripting.executeScript({
        target: { tabId: updateTab.id },
        func: () => document.documentElement.dataset.moriUserScriptSmoke || ""
      });
      const value = result && result[0] && result[0].result || "";
      if (value !== updatedValue) throw new Error("updated user script not injected");
      return value;
    });

    const executeValue = "executed-" + runId;
    const executeResult = await chrome.userScripts.execute({
      target: { tabId: updateTab.id },
      js: [{
        code: "document.documentElement.dataset.moriUserScriptExecute = " +
          JSON.stringify(executeValue) + ";"
      }],
      world: chrome.userScripts.ExecutionWorld.USER_SCRIPT
    });
    const executeObserved = await retry("userScripts execute", async () => {
      const result = await chrome.scripting.executeScript({
        target: { tabId: updateTab.id },
        func: () => document.documentElement.dataset.moriUserScriptExecute || ""
      });
      const value = result && result[0] && result[0].result || "";
      if (value !== executeValue) throw new Error("userScripts.execute did not run");
      return value;
    });

    await chrome.userScripts.unregister({ ids: [scriptId] });
    const afterUnregister = await chrome.userScripts.getScripts({ ids: [scriptId] });
    await chrome.userScripts.resetWorldConfiguration(worldId);
    const worldConfigurationsAfterReset =
      await chrome.userScripts.getWorldConfigurations();
    await chrome.tabs.remove([tab.id, updateTab.id]).catch(() => null);

    return {
      namespace: !!(chrome.userScripts &&
        chrome.userScripts.register &&
        chrome.userScripts.getScripts &&
        chrome.runtime.onUserScriptMessage &&
        chrome.runtime.onUserScriptConnect),
      browserNamespace: !!(browser.userScripts &&
        browser.userScripts.getScripts),
      executionWorld: chrome.userScripts.ExecutionWorld,
      before,
      worldConfigurations,
      registered,
      injected,
      updated,
      injectedAfterUpdate,
      executeResult,
      executeObserved,
      afterUnregister,
      worldConfigurationsAfterReset
    };
  }

  async function webRequestRoundTrip() {
    const target = smokePageURL + "?webrequest=" + encodeURIComponent(runId);
    const targetURL = new URL(target);
    const targetPattern = targetURL.protocol.replace(":", "") +
      "://" + targetURL.hostname + "/*webrequest=*";
    const events = {
      beforeRequest: [],
      beforeSendHeaders: [],
      beforeSendHeadersWithoutExtra: [],
      headersReceived: [],
      headersReceivedWithoutExtra: [],
      completed: [],
      errors: [],
      nonMatching: [],
      removed: []
    };
    const filter = { urls: [targetPattern], types: ["main_frame"] };
    const removedListener = (details) => {
      events.removed.push(details);
    };
    chrome.webRequest.onBeforeRequest.addListener(removedListener, filter);
    const hasBeforeRemove =
      chrome.webRequest.onBeforeRequest.hasListener(removedListener);
    chrome.webRequest.onBeforeRequest.removeListener(removedListener);
    const hasAfterRemove =
      chrome.webRequest.onBeforeRequest.hasListener(removedListener);
    chrome.webRequest.onBeforeRequest.addListener((details) => {
      if (String(details.url || "") === target) events.beforeRequest.push(details);
    }, filter);
    chrome.webRequest.onBeforeRequest.addListener((details) => {
      events.nonMatching.push(details);
    }, { urls: ["*://example.invalid/*"] });
    chrome.webRequest.onBeforeSendHeaders.addListener((details) => {
      if (String(details.url || "") === target) events.beforeSendHeaders.push(details);
    }, filter, ["requestHeaders"]);
    chrome.webRequest.onBeforeSendHeaders.addListener((details) => {
      if (String(details.url || "") === target) {
        events.beforeSendHeadersWithoutExtra.push(details);
      }
    }, filter);
    chrome.webRequest.onHeadersReceived.addListener((details) => {
      if (String(details.url || "") === target) events.headersReceived.push(details);
    }, filter, ["responseHeaders"]);
    chrome.webRequest.onHeadersReceived.addListener((details) => {
      if (String(details.url || "") === target) {
        events.headersReceivedWithoutExtra.push(details);
      }
    }, filter);
    chrome.webRequest.onCompleted.addListener((details) => {
      if (String(details.url || "") === target) events.completed.push(details);
    }, filter);
    chrome.webRequest.onErrorOccurred.addListener((details) => {
      if (String(details.url || "") === target) events.errors.push(details);
    }, filter);

    const tab = await chrome.tabs.create({ url: target, active: true });
    await retry("webRequest events", () => {
      if (!events.beforeRequest.length ||
          !events.beforeSendHeaders.length ||
          !events.headersReceived.length ||
          !events.completed.length) {
        throw new Error("missing webRequest event");
      }
      return events;
    });
    await chrome.tabs.remove(tab.id);
    events.hasBeforeRemove = hasBeforeRemove;
    events.hasAfterRemove = hasAfterRemove;
    return events;
  }

  async function webNavigationRoundTrip() {
    const tab = await chrome.tabs.create({
      url: smokePageURL + "?webnav=" + encodeURIComponent(runId),
      active: true
    });
    const events = {
      history: [],
      fragment: []
    };
    chrome.webNavigation.onHistoryStateUpdated.addListener((details) => {
      if (details && details.tabId === tab.id) events.history.push(details);
    });
    chrome.webNavigation.onReferenceFragmentUpdated.addListener((details) => {
      if (details && details.tabId === tab.id) events.fragment.push(details);
    });
    await delay(700);
    const scriptResult = await withTimeout(chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: (value) => {
        history.pushState({ value }, "", "?spa=" + encodeURIComponent(value));
        location.hash = "fragment-" + encodeURIComponent(value);
        return location.href;
      },
      args: [runId]
    }), 3000);
    await retry("webNavigation SPA/hash events", () => {
      if (!events.history.some((event) =>
          String(event.url || "").includes("?spa=" + encodeURIComponent(runId))) ||
          !events.fragment.some((event) =>
            String(event.url || "").includes("#fragment-" + encodeURIComponent(runId)))) {
        throw new Error("missing webNavigation event");
      }
      return events;
    });
    await chrome.tabs.remove(tab.id);
    return { events, scriptResult };
  }

  async function declarativeNetRequestRoundTrip() {
    const ruleId = 9001;
    const redirectRuleId = 9002;
    const modifyHeadersRuleId = 9003;
    const responseHeadersRuleId = 9004;
    const blockedURL = smokePageURL + "?dnr-blocked=" + encodeURIComponent(runId);
    const redirectSourceURL = smokePageURL + "?dnr-source=" + encodeURIComponent(runId);
    const redirectTargetURL = smokePageURL + "?dnr-redirected=" + encodeURIComponent(runId);
    const headerURL = smokePageURL + "?dnr-headers=" + encodeURIComponent(runId);
    const responseHeaderURL = new URL(
      "/dnr-response?run=" + encodeURIComponent(runId),
      corsURL
    ).href;
    const observedErrors = [];
    const observedRedirects = [];
    const redirectCompletions = [];
    const observedHeaderRequests = [];
    const observedHeaderResponses = [];
    const headerValue = (headers, name) => {
      const header = (headers || []).find((item) =>
        String(item.name || "").toLowerCase() === name.toLowerCase());
      return header && header.value;
    };
    chrome.webRequest.onErrorOccurred.addListener((details) => {
      observedErrors.push(details);
    }, { urls: ["<all_urls>"] });
    chrome.webRequest.onBeforeRedirect.addListener((details) => {
      if (String(details.url || "") === redirectSourceURL) observedRedirects.push(details);
    }, { urls: ["<all_urls>"] });
    chrome.webRequest.onCompleted.addListener((details) => {
      if (String(details.url || "") === redirectTargetURL) redirectCompletions.push(details);
    }, { urls: ["<all_urls>"] });
    chrome.webRequest.onBeforeSendHeaders.addListener((details) => {
      if (String(details.url || "") === headerURL) observedHeaderRequests.push(details);
    }, { urls: ["<all_urls>"] }, ["requestHeaders"]);
    chrome.webRequest.onHeadersReceived.addListener((details) => {
      if (String(details.url || "") === responseHeaderURL) {
        observedHeaderResponses.push(details);
      }
    }, { urls: ["<all_urls>"] }, ["responseHeaders"]);

    await chrome.declarativeNetRequest.updateSessionRules({
      removeRuleIds: [
        ruleId,
        redirectRuleId,
        modifyHeadersRuleId,
        responseHeadersRuleId
      ],
      addRules: [{
        id: ruleId,
        priority: 1,
        action: { type: "block" },
        condition: {
          urlFilter: "dnr-blocked=" + runId,
          resourceTypes: ["main_frame"]
        }
      }, {
        id: redirectRuleId,
        priority: 1,
        action: {
          type: "redirect",
          redirect: { url: redirectTargetURL }
        },
        condition: {
          urlFilter: "dnr-source=" + runId,
          resourceTypes: ["main_frame"]
        }
      }, {
        id: modifyHeadersRuleId,
        priority: 1,
        action: {
          type: "modifyHeaders",
          requestHeaders: [{
            header: "X-Mori-DNR-Request",
            operation: "set",
            value: "request-" + runId
          }, {
            header: "Upgrade-Insecure-Requests",
            operation: "remove"
          }]
        },
        condition: {
          urlFilter: "dnr-headers=" + runId,
          resourceTypes: ["main_frame"]
        }
      }, {
        id: responseHeadersRuleId,
        priority: 1,
        action: {
          type: "modifyHeaders",
          responseHeaders: [{
            header: "X-Mori-DNR-Response",
            operation: "set",
            value: "response-" + runId
          }, {
            header: "X-Mori-DNR-Remove",
            operation: "remove"
          }]
        },
        condition: {
          urlFilter: "dnr-response?run=" + runId,
          resourceTypes: ["main_frame"]
        }
      }]
    });
    const rules = await chrome.declarativeNetRequest.getSessionRules({
      ruleIds: [
        ruleId,
        redirectRuleId,
        modifyHeadersRuleId,
        responseHeadersRuleId
      ]
    });
    const tab = await chrome.tabs.create({ url: blockedURL, active: true });
    await retry("DNR block", () => {
      if (!observedErrors.some((event) =>
          event &&
          event.error === "net::ERR_BLOCKED_BY_CLIENT" &&
          String(event.url || "").includes("dnr-blocked=" + encodeURIComponent(runId)))) {
        throw new Error("blocked request not observed " +
          JSON.stringify(observedErrors.slice(-5)));
      }
      return observedErrors;
    });
    await chrome.tabs.remove(tab.id);
    const redirectedTab = await chrome.tabs.create({ url: redirectSourceURL, active: true });
    await retry("DNR redirect", () => {
      if (!observedRedirects.some((event) =>
          event &&
          event.redirectUrl === redirectTargetURL) ||
          !redirectCompletions.some((event) =>
            event &&
            String(event.url || "") === redirectTargetURL)) {
        throw new Error("redirect request not observed");
      }
      return observedRedirects;
    });
    await chrome.tabs.remove(redirectedTab.id);
    const headerTab = await chrome.tabs.create({ url: headerURL, active: true });
    await retry("DNR modifyHeaders", () => {
      const request = observedHeaderRequests.find((event) =>
        headerValue(event.requestHeaders, "X-Mori-DNR-Request") ===
          "request-" + runId &&
        !headerValue(event.requestHeaders, "Upgrade-Insecure-Requests"));
      if (!request) {
        throw new Error("modified headers not observed " + JSON.stringify({
          requests: observedHeaderRequests.slice(-3)
        }));
      }
      return { request };
    });
    await chrome.tabs.remove(headerTab.id);
    const responseHeaderTab = await chrome.tabs.create({
      url: responseHeaderURL,
      active: true
    });
    await retry("DNR response modifyHeaders", () => {
      const response = observedHeaderResponses.find((event) =>
        headerValue(event.responseHeaders, "X-Mori-DNR-Response") ===
          "response-" + runId &&
        !headerValue(event.responseHeaders, "X-Mori-DNR-Remove"));
      if (!response) {
        throw new Error("modified response headers not observed " + JSON.stringify({
          responses: observedHeaderResponses.slice(-3)
        }));
      }
      return { response };
    });
    await chrome.tabs.remove(responseHeaderTab.id);
    await chrome.declarativeNetRequest.updateSessionRules({
      removeRuleIds: [
        ruleId,
        redirectRuleId,
        modifyHeadersRuleId,
        responseHeadersRuleId
      ]
    });
    const afterRemove = await chrome.declarativeNetRequest.getSessionRules({
      ruleIds: [
        ruleId,
        redirectRuleId,
        modifyHeadersRuleId,
        responseHeadersRuleId
      ]
    });
    return {
      rules,
      observedErrors,
      observedRedirects,
      redirectCompletions,
      observedHeaderRequests,
      observedHeaderResponses,
      afterRemove
    };
  }

  async function tabManagementRoundTrip() {
    const events = {
      created: [],
      activated: [],
      highlighted: [],
      moved: [],
      removed: []
    };
    chrome.tabs.onCreated.addListener((tab) => events.created.push(tab));
    chrome.tabs.onActivated.addListener((activeInfo) => events.activated.push(activeInfo));
    chrome.tabs.onHighlighted.addListener((highlightInfo) => events.highlighted.push(highlightInfo));
    chrome.tabs.onMoved.addListener((tabId, moveInfo) => events.moved.push({ tabId, moveInfo }));
    chrome.tabs.onRemoved.addListener((tabId, removeInfo) => events.removed.push({ tabId, removeInfo }));

    const managed = await chrome.tabs.create({
      url: smokePageURL + "?managed=" + encodeURIComponent(runId),
      active: false
    });
    const duplicated = await chrome.tabs.duplicate(managed.id);
    const moved = await chrome.tabs.move(duplicated.id, { index: 0 });
    await chrome.tabs.highlight({ tabs: [1] });
    const highlightedWindow = await chrome.tabs.highlight({ tabs: [0] });
    const activeTabs = await chrome.tabs.query({ active: true, currentWindow: true });
    await chrome.tabs.remove([managed.id, duplicated.id]);
    await delay(250);
    return {
      managed,
      duplicated,
      moved,
      highlightedWindow,
      activeAfterHighlight: activeTabs[0] || null,
      events
    };
  }

  async function tabGroupsRoundTrip() {
    const events = {
      created: [],
      updated: [],
      moved: [],
      removed: [],
      tabUpdated: []
    };
    chrome.tabGroups.onCreated.addListener((group) => events.created.push(group));
    chrome.tabGroups.onUpdated.addListener((group) => events.updated.push(group));
    chrome.tabGroups.onMoved.addListener((group) => events.moved.push(group));
    chrome.tabGroups.onRemoved.addListener((group) => events.removed.push(group));
    chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
      events.tabUpdated.push({ tabId, changeInfo, tab });
    });

    const first = await chrome.tabs.create({
      url: smokePageURL + "?group-a=" + encodeURIComponent(runId),
      active: false
    });
    const second = await chrome.tabs.create({
      url: smokePageURL + "?group-b=" + encodeURIComponent(runId),
      active: false
    });
    const groupId = await chrome.tabs.group({ tabIds: [first.id, second.id] });
    const group = await chrome.tabGroups.get(groupId);
    const firstAfterGroup = await chrome.tabs.get(first.id);
    const updated = await chrome.tabGroups.update(groupId, {
      title: "Mori Group " + runId,
      color: "cyan",
      collapsed: true
    });
    const queried = await chrome.tabGroups.query({
      title: "*Group " + runId,
      color: "cyan",
      collapsed: true
    });
    const moved = await chrome.tabGroups.move(groupId, { index: 0 });
    await chrome.tabs.ungroup(first.id);
    const firstAfterUngroup = await chrome.tabs.get(first.id);
    await chrome.tabs.ungroup(second.id);
    await delay(250);
    await chrome.tabs.remove([first.id, second.id]).catch(() => null);
    return {
      constants: {
        tabIdNone: chrome.tabs.TAB_ID_NONE,
        tabIndexNone: chrome.tabs.TAB_INDEX_NONE,
        tabGroupIdNone: chrome.tabGroups.TAB_GROUP_ID_NONE
      },
      first,
      second,
      groupId,
      group,
      firstAfterGroup,
      updated,
      queried,
      moved,
      firstAfterUngroup,
      events
    };
  }

  async function protectedGlobalRoundTrip() {
    const realChrome = globalThis.chrome;
    const realBrowser = globalThis.browser;
    const proxyReads = [];
    const protectedProxy = new Proxy(realChrome, {
      get(target, prop) {
        proxyReads.push(String(prop));
        if (prop === "app") return target[prop];
        return undefined;
      },
      set() {
        return false;
      }
    });
    globalThis.chrome = protectedProxy;
    globalThis.browser = protectedProxy;
    await retry("protected global restore", () => {
      if (!(globalThis.chrome === realChrome &&
          globalThis.browser === realBrowser &&
          globalThis.chrome &&
          globalThis.chrome.runtime &&
          globalThis.chrome.runtime.id === extensionId &&
          typeof globalThis.chrome.runtime.sendMessage === "function")) {
        throw new Error("chrome/browser globals still protected");
      }
      return true;
    });
    const response = await chrome.runtime.sendMessage(chrome.runtime.id, {
      type: "smoke-ping",
      runId,
      protectedGlobal: true
    });
    return {
      chromeStillReal: globalThis.chrome === realChrome,
      browserStillReal: globalThis.browser === realBrowser,
      runtimeId: globalThis.chrome && globalThis.chrome.runtime && globalThis.chrome.runtime.id,
      browserRuntimeId: globalThis.browser && globalThis.browser.runtime && globalThis.browser.runtime.id,
      sendMessageType: typeof globalThis.chrome.runtime.sendMessage,
      response,
      proxyReads
    };
  }

  async function contextMenusRoundTrip() {
    const events = [];
    chrome.contextMenus.onClicked.addListener(function(info, tab) {
      events.push({
        info,
        tab,
        argCount: arguments.length
      });
    });
    await chrome.contextMenus.removeAll();
    const normalId = "normal-" + runId;
    const checkboxId = "checkbox-" + runId;
    const createdNormalId = await chrome.contextMenus.create({
      id: normalId,
      title: "Mori selection %s",
      contexts: ["selection"]
    });
    const createdCheckboxId = await chrome.contextMenus.create({
      id: checkboxId,
      title: "Mori checkbox",
      type: "checkbox",
      checked: false,
      contexts: ["page"]
    });
    await chrome.contextMenus.update(normalId, {
      title: "Mori updated selection %s",
      enabled: true
    });
    const tab = await chrome.tabs.create({
      url: smokePageURL + "?contextmenu=" + encodeURIComponent(runId),
      active: true
    });
    await chrome.contextMenus.__moriSmokeClick({
      id: checkboxId,
      pageUrl: tab.url,
      selectionText: "selected smoke text",
      editable: false,
      frameId: 0,
      parentFrameId: -1,
      tabId: tab.id
    });
    const clicked = await retry("contextMenus.onClicked", () => {
      if (!events.length) throw new Error("context menu click not observed");
      return events[0];
    });
    await chrome.contextMenus.remove(normalId);
    await chrome.contextMenus.removeAll();
    await chrome.tabs.remove(tab.id);
    return {
      normalId,
      checkboxId,
      createdNormalId,
      createdCheckboxId,
      tab,
      clicked,
      events
    };
  }

  async function windowsRoundTrip() {
    const events = {
      created: [],
      removed: [],
      focusChanged: []
    };
    chrome.windows.onCreated.addListener((window) => events.created.push(window));
    chrome.windows.onRemoved.addListener((windowId) => events.removed.push(windowId));
    chrome.windows.onFocusChanged.addListener((windowId) => events.focusChanged.push(windowId));

    const current = await chrome.windows.getCurrent({ populate: true });
    const lastFocused = await chrome.windows.getLastFocused({ populate: true });
    const byId = await chrome.windows.get(chrome.windows.WINDOW_ID_CURRENT, { populate: true });
    const all = await chrome.windows.getAll({ populate: true });
    const created = await chrome.windows.create({
      url: smokePageURL + "?window=" + encodeURIComponent(runId),
      focused: true
    });
    const allAfterCreate = await chrome.windows.getAll({ populate: true });
    const focused = await chrome.windows.update(created.id, { focused: true });
    await chrome.windows.remove(created.id);
    const managedTab = created.tabs && created.tabs.find((tab) =>
      String(tab.url || "").includes("?window=" + encodeURIComponent(runId)));
    if (managedTab) {
      await chrome.tabs.remove(managedTab.id);
    }
    await delay(250);
    return {
      current,
      lastFocused,
      byId,
      all,
      allAfterCreate,
      created,
      focused,
      events
    };
  }

  async function identityRoundTrip() {
    const expected = chrome.identity.getRedirectURL(
      "oauth-smoke?code=" + encodeURIComponent(runId)
    );
    const result = await withTimeout(
      chrome.identity.launchWebAuthFlow({
        url: authPageURL,
        interactive: true
      }),
      5000
    );
    return { expected, result };
  }

  async function sidePanelRoundTrip() {
    await chrome.storage.local.remove([
      "sidePanelLoaded",
      "sidePanelRuntimeId",
      "sidePanelURL"
    ]);
    await chrome.sidePanel.setPanelBehavior({ openPanelOnActionClick: true });
    const behavior = await chrome.sidePanel.getPanelBehavior();
    await chrome.sidePanel.setOptions({
      path: "sidepanel.html",
      enabled: true
    });
    const options = await chrome.sidePanel.getOptions({});
    await chrome.sidePanel.open({});
    const loaded = await retry("sidePanel page", async () => {
      const stored = await chrome.storage.local.get([
        "sidePanelLoaded",
        "sidePanelRuntimeId",
        "sidePanelURL"
      ]);
      if (stored.sidePanelLoaded !== runId ||
          stored.sidePanelRuntimeId !== extensionId ||
          stored.sidePanelURL !== chrome.runtime.getURL("sidepanel.html")) {
        throw new Error("side panel did not render");
      }
      return stored;
    });
    await chrome.sidePanel.close({});
    return { behavior, options, loaded };
  }

  async function permissionsRoundTrip() {
    const optional = {
      permissions: ["notifications"],
      origins: ["https://example.com/*"]
    };
    await chrome.permissions.remove(optional);
    const before = await chrome.permissions.contains(optional);
    const added = [];
    const removed = [];
    chrome.permissions.onAdded.addListener((permissions) => added.push(permissions));
    chrome.permissions.onRemoved.addListener((permissions) => removed.push(permissions));

    const requested = await chrome.permissions.request(optional);
    const afterRequest = await chrome.permissions.contains(optional);
    const allAfterRequest = await chrome.permissions.getAll();
    const removedResult = await chrome.permissions.remove(optional);
    const afterRemove = await chrome.permissions.contains(optional);
    const allAfterRemove = await chrome.permissions.getAll();
    await delay(100);
    return {
      before,
      requested,
      afterRequest,
      allAfterRequest,
      removedResult,
      afterRemove,
      allAfterRemove,
      added,
      removed
    };
  }

  async function proxySettingsRoundTrip() {
    const changes = [];
    chrome.proxy.settings.onChange.addListener((details) => changes.push(details));
    await chrome.proxy.settings.clear({});
    const before = await chrome.proxy.settings.get({});
    const value = { mode: "direct" };
    await chrome.proxy.settings.set({ value, scope: "regular" });
    const afterSet = await chrome.proxy.settings.get({});
    await chrome.proxy.settings.clear({});
    const afterClear = await chrome.proxy.settings.get({});
    await delay(100);
    return {
      before,
      afterSet,
      afterClear,
      changes
    };
  }

  async function privacyRoundTrip() {
    const passwordChanges = [];
    chrome.privacy.services.passwordSavingEnabled.onChange.addListener(
      (details) => passwordChanges.push(details));
    await chrome.privacy.services.passwordSavingEnabled.clear({});
    await chrome.privacy.network.webRTCIPHandlingPolicy.clear({});
    await chrome.privacy.websites.topicsEnabled.clear({});
    const passwordBefore =
      await chrome.privacy.services.passwordSavingEnabled.get({});
    const passwordCallbackBefore = await new Promise((resolve) => {
      chrome.privacy.services.passwordSavingEnabled.get({}, (details) =>
        resolve(details));
    });
    await chrome.privacy.services.passwordSavingEnabled.set({
      value: false,
      scope: "regular"
    });
    const passwordAfterSet =
      await chrome.privacy.services.passwordSavingEnabled.get({});
    await chrome.privacy.services.passwordSavingEnabled.clear({});
    const passwordAfterClear =
      await chrome.privacy.services.passwordSavingEnabled.get({});

    const webRTCPolicy =
      chrome.privacy.IPHandlingPolicy.DISABLE_NON_PROXIED_UDP;
    await chrome.privacy.network.webRTCIPHandlingPolicy.set({
      value: webRTCPolicy,
      scope: "regular"
    });
    const webRTCAfterSet =
      await chrome.privacy.network.webRTCIPHandlingPolicy.get({});
    await chrome.privacy.network.webRTCIPHandlingPolicy.clear({});
    const webRTCAfterClear =
      await chrome.privacy.network.webRTCIPHandlingPolicy.get({});

    let topicsEnableError = "";
    try {
      await chrome.privacy.websites.topicsEnabled.set({ value: true });
    } catch (error) {
      topicsEnableError = error && error.message || String(error);
    }
    const browserThirdPartyCookies =
      await browser.privacy.websites.thirdPartyCookiesAllowed.get({});
    await delay(100);
    return {
      namespace: !!(chrome.privacy &&
        chrome.privacy.network &&
        chrome.privacy.services &&
        chrome.privacy.websites &&
        chrome.privacy.services.passwordSavingEnabled &&
        chrome.privacy.services.passwordSavingEnabled.onChange),
      browserNamespace: !!(browser.privacy &&
        browser.privacy.websites &&
        browser.privacy.websites.thirdPartyCookiesAllowed),
      ipHandlingPolicyEnum: chrome.privacy.IPHandlingPolicy,
      passwordBefore,
      passwordCallbackBefore,
      passwordAfterSet,
      passwordAfterClear,
      passwordChanges,
      webRTCAfterSet,
      webRTCAfterClear,
      browserThirdPartyCookies,
      topicsEnableError
    };
  }

  async function contentSettingsRoundTrip() {
    const primary = "https://content.example.com/path/" +
      encodeURIComponent(runId);
    const secondary = "https://top.example.org/frame/" +
      encodeURIComponent(runId);
    const allPrimary = "https://elsewhere.test/path";
    await chrome.contentSettings.cookies.clear({});
    await chrome.contentSettings.images.clear({});
    await chrome.contentSettings.notifications.clear({});

    const cookieBefore = await chrome.contentSettings.cookies.get({
      primaryUrl: primary,
      secondaryUrl: secondary
    });
    const cookieCallbackBefore = await new Promise((resolve) => {
      chrome.contentSettings.cookies.get({
        primaryUrl: primary,
        secondaryUrl: secondary
      }, (details) => resolve(details));
    });

    await chrome.contentSettings.cookies.set({
      primaryPattern: "https://*.example.com/*",
      secondaryPattern: "<all_urls>",
      setting: "block"
    });
    await chrome.contentSettings.cookies.set({
      primaryPattern: "https://content.example.com/*",
      secondaryPattern: "https://top.example.org/*",
      setting: "session_only"
    });
    const cookieAfterSpecific = await chrome.contentSettings.cookies.get({
      primaryUrl: primary,
      secondaryUrl: secondary
    });
    const cookieAfterWildcard = await chrome.contentSettings.cookies.get({
      primaryUrl: "https://other.example.com/path",
      secondaryUrl: secondary
    });

    const imageCallback = await new Promise((resolve) => {
      browser.contentSettings.images.set({
        primaryPattern: "<all_urls>",
        setting: "block"
      }, () => {
        browser.contentSettings.images.get({
          primaryUrl: allPrimary
        }, (details) => resolve({
          details,
          lastError: chrome.runtime.lastError &&
            chrome.runtime.lastError.message || ""
        }));
      });
    });
    await browser.contentSettings.images.clear({});
    const imageAfterClear = await chrome.contentSettings.images.get({
      primaryUrl: allPrimary
    });

    await chrome.contentSettings.notifications.set({
      primaryPattern: "https://notify.example.com/*",
      setting: "allow",
      scope: "incognito_session_only"
    });
    const notificationRegular = await chrome.contentSettings.notifications.get({
      primaryUrl: "https://notify.example.com/path"
    });
    const notificationIncognito = await chrome.contentSettings.notifications.get({
      primaryUrl: "https://notify.example.com/path",
      incognito: true
    });
    const notificationResources =
      await chrome.contentSettings.notifications.getResourceIdentifiers();
    const pluginResources =
      await chrome.contentSettings.plugins.getResourceIdentifiers();
    const pluginFixed = await chrome.contentSettings.plugins.get({
      primaryUrl: primary
    });
    const fullscreenFixed = await chrome.contentSettings.fullscreen.get({
      primaryUrl: primary
    });

    let invalidError = "";
    try {
      await chrome.contentSettings.javascript.set({
        primaryPattern: "<all_urls>",
        setting: "ask"
      });
    } catch (error) {
      invalidError = error && error.message || String(error);
    }

    await chrome.contentSettings.cookies.clear({});
    await chrome.contentSettings.notifications.clear({
      scope: "incognito_session_only"
    });

    return {
      namespace: !!(chrome.contentSettings &&
        chrome.contentSettings.cookies &&
        chrome.contentSettings.javascript),
      browserNamespace: !!(browser.contentSettings &&
        browser.contentSettings.images),
      cookieBefore,
      cookieCallbackBefore,
      cookieAfterSpecific,
      cookieAfterWildcard,
      imageCallback,
      imageAfterClear,
      notificationRegular,
      notificationIncognito,
      notificationResources,
      pluginResources,
      pluginFixed,
      fullscreenFixed,
      invalidError
    };
  }

  async function historyRoundTrip() {
    const url = smokePageURL + "?history=" + encodeURIComponent(runId);
    const title = "Mori History " + runId;
    const visited = [];
    const removed = [];
    await chrome.history.deleteUrl({ url });
    chrome.history.onVisited.addListener((item) => visited.push(item));
    chrome.history.onVisitRemoved.addListener((info) => removed.push(info));

    await chrome.history.addUrl({ url, title });
    const search = await retry("history.search", async () => {
      const result = await chrome.history.search({
        text: runId,
        maxResults: 25
      });
      if (!Array.isArray(result) ||
          !result.some((item) =>
            item &&
            item.url === url &&
            item.title === title &&
            Number(item.visitCount) >= 1)) {
        throw new Error("history item missing");
      }
      return result;
    });
    const visits = await chrome.history.getVisits({ url });
    const topSites = await chrome.topSites.get();
    await chrome.history.deleteUrl({ url });
    const afterDelete = await chrome.history.search({
      text: runId,
      maxResults: 25
    });
    await delay(100);
    return {
      url,
      title,
      search,
      visits,
      topSites,
      afterDelete,
      visited,
      removed
    };
  }

  async function cookiesRoundTrip() {
    const url = "https://example.com/mori-cookie-smoke";
    const name = "mori_cookie_" + runId.replace(/[^a-zA-Z0-9_]/g, "_");
    const value = "value-" + runId;
    const changed = [];
    await chrome.cookies.remove({ url, name });
    chrome.cookies.onChanged.addListener((change) => changed.push(change));

    const setCookie = await chrome.cookies.set({
      url,
      name,
      value,
      path: "/"
    });
    const getCookie = await retry("cookies.get", async () => {
      const result = await chrome.cookies.get({ url, name });
      if (!result || result.name !== name || result.value !== value) {
        throw new Error("cookie not visible");
      }
      return result;
    });
    const allCookies = await chrome.cookies.getAll({
      domain: "example.com",
      name
    });
    const stores = await chrome.cookies.getAllCookieStores();
    const removedCookie = await chrome.cookies.remove({ url, name });
    const afterRemove = await retry("cookies.remove", async () => {
      const result = await chrome.cookies.get({ url, name });
      if (result) {
        throw new Error("cookie still visible");
      }
      return result;
    });
    await delay(100);
    return {
      url,
      name,
      value,
      setCookie,
      getCookie,
      allCookies,
      stores,
      removedCookie,
      afterRemove,
      changed
    };
  }

  async function browsingDataRoundTrip() {
    const historyUrl = smokePageURL + "?browsing-data=" + encodeURIComponent(runId);
    const historyTitle = "Mori Browsing Data " + runId;
    const cookieUrl = "https://example.com/mori-browsing-data-smoke";
    const cookieName = "mori_browsing_data_" + runId.replace(/[^a-zA-Z0-9_]/g, "_");
    const cookieValue = "value-" + runId;
    await chrome.history.deleteUrl({ url: historyUrl });
    await chrome.cookies.remove({ url: cookieUrl, name: cookieName });
    await chrome.history.addUrl({ url: historyUrl, title: historyTitle });
    await chrome.cookies.set({
      url: cookieUrl,
      name: cookieName,
      value: cookieValue,
      path: "/"
    });

    const settings = await chrome.browsingData.settings();
    await retry("browsingData before remove", async () => {
      const history = await chrome.history.search({ text: runId, maxResults: 25 });
      const cookie = await chrome.cookies.get({ url: cookieUrl, name: cookieName });
      if (!Array.isArray(history) ||
          !history.some((item) => item && item.url === historyUrl) ||
          !cookie ||
          cookie.value !== cookieValue) {
        throw new Error("seeded browsing data missing");
      }
      return true;
    });
    await chrome.browsingData.remove({ since: 0 }, {
      cookies: true,
      history: true
    });
    const historyAfter = await chrome.history.search({ text: runId, maxResults: 25 });
    const cookieAfter = await chrome.cookies.get({ url: cookieUrl, name: cookieName });
    return {
      historyUrl,
      cookieName,
      settings,
      historyAfter,
      cookieAfter
    };
  }

  async function downloadsRoundTrip() {
    const events = {
      created: [],
      changed: [],
      erased: []
    };
    chrome.downloads.onCreated.addListener((item) => events.created.push(item));
    chrome.downloads.onChanged.addListener((delta) => events.changed.push(delta));
    chrome.downloads.onErased.addListener((downloadId) => events.erased.push(downloadId));

    const downloadId = await chrome.downloads.download({
      url: slowDownloadURL,
      filename: "mori-smoke-" + runId + ".bin"
    });
    await retry("downloads.created", async () => {
      const matches = await chrome.downloads.search({ id: downloadId });
      if (!Array.isArray(matches) ||
          !matches.some((item) => item && item.id === downloadId)) {
        throw new Error("download not visible");
      }
      return matches;
    });
    await chrome.downloads.cancel(downloadId);
    const afterCancel = await retry("downloads.cancel", async () => {
      const matches = await chrome.downloads.search({ id: downloadId });
      const item = matches && matches[0];
      if (!item || item.state !== "interrupted") {
        throw new Error("download not canceled");
      }
      return item;
    });
    const erased = await chrome.downloads.erase({ id: downloadId });
    await delay(100);
    return {
      downloadId,
      afterCancel,
      erased,
      events
    };
  }

  async function sessionsRoundTrip() {
    const url = smokePageURL + "?session=" + encodeURIComponent(runId);
    const changed = [];
    chrome.sessions.onChanged.addListener(() => changed.push(Date.now()));

    const tab = await chrome.tabs.create({ url, active: true });
    await delay(300);
    await chrome.tabs.remove(tab.id);
    const recentlyClosed = await retry("sessions.getRecentlyClosed", async () => {
      const result = await chrome.sessions.getRecentlyClosed({ maxResults: 10 });
      if (!Array.isArray(result) ||
          !result.some((session) =>
            session &&
            session.tab &&
            session.tab.url === url &&
            session.tab.sessionId)) {
        throw new Error("recently closed tab not visible");
      }
      return result;
    });
    const target = recentlyClosed.find((session) =>
      session && session.tab && session.tab.url === url);
    const restored = await chrome.sessions.restore(target.tab.sessionId);
    await delay(200);
    await chrome.tabs.remove(restored.tab.id);
    const devices = await chrome.sessions.getDevices({ maxResults: 5 });
    return {
      url,
      tab,
      recentlyClosed,
      restored,
      devices,
      changed
    };
  }

  async function managementRoundTrip() {
    const self = await chrome.management.getSelf();
    const companionBefore = await chrome.management.get(companionExtensionId);
    const allBefore = await chrome.management.getAll();
    await chrome.management.setEnabled(companionExtensionId, false);
    const companionDisabled = await chrome.management.get(companionExtensionId);
    await chrome.management.setEnabled(companionExtensionId, true);
    const companionEnabled = await chrome.management.get(companionExtensionId);
    const uninstallURL = "https://example.com/mori-uninstall-" + runId;
    const lifecycle = await retry("runtime.setUninstallURL companion", () =>
      chrome.runtime.sendMessage(companionExtensionId, {
        type: "lifecycle-smoke",
        uninstallURL
      }).then((result) => {
        if (!result ||
            result.uninstallURLSet !== true ||
            result.runtimeId !== companionExtensionId) {
          throw new Error("companion did not set uninstall URL");
        }
        return result;
      }));
    const companionAfterUninstall = await retry("management.uninstallSelf companion", async () => {
      const result = await chrome.management.get(companionExtensionId);
      if (result !== null) {
        throw new Error("companion extension still installed");
      }
      return result;
    });
    const uninstallTab = await retry("runtime.setUninstallURL opened Mori tab", async () => {
      const tabs = await chrome.tabs.query({});
      const tab = tabs.find((item) => item && item.url === uninstallURL);
      if (!tab) {
        throw new Error("uninstall URL tab not opened");
      }
      return tab;
    });
    await chrome.tabs.remove(uninstallTab.id);
    return {
      self,
      companionBefore,
      allBefore,
      companionDisabled,
      companionEnabled,
      uninstallURL,
      lifecycle,
      companionAfterUninstall,
      uninstallTab
    };
  }

  async function nativeMessagingRoundTrip() {
    const single = await chrome.runtime.sendNativeMessage("com.mori.smoke", {
      runId,
      message: "native-message-smoke",
      nested: { value: 42 },
      mode: "single"
    });
    const chromeCompatible = await chrome.runtime.sendNativeMessage(
      "com.mori.smoke_chrome",
      {
        runId,
        message: "native-message-chrome-origin-smoke",
        mode: "chrome-compatible"
      }
    );
    const port = chrome.runtime.connectNative("com.mori.smoke");
    const portMessages = [];
    const disconnects = [];
    port.onMessage.addListener((message) => portMessages.push(message));
    port.onDisconnect.addListener(() => disconnects.push(Date.now()));
    port.postMessage({
      runId,
      message: "native-port-smoke",
      nested: { value: 84 },
      mode: "port"
    });
    const portMessage = await retry("runtime.connectNative response", () => {
      if (!portMessages.length) {
        throw new Error("native port did not respond");
      }
      return portMessages[0];
    });
    port.disconnect();
    await retry("runtime.connectNative disconnect", () => {
      if (!disconnects.length) {
        throw new Error("native port did not disconnect");
      }
      return disconnects.slice();
    });
    return {
      single,
      chromeCompatible,
      portMessage,
      disconnectCount: disconnects.length
    };
  }

  async function crxDownloadInstallRoundTrip() {
    const before = await chrome.management.get(crxExtensionId);
    const downloadId = await chrome.downloads.download({
      url: crxURL,
      filename: "extension.crx",
      conflictAction: "overwrite"
    });
    const installed = await retry("CRX download installed into Mori", async () => {
      const ext = await chrome.management.get(crxExtensionId);
      if (!ext ||
          ext.id !== crxExtensionId ||
          ext.name !== "Mori CRX Download Smoke" ||
          !String(ext.description || "").includes(runId) ||
          ext.enabled !== true) {
        throw new Error("CRX extension not installed yet");
      }
      return ext;
    });
    const legacyURL = "chrome-" + "extension://" + crxExtensionId + "/popup.html";
    const expectedURL = "mori-extension://" + crxExtensionId + "/popup.html";
    const legacyTab = await chrome.tabs.create({ url: legacyURL, active: true });
    const legacyScheme = await retry("legacy extension scheme rewrote to Mori", async () => {
      const tab = await chrome.tabs.get(legacyTab.id);
      if (!tab || tab.url !== expectedURL) {
        throw new Error("legacy extension URL was not rewritten into Mori");
      }
      return {
        tabId: legacyTab.id,
        requested: legacyURL,
        url: tab.url,
        expected: expectedURL
      };
    });
    await chrome.tabs.remove(legacyTab.id);
    await chrome.management.uninstall(crxExtensionId, { showConfirmDialog: false });
    const afterUninstall = await retry("CRX smoke extension uninstalled", async () => {
      const ext = await chrome.management.get(crxExtensionId);
      if (ext) {
        throw new Error("CRX smoke extension still installed");
      }
      return ext;
    });
    return { before, downloadId, installed, legacyScheme, afterUninstall };
  }

  async function offscreenRoundTrip() {
    await chrome.storage.local.remove([
      "offscreenLoaded",
      "offscreenRuntimeId",
      "offscreenURL",
      "offscreenDocumentHidden"
    ]);
    const beforeHasDocument = await chrome.offscreen.hasDocument();
    await chrome.offscreen.createDocument({
      url: "offscreen.html",
      reasons: [chrome.offscreen.Reason.DOM_PARSER],
      justification: "Mori extension smoke offscreen document"
    });
    const loaded = await retry("offscreen document", async () => {
      const stored = await chrome.storage.local.get([
        "offscreenLoaded",
        "offscreenRuntimeId",
        "offscreenURL",
        "offscreenDocumentHidden"
      ]);
      if (!stored ||
          stored.offscreenLoaded !== runId ||
          stored.offscreenRuntimeId !== extensionId ||
          stored.offscreenURL !== chrome.runtime.getURL("offscreen.html")) {
        throw new Error("offscreen document did not report ready");
      }
      return stored;
    });
    const afterCreateHasDocument = await chrome.offscreen.hasDocument();
    const contexts = await chrome.runtime.getContexts({
      contextTypes: [chrome.runtime.ContextType.OFFSCREEN_DOCUMENT],
      documentUrls: [chrome.runtime.getURL("offscreen.html")]
    });
    const clipboardValue = "mori-offscreen-clipboard-" + runId;
    const clipboard = await retry("offscreen clipboard", async () => {
      const result = await chrome.runtime.sendMessage({
        type: "clipboard-smoke",
        value: clipboardValue
      });
      if (!result || result.error) {
        throw new Error(result && result.error || "offscreen clipboard did not respond");
      }
      if (result.asyncRead !== clipboardValue ||
          result.finalRead !== clipboardValue ||
          result.pasted !== clipboardValue) {
        throw new Error("offscreen clipboard round-trip mismatch: " +
          JSON.stringify(result));
      }
      return result;
    });
    await chrome.offscreen.closeDocument();
    const afterCloseHasDocument = await retry("offscreen document closed", async () => {
      const result = await chrome.offscreen.hasDocument();
      if (result !== false) {
        throw new Error("offscreen document still open");
      }
      return result;
    });
    return {
      beforeHasDocument,
      afterCreateHasDocument,
      afterCloseHasDocument,
      contexts,
      clipboardValue,
      clipboard,
      loaded
    };
  }

  async function browserNamespaceRoundTrip() {
    const b = globalThis.browser;
    if (!b) {
      throw new Error("browser namespace missing");
    }
    const key = "browserNamespaceSmoke";
    await b.storage.local.set({ [key]: runId });
    const stored = await b.storage.local.get([key]);
    const popupURL = b.runtime.getURL("popup.html");
    const popupContexts = await b.runtime.getContexts({
      contextTypes: [b.runtime.ContextType.POPUP],
      documentUrls: [popupURL]
    });
    const activeTabs = await b.tabs.query({ active: true, currentWindow: true });
    const self = await b.management.getSelf();
    const hasStoragePermission = await b.permissions.contains({
      permissions: ["storage"]
    });
    const language = b.i18n.getUILanguage();
    return {
      runtimeId: b.runtime.id,
      popupURL,
      popupContexts,
      activeTabs,
      self,
      stored,
      hasStoragePermission,
      language,
      apiShape: {
        storageSet: typeof b.storage.local.set === "function",
        tabsQuery: typeof b.tabs.query === "function",
        managementGetSelf: typeof b.management.getSelf === "function",
        permissionsContains: typeof b.permissions.contains === "function"
      }
    };
  }

  async function webAccessibleResourcesRoundTrip() {
    const tab = await chrome.tabs.create({
      url: smokePageURL + "?war=" + encodeURIComponent(runId),
      active: true
    });
    await delay(700);
    const result = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      args: [
        chrome.runtime.getURL("public.txt"),
        chrome.runtime.getURL("private.txt")
      ],
      func: async (publicURL, privateURL) => {
        const publicResponse = await fetch(publicURL);
        const publicText = await publicResponse.text();
        let privateResult = null;
        try {
          const privateResponse = await fetch(privateURL);
          privateResult = {
            ok: privateResponse.ok,
            status: privateResponse.status,
            text: await privateResponse.text()
          };
        } catch (error) {
          privateResult = {
            ok: false,
            error: error && error.message || String(error)
          };
        }
        return {
          publicStatus: publicResponse.status,
          publicText,
          privateResult
        };
      }
    });
    await chrome.tabs.remove(tab.id);
    return result;
  }

  async function notificationsRoundTrip() {
    const notificationId = "mori-notification-" + runId;
    const popupClosedEvents = [];
    chrome.notifications.onClosed.addListener((id, byUser) => {
      popupClosedEvents.push({ id, byUser });
    });
    await chrome.notifications.clear(notificationId);
    const permission = await chrome.notifications.getPermissionLevel();
    const createdId = await chrome.notifications.create(notificationId, {
      type: "basic",
      title: "Mori smoke",
      message: "created-" + runId
    });
    const updated = await chrome.notifications.update(notificationId, {
      message: "updated-" + runId
    });
    const popupAll = await chrome.notifications.getAll();
    const backgroundBeforeClear = await retry("notifications background getAll", () =>
      chrome.runtime.sendMessage(chrome.runtime.id, {
        type: "notification-smoke-state",
        runId
      }).then((state) => {
        if (!state || !state.all || !state.all[notificationId]) {
          throw new Error("background cannot see notification");
        }
        return state;
      }));
    const cleared = await chrome.notifications.clear(notificationId);
    await delay(100);
    const popupAfterClear = await chrome.notifications.getAll();
    const backgroundAfterClear = await retry("notifications background closed", () =>
      chrome.runtime.sendMessage(chrome.runtime.id, {
        type: "notification-smoke-state",
        runId
      }).then((state) => {
        if (!state ||
            !Array.isArray(state.closedEvents) ||
            !state.closedEvents.some((event) =>
              event &&
              event.notificationId === notificationId &&
              event.byUser === false)) {
          throw new Error("background did not observe notification close");
        }
        return state;
      }));
    return {
      notificationId,
      permission,
      createdId,
      updated,
      popupAll,
      backgroundBeforeClear,
      cleared,
      popupAfterClear,
      popupClosedEvents,
      backgroundAfterClear
    };
  }

  async function alarmsRoundTrip() {
    const popupName = "mori-popup-alarm-" + runId;
    const backgroundName = "mori-background-alarm-" + runId;
    await chrome.alarms.clearAll();
    chrome.alarms.create(popupName, { when: Date.now() + 300000 });
    const popupCreated = await chrome.alarms.get(popupName);
    const popupAll = await chrome.alarms.getAll();
    const popupCleared = await chrome.alarms.clear(popupName);
    const popupMissingCleared = await chrome.alarms.clear(popupName);
    const backgroundStarted = await retry("alarms background create", () =>
      chrome.runtime.sendMessage(chrome.runtime.id, {
        type: "alarm-smoke-start",
        name: backgroundName,
        runId
      }).then((state) => {
        if (!state ||
            !state.created ||
            state.created.name !== backgroundName ||
            !Array.isArray(state.all) ||
            !state.all.some((alarm) => alarm && alarm.name === backgroundName)) {
          throw new Error("background alarm was not created");
        }
        return state;
      }));
    const backgroundFired = await retry("alarms background onAlarm", () =>
      chrome.runtime.sendMessage(chrome.runtime.id, {
        type: "alarm-smoke-state",
        name: backgroundName,
        runId
      }).then((state) => {
        if (!state ||
            !Array.isArray(state.events) ||
            !state.events.some((alarm) => alarm && alarm.name === backgroundName)) {
          throw new Error("background alarm did not fire");
        }
        return state;
      }));
    return {
      popupCreated,
      popupAll,
      popupCleared,
      popupMissingCleared,
      backgroundStarted,
      backgroundFired
    };
  }

  async function bookmarksRoundTrip() {
    const firstURL = "https://example.com/mori-bookmark-first-" + runId;
    const secondURL = "https://example.com/mori-bookmark-second-" + runId;
    const updatedURL = firstURL + "?updated=1";
    const events = { created: [], changed: [], moved: [], removed: [] };
    chrome.bookmarks.onCreated.addListener((id, node) => events.created.push({ id, node }));
    chrome.bookmarks.onChanged.addListener((id, changeInfo) => events.changed.push({ id, changeInfo }));
    chrome.bookmarks.onMoved.addListener((id, moveInfo) => events.moved.push({ id, moveInfo }));
    chrome.bookmarks.onRemoved.addListener((id, removeInfo) => events.removed.push({ id, removeInfo }));

    const first = await chrome.bookmarks.create({
      parentId: "1",
      index: 0,
      title: "Mori Bookmark First " + runId,
      url: firstURL
    });
    const second = await chrome.bookmarks.create({
      parentId: "1",
      index: 0,
      title: "Mori Bookmark Second " + runId,
      url: secondURL
    });
    const moved = await chrome.bookmarks.move(first.id, {
      parentId: "1",
      index: 0
    });
    const updated = await chrome.bookmarks.update(first.id, {
      title: "Mori Bookmark Updated " + runId,
      url: updatedURL
    });
    const fetched = await chrome.bookmarks.get(first.id);
    const children = await chrome.bookmarks.getChildren("1");
    const tree = await chrome.bookmarks.getTree();
    const search = await chrome.bookmarks.search({ query: runId });
    await chrome.bookmarks.remove(second.id);
    await chrome.bookmarks.remove(first.id);
    return {
      first,
      second,
      moved,
      updated,
      fetched,
      children,
      tree,
      search,
      events
    };
  }

  async function captureVisibleTabRoundTrip() {
    const tab = await chrome.tabs.create({ url: capturePageURL, active: true });
    await delay(800);
    const dataUrl = await retry("tabs.captureVisibleTab", async () => {
      const result = await chrome.tabs.captureVisibleTab({ format: "png" });
      if (typeof result !== "string" || !result.startsWith("data:image/png;base64,")) {
        throw new Error("capture did not return a PNG data URL");
      }
      if (result === "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/luzZqAAAAABJRU5ErkJggg==") {
        throw new Error("capture returned the old 1x1 fake PNG");
      }
      return result;
    });
    const imageInfo = await new Promise((resolve, reject) => {
      const image = new Image();
      image.onload = () => {
        try {
          const canvas = document.createElement("canvas");
          canvas.width = image.naturalWidth;
          canvas.height = image.naturalHeight;
          const ctx = canvas.getContext("2d");
          ctx.drawImage(image, 0, 0);
          const x = Math.floor(image.naturalWidth * 0.75);
          const y = Math.floor(image.naturalHeight * 0.75);
          const pixel = Array.from(ctx.getImageData(x, y, 1, 1).data);
          resolve({
            width: image.naturalWidth,
            height: image.naturalHeight,
            sample: pixel,
            byteLength: Math.floor((dataUrl.length - "data:image/png;base64,".length) * 3 / 4)
          });
        } catch (error) {
          reject(error);
        }
      };
      image.onerror = () => reject(new Error("capture data URL did not decode"));
      image.src = dataUrl;
    });
    await chrome.tabs.remove(tab.id);
    return { tab, dataUrlPrefix: dataUrl.slice(0, 32), imageInfo };
  }

  async function legacyExtensionViewsRoundTrip() {
    const allViews = chrome.extension.getViews();
    const popupViews = chrome.extension.getViews({ type: "popup" });
    const tabViews = chrome.extension.getViews({ type: "tab" });
    const extensionTabs = chrome.extension.getExtensionTabs();
    const backgroundFromPopup = chrome.extension.getBackgroundPage();
    const runtimeBackgroundFromPopup = await chrome.runtime.getBackgroundPage();
    const background = await retry("legacy extension background views", () =>
      chrome.runtime.sendMessage(chrome.runtime.id, {
        type: "legacy-extension-views-smoke"
      }));
    return {
      namespace: !!chrome.extension,
      inIncognitoContext: chrome.extension.inIncognitoContext,
      getURL: chrome.extension.getURL("popup.html"),
      allViewsIncludeSelf: allViews.includes(globalThis),
      allViewsLength: allViews.length,
      popupViewsIncludeSelf: popupViews.includes(globalThis),
      popupViewsLength: popupViews.length,
      tabViewsLength: tabViews.length,
      extensionTabsLength: extensionTabs.length,
      backgroundFromPopupIsMissing: backgroundFromPopup == null,
      runtimeBackgroundFromPopupIsMissing: runtimeBackgroundFromPopup == null,
      background
    };
  }

  try {
	    const checks = {
	      runtimeId: chrome.runtime.id === extensionId,
	      onMessageEvent: !!(chrome.runtime.onMessage && chrome.runtime.onMessage.addListener),
	      browserRuntimeAlias: !!(globalThis.browser &&
	        browser.runtime &&
	        browser.runtime.id === extensionId &&
	        browser.runtime.onMessage &&
	        browser.runtime.onMessage.addListener &&
	        typeof browser.runtime.sendMessage === "function"),
	      earlyNavigatorOnline: window.__moriEarlyNavigatorOnline === true,
	      navigatorOnline: navigator.onLine === true,
	      getURL: chrome.runtime.getURL("popup.html") ===
	        "mori-extension://" + extensionId + "/popup.html"
	    };
    const contexts = await chrome.runtime.getContexts({
      contextTypes: [chrome.runtime.ContextType.POPUP],
      documentUrls: [chrome.runtime.getURL("popup.html")]
    });
    const backgroundContexts = await retry("background context", async () => {
      const result = await chrome.runtime.getContexts({
        contextTypes: [chrome.runtime.ContextType.BACKGROUND],
        documentUrls: [chrome.runtime.getURL("__mori_background__.html")]
      });
      if (!Array.isArray(result) ||
          !result.some((context) =>
            context &&
            context.contextType === chrome.runtime.ContextType.BACKGROUND &&
            context.documentUrl === chrome.runtime.getURL("__mori_background__.html"))) {
        throw new Error("background context not visible");
      }
      return result;
    });
    const legacyExtensionViewsResponse = await legacyExtensionViewsRoundTrip();
    const storageEvents = [];
    chrome.storage.onChanged.addListener((changes, areaName) => {
      storageEvents.push({ changes, areaName });
    });
    await chrome.storage.local.clear();
    await chrome.storage.local.setAccessLevel({
      accessLevel: "TRUSTED_AND_UNTRUSTED_CONTEXTS"
    });
    await chrome.storage.local.set({ smokeKey: runId, count: 1 });
    const stored = await chrome.storage.local.get(["smokeKey", "count", "missing"]);
    const storageKeys = await chrome.storage.local.getKeys();
    const storageBytes = await chrome.storage.local.getBytesInUse(["smokeKey", "count"]);
    const storageConstants = {
      localQuota: chrome.storage.local.QUOTA_BYTES,
      syncQuota: chrome.storage.sync.QUOTA_BYTES,
      syncItemQuota: chrome.storage.sync.QUOTA_BYTES_PER_ITEM,
      syncMaxItems: chrome.storage.sync.MAX_ITEMS,
      sessionQuota: chrome.storage.session.QUOTA_BYTES
    };
    const storageAreasResponse = await storageAreasRoundTrip();
    const userSettings = await chrome.action.getUserSettings();
    await chrome.action.setBadgeText({ text: "OK" });
    await chrome.action.setBadgeBackgroundColor({ color: [18, 122, 255, 255] });
    await chrome.action.setBadgeTextColor({ color: "#102030" });
    const badgeText = await chrome.action.getBadgeText({});
    const badgeBackgroundColor = await chrome.action.getBadgeBackgroundColor({});
    const badgeTextColor = await chrome.action.getBadgeTextColor({});
    await chrome.action.disable();
    const actionDisabled = await chrome.action.isEnabled({});
    await chrome.action.enable();
    const actionEnabled = await chrome.action.isEnabled({});
    const openPopupKey = "mori-open-popup-smoke-" + runId;
    let openPopupResult = null;
    if (!localStorage.getItem(openPopupKey)) {
      localStorage.setItem(openPopupKey, "1");
      openPopupResult = await chrome.action.openPopup();
    }
    const response = await retry("sendMessage", () =>
      chrome.runtime.sendMessage(chrome.runtime.id, { type: "smoke-ping", runId }));
    const commandResponse = await retry("commands.onCommand", () =>
      chrome.runtime.sendMessage(chrome.runtime.id, { type: "command-smoke-state", runId })
        .then((result) => {
          if (!result ||
              !Array.isArray(result.commandEvents) ||
              !result.commandEvents.includes("mori-smoke-command")) {
            throw new Error("command not observed");
          }
          return result;
        }));
    const portResponse = await retry("connect", portRoundTrip);
    const externalMessageResponse = await externalMessageRoundTrip();
    const extensionFetchResponse = await extensionFetchRoundTrip();
    const contentResponse = await contentScriptRoundTrip();
    const executeScriptResponse = await executeScriptRoundTrip();
    const cssInjectionResponse = await cssInjectionRoundTrip();
    const dynamicContentResponse = await dynamicContentScriptRoundTrip();
    const userScriptsResponse = await userScriptsRoundTrip();
    const webRequestResponse = await webRequestRoundTrip();
    const webNavigationResponse = await webNavigationRoundTrip();
    const dnrResponse = await declarativeNetRequestRoundTrip();
    const tabManagementResponse = await tabManagementRoundTrip();
    const tabGroupsResponse = await tabGroupsRoundTrip();
    const contextMenusResponse = await contextMenusRoundTrip();
    const windowsResponse = await windowsRoundTrip();
    const permissionsResponse = await permissionsRoundTrip();
    const proxySettingsResponse = await proxySettingsRoundTrip();
    const privacyResponse = await privacyRoundTrip();
    const contentSettingsResponse = await contentSettingsRoundTrip();
    const historyResponse = await historyRoundTrip();
    const cookiesResponse = await cookiesRoundTrip();
    const browsingDataResponse = await browsingDataRoundTrip();
    const downloadsResponse = await downloadsRoundTrip();
    const sessionsResponse = await sessionsRoundTrip();
    const managementResponse = await managementRoundTrip();
    const crxDownloadInstallResponse = await crxDownloadInstallRoundTrip();
    const offscreenResponse = await offscreenRoundTrip();
    const browserNamespaceResponse = await browserNamespaceRoundTrip();
    const webAccessibleResourcesResponse = await webAccessibleResourcesRoundTrip();
    const nativeMessagingResponse = await nativeMessagingRoundTrip();
    const notificationsResponse = await notificationsRoundTrip();
    const alarmsResponse = await alarmsRoundTrip();
    const bookmarksResponse = await bookmarksRoundTrip();
    const captureVisibleTabResponse = await captureVisibleTabRoundTrip();
    const identityResponse = await identityRoundTrip();
    const sidePanelResponse = await sidePanelRoundTrip();
    const runtimeIntrospectionResponse = await runtimeIntrospectionRoundTrip();
    const powerResponse = await powerRoundTrip();
	    const idleResponse = await idleRoundTrip();
	    const systemResponse = await systemRoundTrip();
	    const searchResponse = await searchRoundTrip();
	    const dnsResponse = await dnsRoundTrip();
	    const protectedGlobalResponse = await protectedGlobalRoundTrip();
    const ok = Object.values(checks).every(Boolean) &&
      Array.isArray(contexts) &&
      contexts.some((context) =>
        context &&
        context.contextType === chrome.runtime.ContextType.POPUP &&
        context.documentUrl === chrome.runtime.getURL("popup.html")) &&
      Array.isArray(backgroundContexts) &&
      backgroundContexts.some((context) =>
        context &&
        context.contextType === chrome.runtime.ContextType.BACKGROUND &&
        context.documentUrl === chrome.runtime.getURL("__mori_background__.html")) &&
      legacyExtensionViewsResponse &&
      legacyExtensionViewsResponse.namespace === true &&
      legacyExtensionViewsResponse.inIncognitoContext === false &&
      legacyExtensionViewsResponse.getURL === chrome.runtime.getURL("popup.html") &&
      legacyExtensionViewsResponse.allViewsIncludeSelf === true &&
      legacyExtensionViewsResponse.allViewsLength >= 1 &&
      legacyExtensionViewsResponse.popupViewsIncludeSelf === true &&
      legacyExtensionViewsResponse.popupViewsLength >= 1 &&
      legacyExtensionViewsResponse.tabViewsLength === 0 &&
      legacyExtensionViewsResponse.extensionTabsLength === 0 &&
      legacyExtensionViewsResponse.backgroundFromPopupIsMissing === true &&
      legacyExtensionViewsResponse.runtimeBackgroundFromPopupIsMissing === true &&
      legacyExtensionViewsResponse.background &&
      legacyExtensionViewsResponse.background.namespace === true &&
      legacyExtensionViewsResponse.background.inIncognitoContext === false &&
      legacyExtensionViewsResponse.background.backgroundPageIsSelf === true &&
      legacyExtensionViewsResponse.background.runtimeBackgroundPageIsSelf === true &&
      legacyExtensionViewsResponse.background.allViewsIncludeSelf === true &&
      legacyExtensionViewsResponse.background.allViewsLength >= 1 &&
      legacyExtensionViewsResponse.background.popupViewsLength === 0 &&
      legacyExtensionViewsResponse.background.tabViewsLength === 0 &&
      legacyExtensionViewsResponse.background.extensionTabsLength === 0 &&
      legacyExtensionViewsResponse.background.getURL === chrome.runtime.getURL("popup.html") &&
      runtimeIntrospectionResponse &&
      runtimeIntrospectionResponse.version === runtimeIntrospectionResponse.manifestVersion &&
      runtimeIntrospectionResponse.version === "1.0.0" &&
      runtimeIntrospectionResponse.platformPromise &&
      runtimeIntrospectionResponse.platformPromise.os === "mac" &&
      ["arm", "arm64", "x86-32", "x86-64", "mips", "mips64", "riscv64"]
        .includes(runtimeIntrospectionResponse.platformPromise.arch) &&
      ["arm", "x86-32", "x86-64", "mips", "mips64"]
        .includes(runtimeIntrospectionResponse.platformPromise.nacl_arch) &&
      runtimeIntrospectionResponse.platformCallback &&
      runtimeIntrospectionResponse.platformCallback.os ===
        runtimeIntrospectionResponse.platformPromise.os &&
      runtimeIntrospectionResponse.platformCallback.arch ===
        runtimeIntrospectionResponse.platformPromise.arch &&
      runtimeIntrospectionResponse.platformCallback.nacl_arch ===
        runtimeIntrospectionResponse.platformPromise.nacl_arch &&
      runtimeIntrospectionResponse.updatePromise &&
      runtimeIntrospectionResponse.updatePromise.status === "no_update" &&
      runtimeIntrospectionResponse.updateCallback &&
      runtimeIntrospectionResponse.updateCallback.status === "no_update" &&
      runtimeIntrospectionResponse.restartAfterDelayCallback === true &&
      runtimeIntrospectionResponse.reloadResultType === "undefined" &&
      powerResponse &&
      powerResponse.initial === null &&
      powerResponse.levels &&
      powerResponse.levels.SYSTEM === "system" &&
      powerResponse.levels.DISPLAY === "display" &&
      powerResponse.systemReturn === null &&
      powerResponse.afterSystem &&
      powerResponse.afterSystem.level === "system" &&
      powerResponse.displayReturn === null &&
      powerResponse.afterDisplay &&
      powerResponse.afterDisplay.level === "display" &&
      powerResponse.releaseReturn === null &&
      powerResponse.afterRelease === null &&
      typeof powerResponse.invalidError === "string" &&
      powerResponse.invalidError.includes("requires level") &&
      idleResponse &&
      idleResponse.namespace === true &&
      idleResponse.browserNamespace === true &&
      idleResponse.idleStateEnum &&
      idleResponse.idleStateEnum.ACTIVE === "active" &&
      idleResponse.idleStateEnum.IDLE === "idle" &&
      idleResponse.idleStateEnum.LOCKED === "locked" &&
      idleResponse.statesValid === true &&
      idleResponse.setReturn === null &&
      idleResponse.autoLockDelay === 0 &&
      idleResponse.autoLockDelayCallback === 0 &&
      systemResponse &&
      systemResponse.hasNamespace === true &&
      systemResponse.browserNamespace === true &&
      systemResponse.cpu &&
      typeof systemResponse.cpu.archName === "string" &&
      systemResponse.cpu.archName.length > 0 &&
      typeof systemResponse.cpu.modelName === "string" &&
      systemResponse.cpu.modelName.length > 0 &&
      Number(systemResponse.cpu.numOfProcessors) >= 1 &&
      Array.isArray(systemResponse.cpu.processors) &&
      systemResponse.cpu.processors.length === systemResponse.cpu.numOfProcessors &&
      systemResponse.cpu.processors.every((processor) =>
        processor &&
        processor.usage &&
        Number(processor.usage.total) >=
          Number(processor.usage.user) + Number(processor.usage.kernel) &&
        Number(processor.usage.total) >= Number(processor.usage.idle)) &&
      systemResponse.cpuCallback &&
      systemResponse.cpuCallback.numOfProcessors ===
        systemResponse.cpu.numOfProcessors &&
      systemResponse.memory &&
      Number(systemResponse.memory.capacity) > 0 &&
      Number(systemResponse.memory.availableCapacity) >= 0 &&
      Number(systemResponse.memory.availableCapacity) <=
        Number(systemResponse.memory.capacity) &&
      systemResponse.memoryCallback &&
      systemResponse.memoryCallback.capacity === systemResponse.memory.capacity &&
      Array.isArray(systemResponse.displays) &&
      systemResponse.displays.length >= 1 &&
      systemResponse.displays.some((display) =>
        display &&
        typeof display.id === "string" &&
        display.id.length > 0 &&
        display.isEnabled === true &&
        display.bounds &&
        Number(display.bounds.width) > 0 &&
        Number(display.bounds.height) > 0 &&
        display.workArea &&
        Number(display.workArea.width) > 0 &&
        Number(display.workArea.height) > 0 &&
        Array.isArray(display.modes) &&
        Array.isArray(display.availableDisplayZoomFactors)) &&
      Array.isArray(systemResponse.displaysCallback) &&
      systemResponse.displaysCallback.length === systemResponse.displays.length &&
      Array.isArray(systemResponse.storageUnits) &&
      systemResponse.storageUnits.length >= 1 &&
      systemResponse.storageUnits[0] &&
      typeof systemResponse.storageUnits[0].id === "string" &&
      systemResponse.storageUnits[0].id.length > 0 &&
      ["fixed", "removable", "unknown"].includes(systemResponse.storageUnits[0].type) &&
      Number(systemResponse.storageUnits[0].capacity) > 0 &&
      Array.isArray(systemResponse.storageCallback) &&
      systemResponse.storageCallback.length === systemResponse.storageUnits.length &&
      systemResponse.available &&
      systemResponse.available.id === systemResponse.storageUnits[0].id &&
      Number(systemResponse.available.availableCapacity) >= 0 &&
      systemResponse.availableCallback &&
      systemResponse.availableCallback.id === systemResponse.available.id &&
      Number(systemResponse.availableCallback.availableCapacity) >= 0 &&
      Number(systemResponse.availableCallback.availableCapacity) <=
        Number(systemResponse.storageUnits[0].capacity) &&
      systemResponse.ejectMissing === "no_such_device" &&
      ["failure", "in_use", "success"].includes(systemResponse.ejectFixed) &&
      systemResponse.storageEvents &&
      systemResponse.storageEvents.attached === true &&
      systemResponse.storageEvents.detached === true &&
      searchResponse &&
      searchResponse.namespace &&
      searchResponse.namespace.chrome === true &&
      searchResponse.namespace.browser === true &&
      searchResponse.namespace.disposition &&
      searchResponse.namespace.disposition.CURRENT_TAB === "CURRENT_TAB" &&
      searchResponse.namespace.disposition.NEW_TAB === "NEW_TAB" &&
      searchResponse.namespace.disposition.NEW_WINDOW === "NEW_WINDOW" &&
      searchResponse.tabReturn === null &&
      searchResponse.currentReturn === null &&
      searchResponse.browserReturn === null &&
      searchResponse.tabAfter &&
      searchResponse.tabAfter.id &&
      String(searchResponse.tabAfter.url || "")
        .includes(moriEncodedQueryToken(searchResponse.tabText)) &&
      searchResponse.currentAfter &&
      String(searchResponse.currentAfter.url || "")
        .includes(moriEncodedQueryToken(searchResponse.currentText)) &&
      searchResponse.newTabCallback &&
      searchResponse.newTabCallback.lastError === "" &&
      searchResponse.newTabAfter &&
      String(searchResponse.newTabAfter.url || "")
        .includes(moriEncodedQueryToken(searchResponse.newTabText)) &&
	      searchResponse.browserAfter &&
	      String(searchResponse.browserAfter.url || "")
	        .includes(moriEncodedQueryToken(searchResponse.browserText)) &&
	      typeof searchResponse.invalidError === "string" &&
	      searchResponse.invalidError.includes("cannot use tabId") &&
	      dnsResponse &&
	      dnsResponse.namespace === true &&
	      dnsResponse.browserNamespace === true &&
	      dnsResponse.promiseResult &&
	      dnsResponse.promiseResult.resultCode === 0 &&
	      typeof dnsResponse.promiseResult.address === "string" &&
	      dnsResponse.promiseResult.address.length > 0 &&
	      dnsResponse.callbackResult &&
	      dnsResponse.callbackResult.resultCode === 0 &&
	      typeof dnsResponse.callbackResult.address === "string" &&
	      dnsResponse.callbackResult.address.length > 0 &&
	      dnsResponse.browserResult &&
	      dnsResponse.browserResult.resultCode === 0 &&
	      typeof dnsResponse.browserResult.address === "string" &&
	      dnsResponse.browserResult.address.length > 0 &&
	      dnsResponse.invalidResult &&
	      Number(dnsResponse.invalidResult.resultCode) !== 0 &&
	      dnsResponse.invalidResult.address === undefined &&
	      stored && stored.smokeKey === runId && stored.count === 1 &&
      Array.isArray(storageKeys) &&
      storageKeys.includes("smokeKey") &&
      storageKeys.includes("count") &&
      Number(storageBytes) > 0 &&
      storageEvents.some((event) =>
        event &&
        event.areaName === "local" &&
        event.changes &&
        event.changes.smokeKey &&
        event.changes.smokeKey.newValue === runId) &&
      storageConstants.localQuota >= 10485760 &&
      storageConstants.syncQuota >= 102400 &&
      storageConstants.syncItemQuota >= 8192 &&
      storageConstants.syncMaxItems >= 512 &&
      storageConstants.sessionQuota >= 10485760 &&
      storageAreasResponse &&
      storageAreasResponse.syncStored &&
      storageAreasResponse.syncStored.syncKey === "sync-" + runId &&
      storageAreasResponse.syncStored.syncCount === 2 &&
      Array.isArray(storageAreasResponse.syncKeys) &&
      storageAreasResponse.syncKeys.includes("syncKey") &&
      storageAreasResponse.syncKeys.includes("syncCount") &&
      Number(storageAreasResponse.syncBytes) > 0 &&
      storageAreasResponse.syncAfterRemove &&
      storageAreasResponse.syncAfterRemove.syncKey === "sync-" + runId &&
      !("syncCount" in storageAreasResponse.syncAfterRemove) &&
      storageAreasResponse.sessionStored &&
      storageAreasResponse.sessionStored.sessionKey === "session-" + runId &&
      storageAreasResponse.sessionStored.sessionFlag === true &&
      Array.isArray(storageAreasResponse.sessionKeys) &&
      storageAreasResponse.sessionKeys.includes("sessionKey") &&
      storageAreasResponse.sessionKeys.includes("sessionFlag") &&
      Number(storageAreasResponse.sessionBytes) > 0 &&
      storageAreasResponse.sessionAfterClear &&
      !("sessionKey" in storageAreasResponse.sessionAfterClear) &&
      !("sessionFlag" in storageAreasResponse.sessionAfterClear) &&
      storageAreasResponse.managedStored &&
      typeof storageAreasResponse.managedStored === "object" &&
      !Array.isArray(storageAreasResponse.managedStored) &&
      Array.isArray(storageAreasResponse.managedKeys) &&
      typeof storageAreasResponse.managedSetError === "string" &&
      storageAreasResponse.managedSetError.includes("read-only") &&
      storageEvents.some((event) =>
        event &&
        event.areaName === "sync" &&
        event.changes &&
        event.changes.syncKey &&
        event.changes.syncKey.newValue === "sync-" + runId) &&
      storageEvents.some((event) =>
        event &&
        event.areaName === "session" &&
        event.changes &&
        event.changes.sessionKey &&
        event.changes.sessionKey.newValue === "session-" + runId) &&
      userSettings && userSettings.isOnToolbar === false &&
      badgeText === "OK" &&
      Array.isArray(badgeBackgroundColor) &&
      badgeBackgroundColor.join(",") === "18,122,255,255" &&
      Array.isArray(badgeTextColor) &&
      badgeTextColor.join(",") === "16,32,48,255" &&
      actionDisabled === false &&
      actionEnabled === true &&
      response && response.pong === true &&
      response.importScripts === "loaded-" + runId &&
      response.sender && String(response.sender.url || "").includes("/popup.html") &&
      checks.earlyNavigatorOnline === true &&
      checks.navigatorOnline === true &&
      commandResponse &&
      commandResponse.commandEvents &&
      commandResponse.commandEvents.includes("mori-smoke-command") &&
      portResponse && portResponse.pong === true &&
      portResponse.sender && String(portResponse.sender.url || "").includes("/popup.html") &&
      externalMessageResponse &&
      externalMessageResponse.response &&
      externalMessageResponse.response.externalPong === true &&
      externalMessageResponse.response.runtimeId === extensionId &&
      externalMessageResponse.response.message &&
      externalMessageResponse.response.message.type === "fork" &&
      externalMessageResponse.response.message.runId === runId &&
      externalMessageResponse.response.sender &&
      !externalMessageResponse.response.sender.id &&
      externalMessageResponse.response.sender.url === externalPageURL &&
      externalMessageResponse.response.sender.origin === new URL(externalPageURL).origin &&
      externalMessageResponse.response.sender.tab &&
      externalMessageResponse.response.sender.tab.url === externalPageURL &&
      extensionFetchResponse &&
      extensionFetchResponse.postStatus === 200 &&
      extensionFetchResponse.postJSON &&
      extensionFetchResponse.postJSON.ok === true &&
      extensionFetchResponse.postJSON.method === "POST" &&
      (extensionFetchResponse.postJSON.origin === null ||
        String(extensionFetchResponse.postJSON.origin || "")
          .startsWith("mori-extension://" + extensionId)) &&
      extensionFetchResponse.postJSON.header === runId &&
      String(extensionFetchResponse.postJSON.cookie || "")
        .includes(extensionFetchResponse.cookieName + "=" +
          extensionFetchResponse.cookieValue) &&
      extensionFetchResponse.postJSON.body &&
      extensionFetchResponse.postJSON.body.runId === runId &&
      extensionFetchResponse.postJSON.body.source === "popup" &&
      extensionFetchResponse.getStatus === 200 &&
      extensionFetchResponse.getJSON &&
      extensionFetchResponse.getJSON.ok === true &&
      extensionFetchResponse.getJSON.method === "GET" &&
      extensionFetchResponse.getJSON.header === runId + "-get" &&
      String(extensionFetchResponse.getJSON.cookie || "")
        .includes(extensionFetchResponse.cookieName + "=" +
          extensionFetchResponse.cookieValue) &&
      contentResponse && contentResponse.contentPong === true &&
      contentResponse.runtimeId === extensionId &&
      contentResponse.marker === runId &&
      contentResponse.startLoaded === "loaded" &&
      contentResponse.endLoaded === "loaded" &&
      contentResponse.idleLoaded === "loaded" &&
      contentResponse.mainWorldLoaded === "page" &&
      contentResponse.focus &&
      contentResponse.focus.done === "done" &&
      contentResponse.focus.error === "" &&
      contentResponse.focus.inputValue === "focused-" + runId &&
      contentResponse.focus.inputDataset === runId &&
      contentResponse.focus.overlayRunId === runId &&
      contentResponse.focus.background &&
      contentResponse.focus.background.focusPong === true &&
      contentResponse.focus.background.sender &&
      contentResponse.focus.background.sender.id === extensionId &&
      contentResponse.focus.background.sender.frameId === 0 &&
      contentResponse.focus.background.sender.tab &&
      String(contentResponse.focus.background.sender.tab.url || "") === smokePageURL &&
      contentResponse.focus.port &&
      contentResponse.focus.port.pong === true &&
      contentResponse.focus.port.sender &&
      contentResponse.focus.port.sender.id === extensionId &&
      contentResponse.focus.port.sender.frameId === 0 &&
      contentResponse.focus.port.sender.tab &&
      String(contentResponse.focus.port.sender.tab.url || "") === smokePageURL &&
      Array.isArray(contentResponse.protectedSetup) &&
      contentResponse.protectedSetup.some((item) =>
        item &&
        item.frameId === 0 &&
        item.result &&
        item.result.chrome &&
        item.result.chrome.afterRuntime !== extensionId &&
        item.result.browser &&
        item.result.browser.afterRuntime !== extensionId) &&
      Array.isArray(contentResponse.protectedDynamic) &&
      contentResponse.protectedDynamic.some((item) =>
        item &&
        item.frameId === 0 &&
        item.result &&
        item.result.runtimeId === extensionId &&
        item.result.browserRuntimeId === extensionId &&
        item.result.marker === extensionId) &&
      contentResponse.frameTarget &&
      Array.isArray(contentResponse.frameTarget.frames) &&
      contentResponse.frameTarget.childFrame &&
      contentResponse.frameTarget.childFrame.frameId > 0 &&
      typeof contentResponse.frameTarget.childFrame.documentId === "string" &&
      contentResponse.frameTarget.childFrame.documentId.length > 0 &&
      String(contentResponse.frameTarget.childFrame.url || "")
        .includes("frame-smoke.html") &&
      contentResponse.frameTarget.documentFrame &&
      contentResponse.frameTarget.documentFrame.frameId ===
        contentResponse.frameTarget.childFrame.frameId &&
      contentResponse.frameTarget.documentFrame.documentId ===
        contentResponse.frameTarget.childFrame.documentId &&
      contentResponse.frameTarget.targeted &&
      contentResponse.frameTarget.targeted.frameTargetPong === true &&
      contentResponse.frameTarget.targeted.frameId ===
        contentResponse.frameTarget.childFrame.frameId &&
      contentResponse.frameTarget.targeted.documentId ===
        contentResponse.frameTarget.childFrame.documentId &&
      contentResponse.frameTarget.targeted.context &&
      contentResponse.frameTarget.targeted.context.documentId ===
        contentResponse.frameTarget.childFrame.documentId &&
      contentResponse.frameTarget.documentTargeted &&
      contentResponse.frameTarget.documentTargeted.frameTargetPong === true &&
      contentResponse.frameTarget.documentTargeted.frameId ===
        contentResponse.frameTarget.childFrame.frameId &&
      contentResponse.frameTarget.documentTargeted.documentId ===
        contentResponse.frameTarget.childFrame.documentId &&
      contentResponse.frameTarget.documentTargeted.context &&
      contentResponse.frameTarget.documentTargeted.context.documentId ===
        contentResponse.frameTarget.childFrame.documentId &&
      String(contentResponse.frameTarget.targeted.href || "")
        .includes("frame-smoke.html") &&
      String(contentResponse.frameTarget.documentTargeted.href || "")
        .includes("frame-smoke.html") &&
      String(contentResponse.href || "") === smokePageURL &&
      executeScriptResponse &&
      Array.isArray(executeScriptResponse.sync) &&
      executeScriptResponse.sync.some((item) =>
        item &&
        item.frameId === 0 &&
        item.result &&
        item.result.value === runId &&
        item.result.marker === runId &&
        item.result.dataset === runId &&
        String(item.result.href || "") ===
          smokePageURL + "?execute=" + encodeURIComponent(runId)) &&
	      Array.isArray(executeScriptResponse.async) &&
	      executeScriptResponse.async.some((item) =>
	        item &&
	        item.frameId === 0 &&
	        item.result &&
	        item.result.asyncValue === runId &&
	        item.result.marker === runId &&
	        item.result.dataset === runId &&
	        String(item.result.href || "") ===
	          smokePageURL + "?execute-async=" + encodeURIComponent(runId)) &&
	      executeScriptResponse.world &&
	      Array.isArray(executeScriptResponse.world.main) &&
	      executeScriptResponse.world.main.some((item) =>
	        item &&
	        item.frameId === 0 &&
	        item.result &&
	        item.result.mainHasRuntime === false) &&
	      Array.isArray(executeScriptResponse.world.isolatedAfterMain) &&
	      executeScriptResponse.world.isolatedAfterMain.some((item) =>
	        item &&
	        item.frameId === 0 &&
	        item.result &&
	        item.result.runtimeId === extensionId &&
	        String(item.result.url || "") ===
	          "mori-extension://" + extensionId + "/dropdown.html" &&
	        item.result.context &&
	        item.result.context.frameId === 0) &&
	      executeScriptResponse.frames &&
	      executeScriptResponse.frames.childFrame &&
	      executeScriptResponse.frames.childFrame.frameId > 0 &&
	      typeof executeScriptResponse.frames.childFrame.documentId === "string" &&
	      executeScriptResponse.frames.childFrame.documentId.length > 0 &&
	      executeScriptResponse.frames.documentFrame &&
	      executeScriptResponse.frames.documentFrame.frameId ===
	        executeScriptResponse.frames.childFrame.frameId &&
	      executeScriptResponse.frames.documentFrame.documentId ===
	        executeScriptResponse.frames.childFrame.documentId &&
	      Array.isArray(executeScriptResponse.frames.all) &&
	      executeScriptResponse.frames.all.some((item) =>
	        item &&
	        item.frameId === 0 &&
	        item.result &&
	        item.result.value === runId &&
	        item.result.runtimeFrameId === 0 &&
	        item.result.context &&
	        item.result.context.frameId === 0 &&
	        item.result.pageMarker === runId &&
	        String(item.result.href || "")
	          .includes("smoke-page.html?execute-frames=")) &&
	      executeScriptResponse.frames.all.some((item) =>
	        item &&
	        item.frameId === executeScriptResponse.frames.childFrame.frameId &&
	        item.documentId === executeScriptResponse.frames.childFrame.documentId &&
	        item.result &&
	        item.result.value === runId &&
	        item.result.runtimeFrameId ===
	          executeScriptResponse.frames.childFrame.frameId &&
	        item.result.context &&
	        item.result.context.frameId ===
	          executeScriptResponse.frames.childFrame.frameId &&
	        item.result.context.documentId ===
	          executeScriptResponse.frames.childFrame.documentId &&
	        item.result.frameMarker === runId &&
	        String(item.result.href || "").includes("frame-smoke.html")) &&
	      Array.isArray(executeScriptResponse.frames.targeted) &&
	      executeScriptResponse.frames.targeted.length === 1 &&
	      executeScriptResponse.frames.targeted[0] &&
	      executeScriptResponse.frames.targeted[0].frameId ===
	        executeScriptResponse.frames.childFrame.frameId &&
	      executeScriptResponse.frames.targeted[0].documentId ===
	        executeScriptResponse.frames.childFrame.documentId &&
	      executeScriptResponse.frames.targeted[0].result &&
	      executeScriptResponse.frames.targeted[0].result.runtimeFrameId ===
	        executeScriptResponse.frames.childFrame.frameId &&
	      executeScriptResponse.frames.targeted[0].result.context &&
	      executeScriptResponse.frames.targeted[0].result.context.frameId ===
	        executeScriptResponse.frames.childFrame.frameId &&
	      executeScriptResponse.frames.targeted[0].result.context.documentId ===
	        executeScriptResponse.frames.childFrame.documentId &&
	      executeScriptResponse.frames.targeted[0].result.frameMarker === runId &&
	      Array.isArray(executeScriptResponse.frames.documentTargeted) &&
	      executeScriptResponse.frames.documentTargeted.length === 1 &&
	      executeScriptResponse.frames.documentTargeted[0] &&
	      executeScriptResponse.frames.documentTargeted[0].frameId ===
	        executeScriptResponse.frames.childFrame.frameId &&
	      executeScriptResponse.frames.documentTargeted[0].documentId ===
	        executeScriptResponse.frames.childFrame.documentId &&
	      executeScriptResponse.frames.documentTargeted[0].result &&
	      executeScriptResponse.frames.documentTargeted[0].result.runtimeFrameId ===
	        executeScriptResponse.frames.childFrame.frameId &&
	      executeScriptResponse.frames.documentTargeted[0].result.context &&
	      executeScriptResponse.frames.documentTargeted[0].result.context.frameId ===
	        executeScriptResponse.frames.childFrame.frameId &&
	      executeScriptResponse.frames.documentTargeted[0].result.context.documentId ===
	        executeScriptResponse.frames.childFrame.documentId &&
	      executeScriptResponse.frames.documentTargeted[0].result.frameMarker === runId &&
	      cssInjectionResponse &&
      Array.isArray(cssInjectionResponse.afterInsert) &&
      cssInjectionResponse.afterInsert.some((item) =>
        item && item.result === "rgb(9, 87, 165)") &&
      Array.isArray(cssInjectionResponse.afterRemove) &&
      cssInjectionResponse.afterRemove.some((item) =>
        item && item.result === "rgb(1, 2, 3)") &&
      Array.isArray(cssInjectionResponse.legacyAfterInsert) &&
      cssInjectionResponse.legacyAfterInsert.some((item) =>
        item && item.result === "rgb(201, 77, 33)") &&
      Array.isArray(cssInjectionResponse.legacyAfterRemove) &&
      cssInjectionResponse.legacyAfterRemove.some((item) =>
        item && item.result === "rgb(4, 5, 6)") &&
      cssInjectionResponse.frameTargetCSS &&
      cssInjectionResponse.frameTargetCSS.childFrame &&
      cssInjectionResponse.frameTargetCSS.childFrame.frameId > 0 &&
      Array.isArray(cssInjectionResponse.frameTargetCSS.afterInsert) &&
      cssInjectionResponse.frameTargetCSS.afterInsert.some((item) =>
        item &&
        item.frameId === 0 &&
        item.result &&
        item.result.color === "rgb(11, 22, 33)" &&
        String(item.result.href || "").includes("smoke-page.html?css-frames=")) &&
      cssInjectionResponse.frameTargetCSS.afterInsert.some((item) =>
        item &&
        item.frameId === cssInjectionResponse.frameTargetCSS.childFrame.frameId &&
        item.result &&
        item.result.color === "rgb(88, 99, 111)" &&
        String(item.result.href || "").includes("frame-smoke.html")) &&
      Array.isArray(cssInjectionResponse.frameTargetCSS.afterRemove) &&
      cssInjectionResponse.frameTargetCSS.afterRemove.some((item) =>
        item &&
        item.frameId === 0 &&
        item.result &&
        item.result.color === "rgb(11, 22, 33)") &&
      cssInjectionResponse.frameTargetCSS.afterRemove.some((item) =>
        item &&
        item.frameId === cssInjectionResponse.frameTargetCSS.childFrame.frameId &&
        item.result &&
        item.result.color === "rgb(11, 22, 33)") &&
      dynamicContentResponse &&
      Array.isArray(dynamicContentResponse.registered) &&
      dynamicContentResponse.registered.length === 1 &&
      dynamicContentResponse.registered[0].id === "mori-dynamic-smoke" &&
      dynamicContentResponse.registered[0].runAt === "document_end" &&
      dynamicContentResponse.response &&
      dynamicContentResponse.response.dynamicPong === true &&
      dynamicContentResponse.response.runtimeId === extensionId &&
      dynamicContentResponse.response.marker === runId &&
      Array.isArray(dynamicContentResponse.updated) &&
      dynamicContentResponse.updated.length === 1 &&
      dynamicContentResponse.updated[0].runAt === "document_idle" &&
      Array.isArray(dynamicContentResponse.afterUnregister) &&
      dynamicContentResponse.afterUnregister.length === 0 &&
      userScriptsResponse &&
      userScriptsResponse.namespace === true &&
      userScriptsResponse.browserNamespace === true &&
      userScriptsResponse.executionWorld &&
      userScriptsResponse.executionWorld.USER_SCRIPT === "USER_SCRIPT" &&
      Array.isArray(userScriptsResponse.before) &&
      userScriptsResponse.before.length === 0 &&
      Array.isArray(userScriptsResponse.worldConfigurations) &&
      userScriptsResponse.worldConfigurations.some((world) =>
        world &&
        world.worldId === "moriUserWorld" &&
        world.messaging === true &&
        world.csp === "script-src 'self'") &&
      Array.isArray(userScriptsResponse.registered) &&
      userScriptsResponse.registered.length === 1 &&
      userScriptsResponse.registered[0].id === "mori-user-script-smoke" &&
      Array.isArray(userScriptsResponse.registered[0].js) &&
      userScriptsResponse.registered[0].js[0] &&
      String(userScriptsResponse.registered[0].js[0].code || "").includes("registered-") &&
      userScriptsResponse.injected === "registered-" + runId &&
      Array.isArray(userScriptsResponse.updated) &&
      userScriptsResponse.updated.length === 1 &&
      String(userScriptsResponse.updated[0].js[0].code || "").includes("updated-") &&
      userScriptsResponse.injectedAfterUpdate === "updated-" + runId &&
      Array.isArray(userScriptsResponse.executeResult) &&
      userScriptsResponse.executeObserved === "executed-" + runId &&
      Array.isArray(userScriptsResponse.afterUnregister) &&
      userScriptsResponse.afterUnregister.length === 0 &&
      Array.isArray(userScriptsResponse.worldConfigurationsAfterReset) &&
      !userScriptsResponse.worldConfigurationsAfterReset.some((world) =>
        world && world.worldId === "moriUserWorld") &&
      webRequestResponse &&
      webRequestResponse.hasBeforeRemove === true &&
      webRequestResponse.hasAfterRemove === false &&
      Array.isArray(webRequestResponse.nonMatching) &&
      webRequestResponse.nonMatching.length === 0 &&
      Array.isArray(webRequestResponse.removed) &&
      webRequestResponse.removed.length === 0 &&
      Array.isArray(webRequestResponse.beforeRequest) &&
      webRequestResponse.beforeRequest.some((event) =>
        event &&
        String(event.url || "") === smokePageURL + "?webrequest=" + encodeURIComponent(runId) &&
        event.type === "main_frame") &&
      Array.isArray(webRequestResponse.beforeSendHeaders) &&
      webRequestResponse.beforeSendHeaders.some((event) =>
        event &&
        Array.isArray(event.requestHeaders)) &&
      Array.isArray(webRequestResponse.beforeSendHeadersWithoutExtra) &&
      webRequestResponse.beforeSendHeadersWithoutExtra.some((event) =>
        event &&
        event.requestHeaders === undefined) &&
      Array.isArray(webRequestResponse.headersReceived) &&
      webRequestResponse.headersReceived.some((event) =>
        event &&
        Array.isArray(event.responseHeaders) &&
        typeof event.statusCode === "number") &&
      Array.isArray(webRequestResponse.headersReceivedWithoutExtra) &&
      webRequestResponse.headersReceivedWithoutExtra.some((event) =>
        event &&
        event.responseHeaders === undefined &&
        typeof event.statusCode === "number") &&
      Array.isArray(webRequestResponse.completed) &&
      webRequestResponse.completed.some((event) =>
        event &&
        typeof event.statusCode === "number") &&
      Array.isArray(webRequestResponse.errors) &&
      webRequestResponse.errors.length === 0 &&
      webNavigationResponse &&
      webNavigationResponse.events &&
      Array.isArray(webNavigationResponse.events.history) &&
      webNavigationResponse.events.history.some((event) =>
        event &&
        typeof event.timeStamp === "number" &&
        event.frameId === 0 &&
        event.parentFrameId === -1 &&
        String(event.url || "").includes("?spa=" + encodeURIComponent(runId))) &&
      Array.isArray(webNavigationResponse.events.fragment) &&
      webNavigationResponse.events.fragment.some((event) =>
        event &&
        typeof event.timeStamp === "number" &&
        event.frameId === 0 &&
        event.parentFrameId === -1 &&
        String(event.url || "").includes("#fragment-" + encodeURIComponent(runId))) &&
      Array.isArray(webNavigationResponse.scriptResult) &&
      webNavigationResponse.scriptResult.some((item) =>
        item &&
        item.result &&
        String(item.result || "").includes("#fragment-" + encodeURIComponent(runId))) &&
      dnrResponse &&
      Array.isArray(dnrResponse.rules) &&
      dnrResponse.rules.some((rule) => rule && rule.id === 9001) &&
      dnrResponse.rules.some((rule) => rule && rule.id === 9002) &&
      dnrResponse.rules.some((rule) => rule && rule.id === 9003) &&
      dnrResponse.rules.some((rule) => rule && rule.id === 9004) &&
      Array.isArray(dnrResponse.observedErrors) &&
      dnrResponse.observedErrors.some((event) =>
        event &&
        event.error === "net::ERR_BLOCKED_BY_CLIENT" &&
        String(event.url || "").includes("dnr-blocked=" + encodeURIComponent(runId))) &&
      Array.isArray(dnrResponse.observedRedirects) &&
      dnrResponse.observedRedirects.some((event) =>
        event &&
        String(event.url || "") === smokePageURL + "?dnr-source=" + encodeURIComponent(runId) &&
        event.redirectUrl === smokePageURL + "?dnr-redirected=" + encodeURIComponent(runId)) &&
      Array.isArray(dnrResponse.redirectCompletions) &&
      dnrResponse.redirectCompletions.some((event) =>
        event &&
        String(event.url || "") === smokePageURL + "?dnr-redirected=" + encodeURIComponent(runId)) &&
      Array.isArray(dnrResponse.observedHeaderRequests) &&
      dnrResponse.observedHeaderRequests.some((event) =>
        event &&
        Array.isArray(event.requestHeaders) &&
        event.requestHeaders.some((header) =>
          header &&
          String(header.name || "").toLowerCase() === "x-mori-dnr-request" &&
          header.value === "request-" + runId) &&
        !event.requestHeaders.some((header) =>
          header &&
          String(header.name || "").toLowerCase() === "upgrade-insecure-requests")) &&
      Array.isArray(dnrResponse.observedHeaderResponses) &&
      dnrResponse.observedHeaderResponses.some((event) =>
        event &&
        String(event.url || "").includes("/dnr-response?run=" + encodeURIComponent(runId)) &&
        Array.isArray(event.responseHeaders) &&
        event.responseHeaders.some((header) =>
          header &&
          String(header.name || "").toLowerCase() === "x-mori-dnr-response" &&
          header.value === "response-" + runId) &&
        event.responseHeaders.some((header) =>
          header &&
          String(header.name || "").toLowerCase() === "x-mori-dnr-original" &&
          header.value === "original-" + runId) &&
        !event.responseHeaders.some((header) =>
          header &&
          String(header.name || "").toLowerCase() === "x-mori-dnr-remove")) &&
      Array.isArray(dnrResponse.afterRemove) &&
      dnrResponse.afterRemove.length === 0 &&
      tabManagementResponse &&
      tabManagementResponse.managed &&
      tabManagementResponse.duplicated &&
      tabManagementResponse.moved &&
      tabManagementResponse.moved.index === 0 &&
      tabManagementResponse.moved.id === tabManagementResponse.duplicated.id &&
      tabManagementResponse.activeAfterHighlight &&
      tabManagementResponse.activeAfterHighlight.id === tabManagementResponse.duplicated.id &&
      tabManagementResponse.highlightedWindow &&
      Array.isArray(tabManagementResponse.highlightedWindow.tabs) &&
      tabManagementResponse.highlightedWindow.tabs.some((tab) =>
        tab &&
        tab.id === tabManagementResponse.duplicated.id &&
        tab.active === true) &&
      tabManagementResponse.events &&
      Array.isArray(tabManagementResponse.events.created) &&
      tabManagementResponse.events.created.some((tab) => tab && tab.id === tabManagementResponse.managed.id) &&
      tabManagementResponse.events.created.some((tab) => tab && tab.id === tabManagementResponse.duplicated.id) &&
      Array.isArray(tabManagementResponse.events.activated) &&
      tabManagementResponse.events.activated.some((info) => info && info.tabId === tabManagementResponse.duplicated.id) &&
      Array.isArray(tabManagementResponse.events.highlighted) &&
      tabManagementResponse.events.highlighted.some((info) =>
        info &&
        Array.isArray(info.tabIds) &&
        info.tabIds.includes(tabManagementResponse.duplicated.id)) &&
      Array.isArray(tabManagementResponse.events.moved) &&
      tabManagementResponse.events.moved.some((event) =>
        event &&
        event.tabId === tabManagementResponse.duplicated.id &&
        event.moveInfo &&
        event.moveInfo.toIndex === 0) &&
      Array.isArray(tabManagementResponse.events.removed) &&
      tabManagementResponse.events.removed.some((event) => event && event.tabId === tabManagementResponse.managed.id) &&
      tabManagementResponse.events.removed.some((event) => event && event.tabId === tabManagementResponse.duplicated.id) &&
      tabGroupsResponse &&
      tabGroupsResponse.constants &&
      tabGroupsResponse.constants.tabIdNone === -1 &&
      tabGroupsResponse.constants.tabIndexNone === -1 &&
      tabGroupsResponse.constants.tabGroupIdNone === -1 &&
      Number(tabGroupsResponse.groupId) >= 1 &&
      tabGroupsResponse.group &&
      tabGroupsResponse.group.id === tabGroupsResponse.groupId &&
      tabGroupsResponse.group.color === "grey" &&
      tabGroupsResponse.group.collapsed === false &&
      tabGroupsResponse.firstAfterGroup &&
      tabGroupsResponse.firstAfterGroup.groupId === tabGroupsResponse.groupId &&
      tabGroupsResponse.updated &&
      tabGroupsResponse.updated.id === tabGroupsResponse.groupId &&
      tabGroupsResponse.updated.title === "Mori Group " + runId &&
      tabGroupsResponse.updated.color === "cyan" &&
      tabGroupsResponse.updated.collapsed === true &&
      Array.isArray(tabGroupsResponse.queried) &&
      tabGroupsResponse.queried.some((group) =>
        group && group.id === tabGroupsResponse.groupId) &&
      tabGroupsResponse.moved &&
      tabGroupsResponse.moved.id === tabGroupsResponse.groupId &&
      tabGroupsResponse.firstAfterUngroup &&
      tabGroupsResponse.firstAfterUngroup.groupId === -1 &&
      tabGroupsResponse.events &&
      Array.isArray(tabGroupsResponse.events.created) &&
      tabGroupsResponse.events.created.some((group) =>
        group && group.id === tabGroupsResponse.groupId) &&
      Array.isArray(tabGroupsResponse.events.updated) &&
      tabGroupsResponse.events.updated.some((group) =>
        group &&
        group.id === tabGroupsResponse.groupId &&
        group.color === "cyan" &&
        group.collapsed === true) &&
      Array.isArray(tabGroupsResponse.events.moved) &&
      tabGroupsResponse.events.moved.some((group) =>
        group && group.id === tabGroupsResponse.groupId) &&
      Array.isArray(tabGroupsResponse.events.removed) &&
      tabGroupsResponse.events.removed.some((group) =>
        group && group.id === tabGroupsResponse.groupId) &&
      Array.isArray(tabGroupsResponse.events.tabUpdated) &&
      tabGroupsResponse.events.tabUpdated.some((event) =>
        event &&
        event.tabId === tabGroupsResponse.first.id &&
        event.changeInfo &&
        event.changeInfo.groupId === tabGroupsResponse.groupId) &&
      tabGroupsResponse.events.tabUpdated.some((event) =>
        event &&
        event.tabId === tabGroupsResponse.first.id &&
        event.changeInfo &&
        event.changeInfo.groupId === -1) &&
      contextMenusResponse &&
      contextMenusResponse.createdNormalId === contextMenusResponse.normalId &&
      contextMenusResponse.createdCheckboxId === contextMenusResponse.checkboxId &&
      contextMenusResponse.clicked &&
      contextMenusResponse.clicked.argCount === 2 &&
      contextMenusResponse.clicked.info &&
      contextMenusResponse.clicked.info.menuItemId === contextMenusResponse.checkboxId &&
      contextMenusResponse.clicked.info.pageUrl === contextMenusResponse.tab.url &&
      contextMenusResponse.clicked.info.selectionText === "selected smoke text" &&
      contextMenusResponse.clicked.info.checked === true &&
      contextMenusResponse.clicked.info.wasChecked === false &&
      contextMenusResponse.clicked.info.frameId === 0 &&
      contextMenusResponse.clicked.info.parentFrameId === -1 &&
      contextMenusResponse.clicked.tab &&
      contextMenusResponse.clicked.tab.id === contextMenusResponse.tab.id &&
      contextMenusResponse.clicked.tab.windowId === 1 &&
      contextMenusResponse.clicked.tab.url === contextMenusResponse.tab.url &&
      windowsResponse &&
      windowsResponse.current &&
      windowsResponse.current.id === 1 &&
      Number(windowsResponse.current.width) > 0 &&
      Number(windowsResponse.current.height) > 0 &&
      Array.isArray(windowsResponse.current.tabs) &&
      windowsResponse.lastFocused &&
      windowsResponse.lastFocused.id === 1 &&
      windowsResponse.byId &&
      windowsResponse.byId.id === 1 &&
      Array.isArray(windowsResponse.all) &&
      windowsResponse.all.length === 1 &&
      Array.isArray(windowsResponse.allAfterCreate) &&
      windowsResponse.allAfterCreate.length === windowsResponse.all.length &&
      windowsResponse.allAfterCreate[0] &&
      windowsResponse.allAfterCreate[0].id === 1 &&
      Array.isArray(windowsResponse.allAfterCreate[0].tabs) &&
      windowsResponse.allAfterCreate[0].tabs.some((tab) =>
        tab &&
        String(tab.url || "") === smokePageURL + "?window=" + encodeURIComponent(runId)) &&
      windowsResponse.created &&
      windowsResponse.created.id === 1 &&
      Array.isArray(windowsResponse.created.tabs) &&
      windowsResponse.created.tabs.some((tab) =>
        tab &&
        String(tab.url || "") === smokePageURL + "?window=" + encodeURIComponent(runId)) &&
      windowsResponse.focused &&
      windowsResponse.focused.focused === true &&
      windowsResponse.events &&
      Array.isArray(windowsResponse.events.created) &&
      windowsResponse.events.created.some((window) => window && window.id === 1) &&
      Array.isArray(windowsResponse.events.focusChanged) &&
      windowsResponse.events.focusChanged.includes(1) &&
      permissionsResponse &&
      permissionsResponse.before === false &&
      permissionsResponse.requested === true &&
      permissionsResponse.afterRequest === true &&
      permissionsResponse.allAfterRequest &&
      Array.isArray(permissionsResponse.allAfterRequest.permissions) &&
      permissionsResponse.allAfterRequest.permissions.includes("notifications") &&
      Array.isArray(permissionsResponse.allAfterRequest.origins) &&
      permissionsResponse.allAfterRequest.origins.includes("https://example.com/*") &&
      permissionsResponse.removedResult === true &&
      permissionsResponse.afterRemove === false &&
      permissionsResponse.allAfterRemove &&
      Array.isArray(permissionsResponse.allAfterRemove.permissions) &&
      !permissionsResponse.allAfterRemove.permissions.includes("notifications") &&
      Array.isArray(permissionsResponse.allAfterRemove.origins) &&
      !permissionsResponse.allAfterRemove.origins.includes("https://example.com/*") &&
      Array.isArray(permissionsResponse.added) &&
      permissionsResponse.added.some((event) =>
        event &&
        Array.isArray(event.permissions) &&
        event.permissions.includes("notifications") &&
        Array.isArray(event.origins) &&
        event.origins.includes("https://example.com/*")) &&
      Array.isArray(permissionsResponse.removed) &&
      permissionsResponse.removed.some((event) =>
        event &&
        Array.isArray(event.permissions) &&
        event.permissions.includes("notifications") &&
        Array.isArray(event.origins) &&
        event.origins.includes("https://example.com/*")) &&
      proxySettingsResponse &&
      proxySettingsResponse.before &&
      proxySettingsResponse.before.levelOfControl === "controllable_by_this_extension" &&
      proxySettingsResponse.before.value &&
      proxySettingsResponse.before.value.mode === "system" &&
      proxySettingsResponse.afterSet &&
      proxySettingsResponse.afterSet.levelOfControl === "controlled_by_this_extension" &&
      proxySettingsResponse.afterSet.value &&
      proxySettingsResponse.afterSet.value.mode === "direct" &&
      proxySettingsResponse.afterClear &&
      proxySettingsResponse.afterClear.levelOfControl === "controllable_by_this_extension" &&
      proxySettingsResponse.afterClear.value &&
      proxySettingsResponse.afterClear.value.mode === "system" &&
      Array.isArray(proxySettingsResponse.changes) &&
      proxySettingsResponse.changes.some((event) =>
        event &&
        event.levelOfControl === "controlled_by_this_extension" &&
        event.value &&
        event.value.mode === "direct") &&
	      proxySettingsResponse.changes.some((event) =>
	        event &&
	        event.levelOfControl === "controllable_by_this_extension" &&
	        event.value &&
	        event.value.mode === "system") &&
	      privacyResponse &&
	      privacyResponse.namespace === true &&
	      privacyResponse.browserNamespace === true &&
	      privacyResponse.ipHandlingPolicyEnum &&
	      privacyResponse.ipHandlingPolicyEnum.DEFAULT === "default" &&
	      privacyResponse.ipHandlingPolicyEnum.DISABLE_NON_PROXIED_UDP ===
	        "disable_non_proxied_udp" &&
	      privacyResponse.passwordBefore &&
	      privacyResponse.passwordBefore.levelOfControl ===
	        "controllable_by_this_extension" &&
	      privacyResponse.passwordBefore.value === true &&
	      privacyResponse.passwordCallbackBefore &&
	      privacyResponse.passwordCallbackBefore.value ===
	        privacyResponse.passwordBefore.value &&
	      privacyResponse.passwordAfterSet &&
	      privacyResponse.passwordAfterSet.levelOfControl ===
	        "controlled_by_this_extension" &&
	      privacyResponse.passwordAfterSet.value === false &&
	      privacyResponse.passwordAfterClear &&
	      privacyResponse.passwordAfterClear.levelOfControl ===
	        "controllable_by_this_extension" &&
	      privacyResponse.passwordAfterClear.value === true &&
	      Array.isArray(privacyResponse.passwordChanges) &&
	      privacyResponse.passwordChanges.some((event) =>
	        event &&
	        event.levelOfControl === "controlled_by_this_extension" &&
	        event.value === false) &&
	      privacyResponse.passwordChanges.some((event) =>
	        event &&
	        event.levelOfControl === "controllable_by_this_extension" &&
	        event.value === true) &&
	      privacyResponse.webRTCAfterSet &&
	      privacyResponse.webRTCAfterSet.value === "disable_non_proxied_udp" &&
	      privacyResponse.webRTCAfterSet.levelOfControl ===
	        "controlled_by_this_extension" &&
	      privacyResponse.webRTCAfterClear &&
	      privacyResponse.webRTCAfterClear.value === "default" &&
	      privacyResponse.browserThirdPartyCookies &&
	      privacyResponse.browserThirdPartyCookies.value === true &&
	      privacyResponse.browserThirdPartyCookies.levelOfControl ===
	        "controllable_by_this_extension" &&
	      typeof privacyResponse.topicsEnableError === "string" &&
	      privacyResponse.topicsEnableError.includes("only be disabled") &&
	      contentSettingsResponse &&
      contentSettingsResponse.namespace === true &&
      contentSettingsResponse.browserNamespace === true &&
      contentSettingsResponse.cookieBefore &&
      contentSettingsResponse.cookieBefore.setting === "allow" &&
      contentSettingsResponse.cookieCallbackBefore &&
      contentSettingsResponse.cookieCallbackBefore.setting === "allow" &&
      contentSettingsResponse.cookieAfterSpecific &&
      contentSettingsResponse.cookieAfterSpecific.setting === "session_only" &&
      contentSettingsResponse.cookieAfterWildcard &&
      contentSettingsResponse.cookieAfterWildcard.setting === "block" &&
      contentSettingsResponse.imageCallback &&
      contentSettingsResponse.imageCallback.details &&
      contentSettingsResponse.imageCallback.details.setting === "block" &&
      contentSettingsResponse.imageCallback.lastError === "" &&
      contentSettingsResponse.imageAfterClear &&
      contentSettingsResponse.imageAfterClear.setting === "allow" &&
      contentSettingsResponse.notificationRegular &&
      contentSettingsResponse.notificationRegular.setting === "ask" &&
      contentSettingsResponse.notificationIncognito &&
      contentSettingsResponse.notificationIncognito.setting === "allow" &&
      Array.isArray(contentSettingsResponse.notificationResources) &&
      Array.isArray(contentSettingsResponse.pluginResources) &&
      contentSettingsResponse.pluginFixed &&
      contentSettingsResponse.pluginFixed.setting === "block" &&
      contentSettingsResponse.fullscreenFixed &&
      contentSettingsResponse.fullscreenFixed.setting === "allow" &&
      String(contentSettingsResponse.invalidError || "").includes("Invalid javascript") &&
      historyResponse &&
      Array.isArray(historyResponse.search) &&
      historyResponse.search.some((item) =>
        item &&
        item.url === historyResponse.url &&
        item.title === historyResponse.title &&
        Number(item.visitCount) >= 1 &&
        Number(item.lastVisitTime) > 0) &&
      Array.isArray(historyResponse.visits) &&
      historyResponse.visits.some((visit) =>
        visit &&
        Number(visit.visitTime) > 0 &&
        visit.transition === "link") &&
      Array.isArray(historyResponse.topSites) &&
      Array.isArray(historyResponse.afterDelete) &&
      !historyResponse.afterDelete.some((item) =>
        item && item.url === historyResponse.url) &&
      Array.isArray(historyResponse.visited) &&
      historyResponse.visited.some((item) =>
        item &&
        item.url === historyResponse.url &&
        item.title === historyResponse.title) &&
      Array.isArray(historyResponse.removed) &&
      historyResponse.removed.some((info) =>
        info &&
        info.allHistory === false &&
        Array.isArray(info.urls) &&
        info.urls.includes(historyResponse.url)) &&
      cookiesResponse &&
      cookiesResponse.setCookie &&
      cookiesResponse.setCookie.name === cookiesResponse.name &&
      cookiesResponse.setCookie.value === cookiesResponse.value &&
      cookiesResponse.getCookie &&
      cookiesResponse.getCookie.name === cookiesResponse.name &&
      cookiesResponse.getCookie.value === cookiesResponse.value &&
      Array.isArray(cookiesResponse.allCookies) &&
      cookiesResponse.allCookies.some((cookie) =>
        cookie &&
        cookie.name === cookiesResponse.name &&
        cookie.value === cookiesResponse.value) &&
      Array.isArray(cookiesResponse.stores) &&
      cookiesResponse.stores.some((store) => store && store.id === "0") &&
      cookiesResponse.removedCookie &&
      cookiesResponse.removedCookie.name === cookiesResponse.name &&
      cookiesResponse.afterRemove === null &&
      Array.isArray(cookiesResponse.changed) &&
      cookiesResponse.changed.some((change) =>
        change &&
        change.removed === false &&
        change.cookie &&
        change.cookie.name === cookiesResponse.name) &&
      cookiesResponse.changed.some((change) =>
        change &&
        change.removed === true &&
        change.cookie &&
        change.cookie.name === cookiesResponse.name) &&
      browsingDataResponse &&
      browsingDataResponse.settings &&
      browsingDataResponse.settings.dataRemovalPermitted &&
      browsingDataResponse.settings.dataRemovalPermitted.cookies === true &&
      browsingDataResponse.settings.dataRemovalPermitted.history === true &&
      browsingDataResponse.settings.dataRemovalPermitted.cache === false &&
      Array.isArray(browsingDataResponse.historyAfter) &&
      !browsingDataResponse.historyAfter.some((item) =>
        item && item.url === browsingDataResponse.historyUrl) &&
      browsingDataResponse.cookieAfter === null &&
      downloadsResponse &&
      typeof downloadsResponse.downloadId === "number" &&
      downloadsResponse.afterCancel &&
      downloadsResponse.afterCancel.id === downloadsResponse.downloadId &&
      downloadsResponse.afterCancel.state === "interrupted" &&
      Array.isArray(downloadsResponse.erased) &&
      downloadsResponse.erased.includes(downloadsResponse.downloadId) &&
      downloadsResponse.events &&
      Array.isArray(downloadsResponse.events.created) &&
      downloadsResponse.events.created.some((item) =>
        item && item.id === downloadsResponse.downloadId) &&
      Array.isArray(downloadsResponse.events.changed) &&
      downloadsResponse.events.changed.some((delta) =>
        delta &&
        delta.id === downloadsResponse.downloadId &&
        delta.state &&
        delta.state.current === "interrupted") &&
      Array.isArray(downloadsResponse.events.erased) &&
      downloadsResponse.events.erased.includes(downloadsResponse.downloadId) &&
      sessionsResponse &&
      Array.isArray(sessionsResponse.recentlyClosed) &&
      sessionsResponse.recentlyClosed.some((session) =>
        session &&
        session.tab &&
        session.tab.url === sessionsResponse.url &&
        typeof session.tab.sessionId === "string" &&
        session.tab.sessionId.length > 0) &&
      sessionsResponse.restored &&
      sessionsResponse.restored.tab &&
      sessionsResponse.restored.tab.url === sessionsResponse.url &&
      typeof sessionsResponse.restored.tab.id === "number" &&
      Array.isArray(sessionsResponse.devices) &&
      sessionsResponse.devices.length === 0 &&
      Array.isArray(sessionsResponse.changed) &&
      sessionsResponse.changed.length >= 2 &&
      managementResponse &&
      managementResponse.self &&
      managementResponse.self.id === extensionId &&
      managementResponse.self.enabled === true &&
      managementResponse.self.name === "Mori Extension Smoke" &&
      Array.isArray(managementResponse.allBefore) &&
      managementResponse.allBefore.some((item) =>
        item && item.id === extensionId && item.enabled === true) &&
      managementResponse.allBefore.some((item) =>
        item && item.id === companionExtensionId && item.enabled === true) &&
      managementResponse.companionBefore &&
      managementResponse.companionBefore.id === companionExtensionId &&
      managementResponse.companionBefore.enabled === true &&
      managementResponse.companionDisabled &&
      managementResponse.companionDisabled.id === companionExtensionId &&
      managementResponse.companionDisabled.enabled === false &&
      managementResponse.companionEnabled &&
      managementResponse.companionEnabled.id === companionExtensionId &&
      managementResponse.companionEnabled.enabled === true &&
      managementResponse.lifecycle &&
      managementResponse.lifecycle.uninstallURLSet === true &&
      managementResponse.lifecycle.runtimeId === companionExtensionId &&
      managementResponse.companionAfterUninstall === null &&
      managementResponse.uninstallTab &&
      managementResponse.uninstallTab.url === managementResponse.uninstallURL &&
      crxDownloadInstallResponse &&
      crxDownloadInstallResponse.installed &&
      crxDownloadInstallResponse.installed.id === crxExtensionId &&
      crxDownloadInstallResponse.installed.name === "Mori CRX Download Smoke" &&
      crxDownloadInstallResponse.installed.enabled === true &&
      String(crxDownloadInstallResponse.installed.description || "").includes(runId) &&
      crxDownloadInstallResponse.legacyScheme &&
      crxDownloadInstallResponse.legacyScheme.requested ===
        ("chrome-" + "extension://" + crxExtensionId + "/popup.html") &&
      crxDownloadInstallResponse.legacyScheme.url ===
        "mori-extension://" + crxExtensionId + "/popup.html" &&
      crxDownloadInstallResponse.afterUninstall === null &&
      offscreenResponse &&
      offscreenResponse.beforeHasDocument === false &&
      offscreenResponse.afterCreateHasDocument === true &&
      offscreenResponse.afterCloseHasDocument === false &&
      offscreenResponse.loaded &&
      offscreenResponse.loaded.offscreenLoaded === runId &&
      offscreenResponse.loaded.offscreenRuntimeId === extensionId &&
      offscreenResponse.loaded.offscreenURL === chrome.runtime.getURL("offscreen.html") &&
      Array.isArray(offscreenResponse.contexts) &&
      offscreenResponse.contexts.some((context) =>
        context &&
        context.contextType === chrome.runtime.ContextType.OFFSCREEN_DOCUMENT &&
        context.documentUrl === chrome.runtime.getURL("offscreen.html")) &&
      offscreenResponse.clipboard &&
      offscreenResponse.clipboard.hasPermissions === true &&
      offscreenResponse.clipboard.hasClipboardApi === true &&
      offscreenResponse.clipboard.execCommandWrapped === true &&
      offscreenResponse.clipboard.asyncRead === offscreenResponse.clipboardValue &&
      offscreenResponse.clipboard.copyCommand === true &&
      offscreenResponse.clipboard.pasteCommand === true &&
      offscreenResponse.clipboard.pasted === offscreenResponse.clipboardValue &&
      offscreenResponse.clipboard.finalRead === offscreenResponse.clipboardValue &&
      browserNamespaceResponse &&
      browserNamespaceResponse.runtimeId === extensionId &&
      browserNamespaceResponse.popupURL === chrome.runtime.getURL("popup.html") &&
      browserNamespaceResponse.stored &&
      browserNamespaceResponse.stored.browserNamespaceSmoke === runId &&
      browserNamespaceResponse.self &&
      browserNamespaceResponse.self.id === extensionId &&
      browserNamespaceResponse.hasStoragePermission === true &&
      browserNamespaceResponse.apiShape &&
      browserNamespaceResponse.apiShape.storageSet === true &&
      browserNamespaceResponse.apiShape.tabsQuery === true &&
      browserNamespaceResponse.apiShape.managementGetSelf === true &&
      browserNamespaceResponse.apiShape.permissionsContains === true &&
      Array.isArray(browserNamespaceResponse.activeTabs) &&
      Array.isArray(browserNamespaceResponse.popupContexts) &&
      browserNamespaceResponse.popupContexts.some((context) =>
        context &&
        context.contextType === chrome.runtime.ContextType.POPUP &&
        context.documentUrl === chrome.runtime.getURL("popup.html")) &&
      Array.isArray(webAccessibleResourcesResponse) &&
      webAccessibleResourcesResponse[0] &&
      webAccessibleResourcesResponse[0].result &&
      webAccessibleResourcesResponse[0].result.publicStatus === 200 &&
      webAccessibleResourcesResponse[0].result.publicText.trim() ===
        "public-web-accessible-" + runId &&
      webAccessibleResourcesResponse[0].result.privateResult &&
      webAccessibleResourcesResponse[0].result.privateResult.ok === false &&
      nativeMessagingResponse &&
      nativeMessagingResponse.single &&
      nativeMessagingResponse.single.ok === true &&
      nativeMessagingResponse.single.host === "com.mori.smoke" &&
      nativeMessagingResponse.single.origin === "mori-extension://" + extensionId + "/" &&
      nativeMessagingResponse.single.mode === "single" &&
      nativeMessagingResponse.single.echo &&
      nativeMessagingResponse.single.echo.runId === runId &&
      nativeMessagingResponse.single.echo.message === "native-message-smoke" &&
      nativeMessagingResponse.single.echo.nested &&
      nativeMessagingResponse.single.echo.nested.value === 42 &&
      nativeMessagingResponse.chromeCompatible &&
      nativeMessagingResponse.chromeCompatible.ok === true &&
      nativeMessagingResponse.chromeCompatible.host === "com.mori.smoke" &&
      String(nativeMessagingResponse.chromeCompatible.origin || "")
        .startsWith("chrome-" + "extension://" + extensionId + "/") &&
      nativeMessagingResponse.chromeCompatible.mode === "chrome-compatible" &&
      nativeMessagingResponse.chromeCompatible.echo &&
      nativeMessagingResponse.chromeCompatible.echo.runId === runId &&
      nativeMessagingResponse.chromeCompatible.echo.message ===
        "native-message-chrome-origin-smoke" &&
      nativeMessagingResponse.portMessage &&
      nativeMessagingResponse.portMessage.ok === true &&
      nativeMessagingResponse.portMessage.host === "com.mori.smoke" &&
      nativeMessagingResponse.portMessage.origin === "mori-extension://" + extensionId + "/" &&
      nativeMessagingResponse.portMessage.mode === "port" &&
      nativeMessagingResponse.portMessage.echo &&
      nativeMessagingResponse.portMessage.echo.runId === runId &&
      nativeMessagingResponse.portMessage.echo.message === "native-port-smoke" &&
      nativeMessagingResponse.portMessage.echo.nested &&
      nativeMessagingResponse.portMessage.echo.nested.value === 84 &&
      nativeMessagingResponse.disconnectCount >= 1 &&
      notificationsResponse &&
      notificationsResponse.permission === "granted" &&
      notificationsResponse.createdId === notificationsResponse.notificationId &&
      notificationsResponse.updated === true &&
      notificationsResponse.popupAll &&
      notificationsResponse.popupAll[notificationsResponse.notificationId] &&
      notificationsResponse.popupAll[notificationsResponse.notificationId].message ===
        "updated-" + runId &&
      notificationsResponse.backgroundBeforeClear &&
      notificationsResponse.backgroundBeforeClear.all &&
      notificationsResponse.backgroundBeforeClear.all[notificationsResponse.notificationId] &&
      notificationsResponse.backgroundBeforeClear.all[notificationsResponse.notificationId].message ===
        "updated-" + runId &&
      notificationsResponse.cleared === true &&
      notificationsResponse.popupAfterClear &&
      !notificationsResponse.popupAfterClear[notificationsResponse.notificationId] &&
      Array.isArray(notificationsResponse.popupClosedEvents) &&
      notificationsResponse.popupClosedEvents.some((event) =>
        event &&
        event.id === notificationsResponse.notificationId &&
        event.byUser === false) &&
      notificationsResponse.backgroundAfterClear &&
      Array.isArray(notificationsResponse.backgroundAfterClear.closedEvents) &&
      notificationsResponse.backgroundAfterClear.closedEvents.some((event) =>
        event &&
        event.notificationId === notificationsResponse.notificationId &&
        event.byUser === false) &&
      alarmsResponse &&
      alarmsResponse.popupCreated &&
      alarmsResponse.popupCreated.name === "mori-popup-alarm-" + runId &&
      Array.isArray(alarmsResponse.popupAll) &&
      alarmsResponse.popupAll.some((alarm) =>
        alarm && alarm.name === "mori-popup-alarm-" + runId) &&
      alarmsResponse.popupCleared === true &&
      alarmsResponse.popupMissingCleared === false &&
      alarmsResponse.backgroundStarted &&
      alarmsResponse.backgroundStarted.created &&
      alarmsResponse.backgroundStarted.created.name ===
        "mori-background-alarm-" + runId &&
      alarmsResponse.backgroundFired &&
      Array.isArray(alarmsResponse.backgroundFired.events) &&
      alarmsResponse.backgroundFired.events.some((alarm) =>
        alarm && alarm.name === "mori-background-alarm-" + runId) &&
      bookmarksResponse &&
      bookmarksResponse.first &&
      bookmarksResponse.first.url === "https://example.com/mori-bookmark-first-" + runId &&
      bookmarksResponse.second &&
      bookmarksResponse.second.index === 0 &&
      bookmarksResponse.moved &&
      bookmarksResponse.moved.id === bookmarksResponse.first.id &&
      bookmarksResponse.moved.index === 0 &&
      bookmarksResponse.updated &&
      bookmarksResponse.updated.title === "Mori Bookmark Updated " + runId &&
      bookmarksResponse.updated.url ===
        "https://example.com/mori-bookmark-first-" + runId + "?updated=1" &&
      Array.isArray(bookmarksResponse.fetched) &&
      bookmarksResponse.fetched.some((node) =>
        node &&
        node.id === bookmarksResponse.first.id &&
        node.url === bookmarksResponse.updated.url) &&
      Array.isArray(bookmarksResponse.children) &&
      bookmarksResponse.children.some((node) =>
        node && node.id === bookmarksResponse.first.id && node.index === 0) &&
      Array.isArray(bookmarksResponse.tree) &&
      bookmarksResponse.tree.length > 0 &&
      Array.isArray(bookmarksResponse.search) &&
      bookmarksResponse.search.some((node) =>
        node && node.id === bookmarksResponse.first.id) &&
      bookmarksResponse.events &&
      Array.isArray(bookmarksResponse.events.created) &&
      bookmarksResponse.events.created.some((event) =>
        event && event.id === bookmarksResponse.first.id) &&
      Array.isArray(bookmarksResponse.events.changed) &&
      bookmarksResponse.events.changed.some((event) =>
        event &&
        event.id === bookmarksResponse.first.id &&
        event.changeInfo &&
        event.changeInfo.title === "Mori Bookmark Updated " + runId) &&
      Array.isArray(bookmarksResponse.events.moved) &&
      bookmarksResponse.events.moved.some((event) =>
        event &&
        event.id === bookmarksResponse.first.id &&
        event.moveInfo &&
        event.moveInfo.index === 0 &&
        event.moveInfo.oldIndex === 1) &&
      Array.isArray(bookmarksResponse.events.removed) &&
      bookmarksResponse.events.removed.some((event) =>
        event && event.id === bookmarksResponse.first.id) &&
      captureVisibleTabResponse &&
      captureVisibleTabResponse.dataUrlPrefix === "data:image/png;base64,iVBORw0KGg" &&
      captureVisibleTabResponse.imageInfo &&
      captureVisibleTabResponse.imageInfo.width > 1 &&
      captureVisibleTabResponse.imageInfo.height > 1 &&
      captureVisibleTabResponse.imageInfo.byteLength > 100 &&
      Array.isArray(captureVisibleTabResponse.imageInfo.sample) &&
      Math.abs(captureVisibleTabResponse.imageInfo.sample[0] - 42) <= 12 &&
      Math.abs(captureVisibleTabResponse.imageInfo.sample[1] - 171) <= 12 &&
      Math.abs(captureVisibleTabResponse.imageInfo.sample[2] - 91) <= 12 &&
      captureVisibleTabResponse.imageInfo.sample[3] === 255 &&
      identityResponse &&
      identityResponse.result === identityResponse.expected &&
      sidePanelResponse &&
      sidePanelResponse.behavior &&
      sidePanelResponse.behavior.openPanelOnActionClick === true &&
      sidePanelResponse.options &&
      sidePanelResponse.options.enabled === true &&
      sidePanelResponse.options.path === "sidepanel.html" &&
      sidePanelResponse.loaded &&
      sidePanelResponse.loaded.sidePanelLoaded === runId &&
      sidePanelResponse.loaded.sidePanelRuntimeId === extensionId &&
      sidePanelResponse.loaded.sidePanelURL === chrome.runtime.getURL("sidepanel.html") &&
      protectedGlobalResponse &&
      protectedGlobalResponse.chromeStillReal === true &&
      protectedGlobalResponse.browserStillReal === true &&
      protectedGlobalResponse.runtimeId === extensionId &&
      protectedGlobalResponse.browserRuntimeId === extensionId &&
      protectedGlobalResponse.sendMessageType === "function" &&
      protectedGlobalResponse.response &&
      protectedGlobalResponse.response.pong === true;

	    document.body.textContent = ok
	      ? "Mori extension smoke ok: storage sync/session/managed/power/idle/system/search/dns/privacy/contentSettings/userScripts"
	      : "Mori extension smoke failed";
    console.info("__MORI_EXTENSION_SMOKE__" + JSON.stringify({
      runId,
      ok,
      checks,
      contexts,
      backgroundContexts,
      legacyExtensionViewsResponse,
      storage: {
        stored,
        storageKeys,
        storageBytes,
        storageConstants,
        storageAreasResponse,
        storageEvents
      },
      userSettings,
      badgeText,
      badgeBackgroundColor,
      badgeTextColor,
      actionEnabled,
      actionDisabled,
      openPopupResult,
      response,
      commandResponse,
      portResponse,
      externalMessageResponse,
      extensionFetchResponse,
      contentResponse,
      executeScriptResponse,
      cssInjectionResponse,
      dynamicContentResponse,
      userScriptsResponse,
      webRequestResponse,
      webNavigationResponse,
      dnrResponse,
      tabManagementResponse,
      tabGroupsResponse,
      contextMenusResponse,
      windowsResponse,
	      permissionsResponse,
	      proxySettingsResponse,
	      privacyResponse,
	      contentSettingsResponse,
      historyResponse,
      cookiesResponse,
      browsingDataResponse,
      downloadsResponse,
      sessionsResponse,
      managementResponse,
      crxDownloadInstallResponse,
      offscreenResponse,
      browserNamespaceResponse,
      webAccessibleResourcesResponse,
      nativeMessagingResponse,
      notificationsResponse,
      alarmsResponse,
      bookmarksResponse,
      captureVisibleTabResponse,
      identityResponse,
      sidePanelResponse,
      runtimeIntrospectionResponse,
      powerResponse,
	      idleResponse,
	      systemResponse,
	      searchResponse,
	      dnsResponse,
	      protectedGlobalResponse,
      href: location.href
    }));
  } catch (error) {
    document.body.textContent = "Mori extension smoke failed";
    console.error("__MORI_EXTENSION_SMOKE__" + JSON.stringify({
      runId,
      ok: false,
      error: error && error.message || String(error),
      href: location.href
    }));
  }
})();
JS

  escaped_dir="$(json_escape "$smoke_dir")"
  escaped_companion_dir="$(json_escape "$smoke_companion_dir")"
  SMOKE_EXTENSION_ID="$smoke_id"
  SMOKE_EXTENSION_DIR="$smoke_dir"
  SMOKE_NATIVE_MESSAGING_HOSTS_DIR="$native_hosts_dir"
  SMOKE_EXTENSION_CATALOG="[{\"id\":\"$smoke_id\",\"name\":\"Mori Extension Smoke\",\"version\":\"1.0.0\",\"detail\":\"Mori extension runtime smoke test.\",\"path\":\"$escaped_dir\",\"iconPath\":null,\"popupPage\":\"popup.html\",\"optionsPage\":null,\"enabled\":true,\"pinned\":false},{\"id\":\"$smoke_companion_id\",\"name\":\"Mori Smoke Companion\",\"version\":\"1.0.0\",\"detail\":\"Second extension for management API smoke coverage.\",\"path\":\"$escaped_companion_dir\",\"iconPath\":null,\"popupPage\":null,\"optionsPage\":null,\"enabled\":true,\"pinned\":false}]"
}

free_tcp_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

start_smoke_download_server() {
  local port="$1"
  local run_id="$2"
  local crx_path="$3"
  python3 - "$port" "$run_id" "$crx_path" >"$ROOT_DIR/build/extension-smoke/download-server.log" 2>&1 <<'PY' &
import http.server
import json
import os
import urllib.parse
import socketserver
import sys
import time

port = int(sys.argv[1])
run_id = sys.argv[2]
crx_path = sys.argv[3]

class Server(socketserver.TCPServer):
    allow_reuse_address = True

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        return

    def trace_cors(self, label):
        print(
            f"{label} {self.command} {self.path} "
            f"origin={self.headers.get('Origin')} "
            f"header={self.headers.get('X-Mori-CORS-Smoke')} "
            f"acr_method={self.headers.get('Access-Control-Request-Method')} "
            f"acr_headers={self.headers.get('Access-Control-Request-Headers')}",
            file=sys.stderr,
            flush=True)

    def send_cors_json(self):
        self.trace_cors("cors-json")
        length = int(self.headers.get("Content-Length") or "0")
        raw_body = self.rfile.read(length) if length > 0 else b""
        parsed_body = None
        if raw_body:
            try:
                parsed_body = json.loads(raw_body.decode("utf-8"))
            except Exception:
                parsed_body = raw_body.decode("utf-8", "replace")
        payload = {
            "ok": True,
            "runId": run_id,
            "method": self.command,
            "path": self.path,
            "phase": urllib.parse.parse_qs(
                urllib.parse.urlparse(self.path).query).get("phase", [None])[0],
            "origin": self.headers.get("Origin"),
            "header": self.headers.get("X-Mori-CORS-Smoke"),
            "cookie": self.headers.get("Cookie"),
            "contentType": self.headers.get("Content-Type"),
            "body": parsed_body
        }
        data = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_OPTIONS(self):
        if self.path.startswith("/cors-json"):
            self.trace_cors("cors-options-not-intercepted")
            self.send_response(418)
            self.send_header("Content-Type", "text/plain")
            self.end_headers()
            self.wfile.write(b"Mori should intercept extension preflights")
            return
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        if self.path.startswith("/cors-json"):
            self.send_cors_json()
            return
        self.send_response(404)
        self.end_headers()

    def do_GET(self):
        if self.path.startswith("/extension.crx"):
            try:
                with open(crx_path, "rb") as handle:
                    data = handle.read()
            except OSError:
                self.send_response(404)
                self.end_headers()
                return
            self.send_response(200)
            self.send_header("Content-Type", "application/x-chrome-extension")
            self.send_header("Content-Length", str(len(data)))
            self.send_header(
                "Content-Disposition",
                'attachment; filename="' + os.path.basename(crx_path) + '"')
            self.end_headers()
            self.wfile.write(data)
            return
        if self.path.startswith("/frame-smoke.html"):
            body = f"""<!doctype html>
<meta charset=\"utf-8\">
<title>Mori Smoke Child Frame</title>
<body data-mori-smoke-frame=\"{run_id}\">
  <input id=\"mori-smoke-frame-input\" name=\"frame-email\" autocomplete=\"username\">
</body>
"""
            data = body.encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        if self.path.startswith("/external-message.html"):
            body = f"""<!doctype html>
<meta charset=\"utf-8\">
<title>external-pending</title>
<body>External Mori smoke</body>
<script>
const extensionId = \"mori-smoke-extension\";
const runId = {run_id!r};
async function go() {{
  try {{
    const response = await chrome.runtime.sendMessage(extensionId, {{
      type: \"fork\",
      runId,
      external: true
    }});
    document.body.dataset.result = JSON.stringify(response);
    document.title = \"external-ok\";
  }} catch (error) {{
    document.body.dataset.error = error && error.message || String(error);
    document.title = \"external-fail\";
  }}
}}
setTimeout(go, 25);
</script>
"""
            data = body.encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        if self.path.startswith("/cors-json"):
            self.send_cors_json()
            return
        if self.path.startswith("/dnr-response"):
            body = ("mori-dnr-response-" + run_id).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.send_header("X-Mori-DNR-Original", "original-" + run_id)
            self.send_header("X-Mori-DNR-Remove", "remove-" + run_id)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if not self.path.startswith("/slow.bin"):
            self.send_response(404)
            self.end_headers()
            return
        total = 8 * 1024 * 1024
        chunk = (("mori-download-smoke-" + run_id + "\n").encode("utf-8") * 1024)[:32768]
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(total))
        self.send_header("Content-Disposition", "attachment; filename=\"mori-smoke.bin\"")
        self.end_headers()
        sent = 0
        try:
            while sent < total:
                size = min(len(chunk), total - sent)
                self.wfile.write(chunk[:size])
                self.wfile.flush()
                sent += size
                time.sleep(0.02)
        except (BrokenPipeError, ConnectionResetError):
            pass

with Server(("127.0.0.1", port), Handler) as httpd:
    httpd.serve_forever()
PY
  SMOKE_DOWNLOAD_SERVER_PID="$!"
  for _ in {1..50}; do
    if python3 - "$port" 2>/dev/null <<'PY'
import socket, sys
with socket.socket() as s:
    s.settimeout(0.1)
    s.connect(("127.0.0.1", int(sys.argv[1])))
PY
    then
      return 0
    fi
    sleep 0.1
  done
  echo "verify failed: slow download server did not start." >&2
  return 1
}

stop_smoke_download_server() {
  if [[ -n "${SMOKE_DOWNLOAD_SERVER_PID:-}" ]]; then
    /bin/kill "$SMOKE_DOWNLOAD_SERVER_PID" 2>/dev/null || true
    wait "$SMOKE_DOWNLOAD_SERVER_PID" 2>/dev/null || true
    SMOKE_DOWNLOAD_SERVER_PID=""
  fi
}

launch_smoke_app() {
  local start_url="$1"
  local result_path="$2"
  MORI_EXTENSION_CATALOG_JSON="$SMOKE_EXTENSION_CATALOG" \
    MORI_START_URL="$start_url" \
    MORI_EXTENSION_SMOKE_RESULT_PATH="$result_path" \
    MORI_CHROMIUM_ENGINE_AUDIT_PATH="$SMOKE_ENGINE_AUDIT_PATH" \
    MORI_NATIVE_MESSAGING_HOSTS_DIR="$SMOKE_NATIVE_MESSAGING_HOSTS_DIR" \
    MORI_EXTENSION_SMOKE_COMMAND_ID="$SMOKE_EXTENSION_ID" \
    MORI_EXTENSION_SMOKE_COMMAND_NAME="_execute_action" \
    MORI_EXTENSION_SMOKE_EXTRA_COMMAND_NAME="mori-smoke-command" \
    "$APP_BUNDLE/Contents/MacOS/$APP_NAME" \
    >"$ROOT_DIR/build/extension-smoke/mori-smoke-app.log" 2>&1 &
  SMOKE_APP_PID="$!"
}

wait_for_extension_smoke() {
  local run_id="$1"
  local result_path="$2"
  local attempt max_attempts logs api_popup_logs command_popup_logs all_popup_logs smoke_status
  max_attempts="${MORI_EXTENSION_SMOKE_WAIT_ATTEMPTS:-720}"
  for ((attempt = 1; attempt <= max_attempts; attempt += 1)); do
    if [[ -f "$result_path" ]]; then
      logs="$(/bin/cat "$result_path")"
    else
      logs="$(/usr/bin/log show --last 3m --style compact \
        --predicate "process == \"$APP_NAME\" AND eventMessage CONTAINS \"$run_id\" AND eventMessage CONTAINS \"__MORI_EXTENSION_SMOKE__\"" \
        2>/dev/null || true)"
    fi
    smoke_status="$(printf '%s\n' "$logs" | python3 -c '
import json
import sys

run_id = sys.argv[1]
marker = "__MORI_EXTENSION_SMOKE__"
for line in sys.stdin:
    index = line.find(marker)
    if index < 0:
        continue
    raw = line[index + len(marker):].strip()
    try:
        payload = json.loads(raw)
    except Exception:
        continue
    if str(payload.get("runId", "")) != run_id:
        continue
    if payload.get("ok") is True:
        print("ok")
        break
    if payload.get("ok") is False:
        print("fail")
        break
' "$run_id")"
    if [[ "$smoke_status" == "ok" ]]; then
      all_popup_logs="$(/usr/bin/grep -F "__MORI_EXTENSION_POPUP_OPENED__ $SMOKE_EXTENSION_ID" "$ROOT_DIR/build/extension-smoke/mori-smoke-app.log" 2>/dev/null || true)"
      if [[ -z "$all_popup_logs" ]]; then
        all_popup_logs="$(/usr/bin/log show --last 3m --style compact \
          --predicate "process == \"$APP_NAME\" AND eventMessage CONTAINS \"__MORI_EXTENSION_POPUP_OPENED__\" AND eventMessage CONTAINS \"$SMOKE_EXTENSION_ID\"" \
          2>/dev/null || true)"
      fi
      api_popup_logs="$(printf '%s\n' "$all_popup_logs" | /usr/bin/grep -F " api" || true)"
      command_popup_logs="$(printf '%s\n' "$all_popup_logs" | /usr/bin/grep -F " command" || true)"
      if [[ -n "$api_popup_logs" && -n "$command_popup_logs" ]]; then
        printf 'Verified extension runtime smoke: popup rendered, extension pages report navigator.onLine=true, extension-page fetch POST/GET returned Response JSON through Mori-owned networking with CEF cookie sharing, externally_connectable account-page fork messages reached onMessageExternal with a web sender tab, proxy.settings get/set/clear/onChange matched ChromeSetting semantics, chrome.power system/display keep-awake assertions acquired and released via macOS activity assertions, chrome.system cpu/memory/display/storage metadata and storage capacity/eject result shapes round-tripped from macOS, chrome.search.query loaded configured-provider searches into tabId/current/new-tab/browser namespace paths, chrome.contentSettings get/set/clear/getResourceIdentifiers persisted extension-owned rules with primary/secondary precedence, chrome.userScripts register/get/update/unregister/configureWorld/execute injected inline user scripts into Mori tabs, popup/background/offscreen runtime.getContexts, browser.* namespace mirrors Mori extension APIs, extension window creation stayed inside Mori tabs, web_accessible_resources exposed only declared extension files, webNavigation.getFrame/getAllFrames returned stable frame and document ids, scripting.executeScript returned per-frame allFrames/frameIds/documentIds results, manifest world MAIN content scripts stayed page-world, scripting.insertCSS/removeCSS and legacy tabs CSS APIs changed and restored Chromium-rendered page styles, frame-targeted scripting.insertCSS/removeCSS stayed isolated to the requested child frame, MV3 offscreen document create/close plus offscreen clipboard read/write/copy/paste worked in Mori, sidePanel.open rendered an extension page in Mori chrome, action.openPopup and _execute_action reached Mori chrome for an unpinned extension, identity.launchWebAuthFlow captured a Mori tab redirect, cookies/history/topSites, bookmarks create/update/move/remove with events, tabs.group/ungroup and tabGroups.* events round-tripped, contextMenus create/update/remove and onClicked(info, tab) checkbox state round-tripped, DNR block/redirect/request-header and response-header modifications round-tripped, sessions restore, management enable/disable/uninstall, CRX3 header id parsing installed a generically named package into Mori without external Chrome handoff, legacy extension-scheme navigations rewrote into Mori tabs, runtime.getVersion/getPlatformInfo/requestUpdateCheck/restartAfterDelay/reload shape, runtime.setUninstallURL plus management.uninstallSelf opened a Mori tab, runtime.sendNativeMessage and runtime.connectNative round-tripped through a native host, notifications shared across extension contexts, alarms fired in the extension background context, tabs.captureVisibleTab captured real page pixels, browsingData history/cookie removal, downloads.cancel round-tripped through Mori-owned stores, Proton-style protected global chrome/browser replacement was ignored while runtime stayed usable, dynamic content-script file injection recovered chrome.runtime after protected globals, content scripts manipulated focused page inputs and delivered sender.tab/sender.frameId/documentId to background messages and ports, tabs.sendMessage honored frameId/documentId-targeted child-frame delivery, and runtime.sendMessage, runtime.connect, content script injection, and tabs.sendMessage round-tripped through Mori.\n'
        return 0
      fi
    fi
    if [[ "$smoke_status" == "fail" ]]; then
      echo "verify failed: extension runtime smoke reported failure:" >&2
      printf '%s\n' "$logs" >&2
      return 1
    fi
    sleep 0.25
  done

  echo "verify failed: extension runtime smoke did not report success after $((max_attempts / 4))s." >&2
  echo "Expected native popup-open markers: __MORI_EXTENSION_POPUP_OPENED__ $SMOKE_EXTENSION_ID api and command" >&2
  if [[ -f "$result_path" ]]; then
    echo "Smoke result file:" >&2
    /bin/cat "$result_path" >&2 || true
  fi
  echo "Last app stderr/stdout:" >&2
  /usr/bin/tail -80 "$ROOT_DIR/build/extension-smoke/mori-smoke-app.log" >&2 || true
  return 1
}

wait_for_engine() {
  local attempt ps_output mori_processes helper_processes
  for attempt in {1..40}; do
    ps_output="$(/bin/ps ax -o pid=,ppid=,args=)"
    mori_processes="$(printf '%s\n' "$ps_output" | /usr/bin/grep -F "$APP_BUNDLE/Contents" || true)"
    helper_processes="$(printf '%s\n' "$mori_processes" | /usr/bin/grep -F "Mori Helper" || true)"
    if [[ -n "$mori_processes" && -n "$helper_processes" ]]; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

audit_source_contract() {
  local forbidden_source

  forbidden_source="$(/usr/bin/grep -REn \
    'CreateTopLevelWindow|CEF_RUNTIME_STYLE_CHROME|--load-extension|chrome-extension://|Google Chrome\.app|Chrome\.app' \
    Sources Scripts run.sh project.yml Mori.xcodeproj 2>/dev/null || true)"
  if [[ -n "$forbidden_source" ]]; then
    echo "verify failed: source contains Chrome-owned runtime or extension-launch surface:" >&2
    printf '%s\n' "$forbidden_source" >&2
    exit 1
  fi

  if ! /usr/bin/grep -REn 'SetAsChild\(' Sources/Bridge >/dev/null; then
    echo "verify failed: embedded CEF child-view creation was not found" >&2
    exit 1
  fi

  if ! /usr/bin/grep -REn 'OnBeforePopup' Sources/App/BrowserClient.mm >/dev/null ||
     ! /usr/bin/grep -REn 'return true;[[:space:]]*// Cancel the popup' Sources/App/BrowserClient.mm >/dev/null ||
     ! /usr/bin/grep -REn '_requestNewTab' Sources/Bridge >/dev/null; then
    echo "verify failed: CEF popup routing is not proven to stay inside Mori tabs" >&2
    exit 1
  fi
}

audit_bundle() {
  local cef_framework main_executable helper_executable

  cef_framework="$APP_BUNDLE/Contents/Frameworks/Chromium Embedded Framework.framework"
  main_executable="$APP_BUNDLE/Contents/MacOS/$APP_NAME"
  helper_executable="$APP_BUNDLE/Contents/Frameworks/Mori Helper.app/Contents/MacOS/Mori Helper"

  if [[ ! -d "$cef_framework" ]]; then
    echo "verify failed: Chromium Embedded Framework is not bundled in Mori.app" >&2
    exit 1
  fi

  if [[ ! -x "$main_executable" ]]; then
    echo "verify failed: Mori app executable is missing" >&2
    exit 1
  fi

  if [[ ! -x "$helper_executable" ]]; then
    echo "verify failed: bundled CEF helper executable is missing" >&2
    exit 1
  fi
}

audit_engine() {
  local ps_output mori_processes helper_processes forbidden

  ps_output="$(/bin/ps ax -o pid=,ppid=,args=)"
  mori_processes="$(printf '%s\n' "$ps_output" | /usr/bin/grep -F "$APP_BUNDLE/Contents" || true)"
  helper_processes="$(printf '%s\n' "$mori_processes" | /usr/bin/grep -F "Mori Helper" || true)"

  if [[ -z "$mori_processes" ]]; then
    echo "verify failed: no running processes from $APP_BUNDLE" >&2
    exit 1
  fi

  if [[ -z "$helper_processes" ]]; then
    echo "verify failed: no bundled Mori Helper CEF processes are running" >&2
    exit 1
  fi

  if ! printf '%s\n' "$helper_processes" | /usr/bin/grep -F "Chromium Embedded Framework.framework" >/dev/null; then
    echo "verify failed: Mori helpers are not using the bundled Chromium Embedded Framework" >&2
    exit 1
  fi

  forbidden="$(printf '%s\n' "$mori_processes" \
    | /usr/bin/egrep 'Google Chrome|Chrome\.app|CEF_RUNTIME_STYLE_CHROME|--load-extension|chrome-extension://' || true)"
  if [[ -n "$forbidden" ]]; then
    echo "verify failed: Chrome-owned process or extension-launch flag found in Mori runtime:" >&2
    printf '%s\n' "$forbidden" >&2
    exit 1
  fi

  printf 'Verified standalone Mori runtime: bundled CEF helpers, no Google Chrome process path, no Chrome extension launch flags.\n'
}

audit_chromium_engine_log() {
  local marker attempt

  if [[ -z "$SMOKE_ENGINE_AUDIT_PATH" ]]; then
    echo "verify failed: Mori engine audit path was not configured" >&2
    exit 1
  fi

  marker=""
  for attempt in {1..40}; do
    marker="$(/usr/bin/grep -F "__MORI_CHROMIUM_ENGINE__" "$SMOKE_ENGINE_AUDIT_PATH" 2>/dev/null || true)"
    [[ -n "$marker" ]] && break
    sleep 0.25
  done
  if [[ -z "$marker" ]]; then
    echo "verify failed: Mori did not report an attached Chromium engine" >&2
    exit 1
  fi

  if printf '%s\n' "$marker" | /usr/bin/grep -F "runtime=chrome" >/dev/null; then
    echo "verify failed: Mori attached a Chrome-style CEF runtime:" >&2
    printf '%s\n' "$marker" >&2
    exit 1
  fi

  if ! printf '%s\n' "$marker" \
      | /usr/bin/grep -F "runtime=alloy embedding=child-view scheme=mori-extension" >/dev/null; then
    echo "verify failed: Mori did not prove Alloy child-view Chromium embedding:" >&2
    printf '%s\n' "$marker" >&2
    exit 1
  fi
}

stop_app
build_app

case "$MODE" in
  run)
    open_app
    ;;
  --debug|debug)
    /usr/bin/lldb -- "$APP_BUNDLE/Contents/MacOS/$APP_NAME"
    ;;
  --logs|logs)
    open_app
    /usr/bin/log stream --info --style compact --predicate "process == \"$APP_NAME\""
    ;;
  --telemetry|telemetry)
    open_app
    /usr/bin/log stream --info --style compact --predicate 'subsystem BEGINSWITH "com.mori.browser"'
    ;;
  --verify|verify)
    audit_source_contract
    audit_bundle
    record_external_chrome_state
    open_app
    if ! wait_for_engine; then
      echo "verify failed: Mori did not launch bundled CEF helper processes from $APP_BUNDLE" >&2
      exit 1
    fi
    audit_engine
    audit_no_external_chrome_spawned
    ;;
  --verify-extension-smoke|verify-extension-smoke)
    audit_source_contract
    audit_bundle
    SMOKE_RUN_ID="$(/bin/date +%s)-$$"
	    SMOKE_RESULT_PATH="$ROOT_DIR/build/extension-smoke/result-$SMOKE_RUN_ID.json"
	    SMOKE_ENGINE_AUDIT_PATH="$ROOT_DIR/build/extension-smoke/engine-$SMOKE_RUN_ID.log"
	    : >"$SMOKE_ENGINE_AUDIT_PATH"
	    SMOKE_DOWNLOAD_PORT="$(free_tcp_port)"
	    SMOKE_DOWNLOAD_URL="http://127.0.0.1:$SMOKE_DOWNLOAD_PORT/slow.bin?run=$SMOKE_RUN_ID"
	    SMOKE_CRX_EXTENSION_ID="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
	    SMOKE_CRX_PATH="$ROOT_DIR/build/extension-smoke/header-id-extension.crx"
	    SMOKE_CRX_URL="http://127.0.0.1:$SMOKE_DOWNLOAD_PORT/extension.crx?run=$SMOKE_RUN_ID"
	    SMOKE_EXTERNAL_PAGE_URL="http://127.0.0.1:$SMOKE_DOWNLOAD_PORT/external-message.html?run=$SMOKE_RUN_ID"
	    SMOKE_CORS_URL="http://127.0.0.1:$SMOKE_DOWNLOAD_PORT/cors-json?run=$SMOKE_RUN_ID"
	    SMOKE_FRAME_URL="http://127.0.0.1:$SMOKE_DOWNLOAD_PORT/frame-smoke.html?run=$SMOKE_RUN_ID"
	    create_extension_smoke_fixture "$SMOKE_RUN_ID"
	    if ! start_smoke_download_server "$SMOKE_DOWNLOAD_PORT" "$SMOKE_RUN_ID" "$SMOKE_CRX_PATH"; then
	      exit 1
	    fi
    record_external_chrome_state
    launch_smoke_app "mori-extension://$SMOKE_EXTENSION_ID/popup.html" "$SMOKE_RESULT_PATH"
    if ! wait_for_engine; then
      echo "verify failed: Mori did not launch bundled CEF helper processes from $APP_BUNDLE" >&2
      stop_app
      stop_smoke_download_server
      exit 1
    fi
    audit_engine
    audit_chromium_engine_log
    audit_no_external_chrome_spawned
    if ! wait_for_extension_smoke "$SMOKE_RUN_ID" "$SMOKE_RESULT_PATH"; then
      stop_app
      stop_smoke_download_server
      exit 1
    fi
    stop_app
    stop_smoke_download_server
    ;;
  *)
    echo "usage: $0 [run|--debug|--logs|--telemetry|--verify|--verify-extension-smoke]" >&2
    exit 2
    ;;
esac
