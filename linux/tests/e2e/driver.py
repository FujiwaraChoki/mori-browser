"""Drive a real Mori browser in an isolated headless sway, for end-to-end tests.

Everything goes through linux/harness (start.sh, run.sh, ptr.sh, key.sh, shot.sh,
stop.sh), so the guards in run.sh always apply. Python standard library only.

    b = Browser("demo", port=21500, root="/tmp/some-run-dir")
    try:
        b.start(b.fixture.page("one"))
        assert b.tabs()[0]["title"] == "one"
    finally:
        b.teardown()

See README.md. run.py creates and tears down the Browser for a test.
"""
from __future__ import annotations

import base64
import http.server
import json
import os
import re
import signal
import socket
import struct
import subprocess
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import zlib
from pathlib import Path

E2E_DIR = Path(__file__).resolve().parent
REPO = E2E_DIR.parents[2]
HARNESS = REPO / "linux" / "harness"
DEFAULT_OUT = REPO / "ungoogled-chromium-linux" / "build" / "src" / "out" / "Default"

GUARD_LOG = "e2e-guard-violations.log"   # Survives a test process that dies.

# chrome.log lines that mean a process went down, not ordinary log noise.
CRASH_LOG_RE = re.compile(
    r"\bFATAL:|Check failed|DCHECK failed|Received signal \d+|Segmentation fault|\*\*\* stack smashing")

# Variables that could point a tool at the user's real desktop. sway inherits
# what start.sh is given and passes it on to everything it runs, so they go here.
_SCRUB_NAMES = {
    "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DISPLAY", "XAUTHORITY", "SWAYSOCK", "I3SOCK",
    "DBUS_SESSION_BUS_ADDRESS", "XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID",
    "XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "XDG_SESSION_TYPE", "DESKTOP_SESSION",
}
_SCRUB_PREFIXES = ("HYPRLAND_", "HYPRCURSOR_", "UWSM_")


class DriverError(Exception):
    """The test could not be carried out. Never absorbed by @xfail."""


class GuardViolation(DriverError):
    """A test browser reached something outside its sandbox."""


class Skip(Exception):
    """Raise from a test to skip it, with the reason as the message."""


def safe_env(**extra) -> dict:
    env = {k: v for k, v in os.environ.items()
           if k not in _SCRUB_NAMES and not k.startswith(_SCRUB_PREFIXES)}
    env.update(extra)
    return env


def real_wayland_display():
    """The user's real display. run.py scrubs it, so it hands the name over."""
    return os.environ.get("MORI_E2E_REAL_WAYLAND") or os.environ.get("WAYLAND_DISPLAY")


def poll(fn, timeout=10.0, interval=0.2):
    """Call fn until it returns something truthy; return the last result."""
    deadline = time.monotonic() + timeout
    while True:
        result = fn()
        if result or time.monotonic() >= deadline:
            return result
        time.sleep(interval)


def expect(fn, timeout=10.0, message="condition not met"):
    """Assert (test failure) that fn becomes truthy. Returns its result."""
    result = poll(fn, timeout)
    assert result, f"{message} (after {timeout}s, last result: {result!r})"
    return result


def require(fn, timeout=10.0, message="precondition not met"):
    """Like expect, but a miss is a DriverError: the setup broke, so @xfail
    does not hide it behind the known bug."""
    result = poll(fn, timeout)
    if not result:
        raise DriverError(f"{message} (after {timeout}s, last result: {result!r})")
    return result


# --- Test decorators ---------------------------------------------------------

def tags(*names):
    """@tags("smoke"): run.py -k smoke selects these."""
    def apply(fn):
        fn.e2e_tags = tuple(getattr(fn, "e2e_tags", ())) + names
        return fn
    return apply


def xfail(reason):
    """A known bug. The test must fail with an assertion. Setup errors, crashes
    and guard violations still fail it, and passing is reported as XPASS and fails
    the run, so the marker gets removed when the fix lands."""
    def apply(fn):
        fn.e2e_xfail = reason
        return fn
    return apply


def timeout(seconds):
    """Override run.py's per-test timeout."""
    def apply(fn):
        fn.e2e_timeout = seconds
        return fn
    return apply


def allow_blocked(*patterns):
    """Regexes for $HL/desktop-blocked.log lines this test expects to cause."""
    def apply(fn):
        fn.e2e_allow_blocked = tuple(getattr(fn, "e2e_allow_blocked", ())) + patterns
        return fn
    return apply


# --- Processes ---------------------------------------------------------------

def _cmdline(pid) -> str:
    """argv as one string. Chromium retitles its processes, which joins their
    argv with spaces in /proc, so it is not always NUL-separated."""
    try:
        return Path(f"/proc/{pid}/cmdline").read_bytes().decode(errors="replace").replace("\0", " ")
    except OSError:
        return ""


def pids_for_profile(profile) -> list[int]:
    """Every process started with exactly --user-data-dir=<profile>."""
    token = re.compile(re.escape(f"--user-data-dir={profile}") + r"(?: |$)")
    return [int(entry) for entry in os.listdir("/proc")
            if entry.isdigit() and token.search(_cmdline(entry))]


def browser_pid_for_profile(profile):
    """The main browser process: the profile's process without --type=."""
    for pid in pids_for_profile(profile):
        if "--type=" not in _cmdline(pid):
            return pid
    return None


def process_tree(pid) -> set[int]:
    seen, todo = set(), [pid]
    while todo:
        current = todo.pop()
        if current in seen:
            continue
        seen.add(current)
        try:
            for task in os.listdir(f"/proc/{current}/task"):
                children = Path(f"/proc/{current}/task/{task}/children").read_text()
                todo.extend(int(child) for child in children.split())
        except OSError:
            pass
    return seen


def alive(pid) -> bool:
    """True for a running process; a zombie has already exited."""
    try:
        stat = Path(f"/proc/{pid}/stat").read_text()
    except OSError:
        return False
    return stat.rsplit(")", 1)[1].split()[0] != "Z"


# --- Desktop guard (ss) ------------------------------------------------------

_SS_ROW = re.compile(
    r"^(?P<net>\S+)\s+(?P<state>\S+)\s+\d+\s+\d+\s+(?P<local>\S+)\s+(?P<ino>\d+|\*)\s+"
    r"(?P<peer>\S+)\s+(?P<peer_ino>\d+|\*)(?:\s+users:\((?P<users>.*)\))?\s*$")
_SS_PID = re.compile(r"pid=(\d+)")

FORBIDDEN_SOCKETS = (
    re.compile(r"^/run/user/\d+/bus$"),
    re.compile(r"^/run/user/\d+/at-spi/"),
    re.compile(r"^/run/user/\d+/hypr/"),
    re.compile(r"^/run/user/\d+/wayland-\d+$"),
    re.compile(r"^@?/tmp/\.X11-unix/X\d+$"),
)


def read_ss() -> str:
    result = subprocess.run(["ss", "-xapH"], env=safe_env(), capture_output=True, text=True, timeout=30)
    if result.returncode != 0:
        raise DriverError(f"ss failed: {result.stderr.strip()}")
    return result.stdout


def socket_connections(ss_output: str, pids) -> list[tuple[int, str]]:
    """(pid, path) for each named unix socket that one of pids is connected to.

    A client's row has no path of its own, only its peer's inode, so this also
    looks the peer up among the rows of the sockets the servers hold.
    """
    pids = set(pids)
    rows = [m.groupdict() for m in map(_SS_ROW.match, ss_output.splitlines()) if m]
    name_of_inode = {row["ino"]: row["local"] for row in rows
                     if row["local"] != "*" and row["ino"] != "*"}
    found = []
    for row in rows:
        owners = {int(p) for p in _SS_PID.findall(row["users"] or "")} & pids
        if not owners:
            continue
        name = row["peer"] if row["peer"] != "*" else name_of_inode.get(row["peer_ino"])
        if name:
            found.extend((owner, name) for owner in owners)
    return found


def forbidden_connections(connections, allowed=(), patterns=FORBIDDEN_SOCKETS):
    return sorted({(pid, path) for pid, path in connections
                   if path not in allowed and any(p.search(path) for p in patterns)})


def blocked_log_problems(path, allowed_patterns=()) -> list[str]:
    """Lines of a desktop-blocked.log that no pattern allows."""
    try:
        lines = Path(path).read_text(errors="replace").splitlines()
    except OSError:
        return []
    return [line for line in lines
            if line.strip() and not any(re.search(p, line) for p in allowed_patterns)]


# --- Images ------------------------------------------------------------------

class Image:
    """An RGB screenshot. Regions are (x, y, width, height), in screen pixels."""

    def __init__(self, width, height, rgb: bytes):
        self.width, self.height, self.rgb = width, height, rgb

    @classmethod
    def from_ppm(cls, data: bytes) -> "Image":
        match = re.match(rb"P6\s+(?:#[^\n]*\s+)*(\d+)\s+(\d+)\s+(\d+)\s", data)
        if not match:
            raise DriverError("not a binary PPM image")
        width, height = int(match[1]), int(match[2])
        return cls(width, height, data[match.end():match.end() + width * height * 3])

    def pixel(self, x, y):
        offset = (y * self.width + x) * 3
        return tuple(self.rgb[offset:offset + 3])

    def crop(self, region) -> "Image":
        x, y, w, h = region
        x, y = max(x, 0), max(y, 0)
        w, h = min(w, self.width - x), min(h, self.height - y)
        rows = [self.rgb[(row * self.width + x) * 3:(row * self.width + x + w) * 3]
                for row in range(y, y + h)]
        return Image(w, h, b"".join(rows))

    def mean(self, region=None):
        """Mean (r, g, b) of a region or of the whole image."""
        image = self.crop(region) if region else self
        count = image.width * image.height
        if not count:
            raise DriverError(f"empty region {region}")
        return tuple(sum(image.rgb[channel::3]) / count for channel in range(3))

    def is_color(self, color, region=None, tolerance=12) -> bool:
        return close_color(self.mean(region), color, tolerance)

    def diff_fraction(self, other: "Image", region=None, threshold=16) -> float:
        """Share of pixels (0-1) where a channel differs by more than threshold."""
        a, b = (self.crop(region), other.crop(region)) if region else (self, other)
        if (a.width, a.height) != (b.width, b.height):
            raise DriverError("images differ in size")
        changed = 0
        for i in range(0, len(a.rgb), 3):
            if (abs(a.rgb[i] - b.rgb[i]) > threshold or
                    abs(a.rgb[i + 1] - b.rgb[i + 1]) > threshold or
                    abs(a.rgb[i + 2] - b.rgb[i + 2]) > threshold):
                changed += 1
        return changed / max(a.width * a.height, 1)

    def save_png(self, path):
        raw = bytearray()
        stride = self.width * 3
        for y in range(self.height):
            raw.append(0)
            raw += self.rgb[y * stride:(y + 1) * stride]

        def chunk(kind, body):
            return (struct.pack(">I", len(body)) + kind + body +
                    struct.pack(">I", zlib.crc32(kind + body)))
        Path(path).write_bytes(
            b"\x89PNG\r\n\x1a\n" +
            chunk(b"IHDR", struct.pack(">IIBBBBB", self.width, self.height, 8, 2, 0, 0, 0)) +
            chunk(b"IDAT", zlib.compress(bytes(raw), 6)) + chunk(b"IEND", b""))


def close_color(actual, wanted, tolerance=12) -> bool:
    return all(abs(a - w) <= tolerance for a, w in zip(actual, wanted))


# --- Fixture server ----------------------------------------------------------

class FixtureServer:
    """HTTP on 127.0.0.1 only, on a free port.

    /page/NAME         page titled NAME; ?color=rrggbb sets a solid background.
    Anything else:     b.fixture.routes["/x"] = (status, content_type, body).
    b.fixture.delays["/x"] = seconds holds back an answer.
    b.fixture.slow_routes["/x"] = (first, seconds, rest) sends HTML in two parts.
    Every page records real input: see PAGE_SCRIPT.
    """

    PAGE_SCRIPT = """<script>
    window.events = [];
    for (const type of ['click', 'dblclick', 'contextmenu', 'mousedown', 'mouseup', 'keydown'])
      addEventListener(type, e => events.push({type, x: e.clientX, y: e.clientY, key: e.key,
                                               button: e.button, trusted: e.isTrusted}), true);
    </script>"""

    def __init__(self):
        self.routes = {}
        self.delays = {}  # path: seconds before a route answers
        self.slow_routes = {}  # path: (first html, seconds, rest of the html)
        self.requests = []
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                fixture.requests.append(self.path)
                path = urllib.parse.urlsplit(self.path).path
                time.sleep(fixture.delays.get(path, 0))
                if path in fixture.slow_routes:
                    first, wait, rest = fixture.slow_routes[path]
                    self.send_response(200)
                    self.send_header("Content-Type", "text/html")
                    self.send_header("Cache-Control", "no-store")
                    self.end_headers()
                    self.wfile.write(first.encode())
                    self.wfile.flush()
                    time.sleep(wait)
                    self.wfile.write(rest.encode())
                    self.close_connection = True
                    return
                status, kind, body = fixture.respond(self.path)
                self.send_response(status)
                self.send_header("Content-Type", kind)
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.port = self.server.server_address[1]
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def url(self, path="/"):
        return f"http://127.0.0.1:{self.port}{path}"

    def page(self, name, color=None):
        return self.url(f"/page/{name}" + (f"?color={color}" if color else ""))

    def respond(self, path):
        parsed = urllib.parse.urlsplit(path)
        if parsed.path in self.routes:
            status, kind, body = self.routes[parsed.path]
            return status, kind, body if isinstance(body, bytes) else body.encode()
        if parsed.path.startswith("/page/"):
            name = parsed.path[len("/page/"):]
            color = urllib.parse.parse_qs(parsed.query).get("color", ["ffffff"])[0]
            if not re.fullmatch(r"[0-9a-fA-F]{6}", color):
                return 400, "text/plain", b"bad color"
            html = (f"<!doctype html><title>{name}</title>{self.PAGE_SCRIPT}"
                    f"<style>html,body{{margin:0;height:100%;background:#{color}}}</style>"
                    f"<body><input id=field style='position:fixed;left:40px;top:40px'>"
                    f"<div id=name style='position:fixed;left:40px;top:80px'>{name}</div>")
            return 200, "text/html; charset=utf-8", html.encode()
        return 404, "text/plain", b"not found"

    def close(self):
        self.server.shutdown()
        self.server.server_close()


# --- CDP ---------------------------------------------------------------------

class _WebSocket:
    """The little of RFC 6455 that CDP needs."""

    def __init__(self, url, timeout):
        parts = urllib.parse.urlsplit(url)
        self.sock = socket.create_connection((parts.hostname, parts.port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall((f"GET {parts.path} HTTP/1.1\r\nHost: {parts.netloc}\r\n"
                           "Upgrade: websocket\r\nConnection: Upgrade\r\n"
                           f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n").encode())
        header = b""
        while b"\r\n\r\n" not in header:
            piece = self.sock.recv(1)
            if not piece:
                raise DriverError("CDP socket closed during the handshake")
            header += piece
        if b" 101 " not in header.split(b"\r\n", 1)[0]:
            raise DriverError(f"CDP handshake refused: {header[:200]!r}")

    def _read(self, count):
        data = b""
        while len(data) < count:
            piece = self.sock.recv(count - len(data))
            if not piece:
                raise DriverError("CDP socket closed")
            data += piece
        return data

    def send(self, text, opcode=1):
        body = text if isinstance(text, bytes) else text.encode()
        length = len(body)
        if length < 126:
            head = struct.pack("!BB", 0x80 | opcode, 0x80 | length)
        elif length < 65536:
            head = struct.pack("!BBH", 0x80 | opcode, 0x80 | 126, length)
        else:
            head = struct.pack("!BBQ", 0x80 | opcode, 0x80 | 127, length)
        mask = os.urandom(4)
        self.sock.sendall(head + mask + bytes(c ^ mask[i % 4] for i, c in enumerate(body)))

    def recv(self):
        message = b""
        while True:
            first, second = self._read(2)
            length = second & 0x7f
            if length == 126:
                length = struct.unpack("!H", self._read(2))[0]
            elif length == 127:
                length = struct.unpack("!Q", self._read(8))[0]
            body = self._read(length)
            opcode = first & 0x0f
            if opcode == 8:
                raise DriverError("CDP socket closed by the browser")
            if opcode == 9:
                self.send(body, opcode=10)
            elif opcode in (0, 1, 2):
                message += body
                if first & 0x80:
                    return message.decode()

    def close(self):
        self.sock.close()


def _cdp_call(ws_url, method, params=None, timeout=15):
    ws = _WebSocket(ws_url, timeout)
    try:
        ws.send(json.dumps({"id": 1, "method": method, "params": params or {}}))
        while True:
            reply = json.loads(ws.recv())
            if reply.get("id") == 1:
                if "error" in reply:
                    raise DriverError(f"CDP {method}: {reply['error']}")
                return reply.get("result", {})
    finally:
        ws.close()


# --- Browser -----------------------------------------------------------------

class Browser:
    """One sway + one browser profile. Restarts reuse both."""

    def __init__(self, name, port, root, out=None):
        self.name, self.port = name, port
        self.root = Path(root)
        self.hl = self.root / name
        self.hl.mkdir(parents=True, exist_ok=True)
        self.out = Path(out or os.environ.get("MORI_OUT") or DEFAULT_OUT)
        self.fixture = FixtureServer()
        self.crashes: list[str] = []
        self.violations: list[str] = []
        self._holder = None
        self._pid = None
        self._expect_exit = True
        self._sway_env = None
        self._launches = 0
        self._logs_scanned = set()
        self._started_at = time.time()
        self._seen_pids: set[int] = set()
        self._torn_down = False
        self._origin = None
        self._origin_size = None

    # Paths -------------------------------------------------------------------

    @property
    def profile(self) -> Path:
        return self.hl / "profile"

    @property
    def default_profile(self) -> Path:
        return self.profile / "Default"

    @property
    def sessions_dir(self) -> Path:
        return self.default_profile / "Sessions"

    @property
    def downloads(self) -> Path:
        return self.hl / "downloads"

    @property
    def home(self) -> Path:
        return self.hl / "home"

    @property
    def chrome_log(self) -> Path:
        return self.hl / "chrome.log"

    @property
    def blocked_log(self) -> Path:
        return self.hl / "desktop-blocked.log"

    def preferences(self) -> dict:
        return json.loads((self.default_profile / "Preferences").read_text())

    def local_state(self) -> dict:
        return json.loads((self.profile / "Local State").read_text())

    def session_files(self) -> list[Path]:
        if not self.sessions_dir.exists():
            return []
        return sorted(self.sessions_dir.glob("Session_*"), key=lambda p: p.stat().st_mtime)

    def session_contains(self, text) -> bool:
        needle = text.encode()
        return any(needle in path.read_bytes() for path in self.session_files())

    def current_session_contains(self, text, wide=False) -> bool:
        """In the newest session file only. Older ones keep old state. wide: titles
        are UTF-16 on disk."""
        needle = text.encode("utf-16-le" if wide else "utf-8")
        files = self.session_files()
        return bool(files) and needle in files[-1].read_bytes()

    # Harness -----------------------------------------------------------------

    def _env(self, **extra):
        return safe_env(MORI_HARNESS_RUN=str(self.root), MORI_OUT=str(self.out), **extra)

    def _wl_env(self, **extra):
        self._require_sway()
        return self._env(**self._sway_env, **extra)

    def _script(self, script, *args, env=None, timeout=30, check=True):
        env = env if env is not None else self._wl_env()
        result = subprocess.run([str(HARNESS / script), self.name, *map(str, args)], env=env,
                                stdin=subprocess.DEVNULL, capture_output=True, text=True,
                                timeout=timeout)
        if check and result.returncode != 0:
            raise DriverError(f"{script} {args}: exit {result.returncode}: {result.stderr.strip()}")
        return result

    def _sway_pid(self):
        try:
            return int((self.hl / "sway.pid").read_text())
        except (OSError, ValueError):
            return None

    def _require_sway(self):
        pid = self._sway_pid()
        if self._sway_env is None or not pid or not alive(pid):
            raise DriverError("the test's sway is not running")

    def _start_sway(self):
        if self._sway_env is not None:
            return
        real = real_wayland_display()
        self._script("start.sh", env=self._env())
        values = {}
        for line in (self.hl / "env.sh").read_text().splitlines():
            match = re.fullmatch(r"export (\w+)=(.*)", line.strip())
            if match:
                values[match[1]] = match[2]
        display = values.get("WAYLAND_DISPLAY", "")
        pid = self._sway_pid()
        # An empty name would fall back to a default socket, maybe the real one.
        if not display or display == real or f".{pid}.sock" not in values.get("SWAYSOCK", ""):
            raise GuardViolation(f"sway did not start cleanly: {values}, sway pid {pid}")
        self._sway_env = {"SWAYSOCK": values["SWAYSOCK"], "WAYLAND_DISPLAY": display}

    def swaymsg(self, *args):
        result = subprocess.run(["swaymsg", *args], env=self._wl_env(), capture_output=True,
                                text=True, timeout=15)
        if result.returncode != 0:
            raise DriverError(f"swaymsg {args}: {result.stdout.strip()} {result.stderr.strip()}")
        return result.stdout

    def window_tree(self) -> dict:
        return json.loads(self.swaymsg("-t", "get_tree", "-r"))

    def windows(self) -> list[dict]:
        """Mapped toplevels, from the compositor's point of view."""
        found = []

        def walk(node):
            if node.get("pid") and node.get("type") in ("con", "floating_con"):
                found.append(node)
            for child in node.get("nodes", []) + node.get("floating_nodes", []):
                walk(child)
        walk(self.window_tree())
        return found

    def outputs(self) -> list[dict]:
        return json.loads(self.swaymsg("-t", "get_outputs", "-r"))

    # Lifecycle ---------------------------------------------------------------

    def pid(self):
        return self._pid if self._pid and alive(self._pid) else None

    def pids(self) -> set[int]:
        pids = set(pids_for_profile(self.profile))
        if self.pid():
            pids |= process_tree(self._pid)
        self._seen_pids |= pids
        return pids

    def start(self, *urls, args=(), keyboard=True, timeout=60):
        """Launch through run.sh. Waits until CDP answers and a window has mapped."""
        if not (self.out / "chrome").exists():
            raise DriverError(f"no browser build at {self.out}/chrome")
        self._start_sway()
        if pids_for_profile(self.profile):
            raise DriverError("a browser is still using this profile")
        self._rotate_log()
        if keyboard:
            self.hold_keyboard()
        else:
            self._stop_holder()
        self._expect_exit = False
        self._launches += 1
        self._script("run.sh", f"--remote-debugging-port={self.port}", *args, *urls,
                     env=self._wl_env(NOEXT="1"))
        self._pid = require(lambda: browser_pid_for_profile(self.profile), timeout,
                            "browser process did not appear")
        require(lambda: self._json("version", check_alive=False), timeout, "CDP did not answer")
        self._check_cdp_owner()
        require(self.windows, timeout, "no window mapped")
        self.check()
        return self

    def _check_cdp_owner(self):
        # Another browser on this port would silently answer for this one.
        def owners():
            result = subprocess.run(["ss", "-ltnpH", f"sport = :{self.port}"], env=safe_env(),
                                    capture_output=True, text=True, timeout=15)
            return {int(pid) for pid in _SS_PID.findall(result.stdout)}
        found = require(owners, 15, f"nothing listening on port {self.port}")
        if not found & self.pids():
            raise DriverError(f"port {self.port} belongs to pids {found}, not this browser")

    def restart(self, *urls, how="quit", **kwargs):
        """Stop (how: quit, sigterm, kill9) and launch again on the same profile."""
        {"quit": self.quit_clean, "sigterm": self.sigterm, "kill9": self.kill9}[how]()
        return self.start(*urls, **kwargs)

    def _wait_gone(self, timeout, message):
        gone = poll(lambda: not pids_for_profile(self.profile), timeout)
        if not gone:
            raise DriverError(f"{message}; still running: {pids_for_profile(self.profile)}")

    def _before_stop(self):
        self.check()
        self.pids()
        self._expect_exit = True

    def _after_stop(self):
        self._pid = None
        self._scan_log()

    def quit_clean(self, timeout=30):
        """Close every window through the compositor, as a user does (xdg close).
        Closing the last window quits the browser."""
        pid = self._pid
        self._before_stop()
        self.swaymsg(f"[pid={pid}]", "kill")
        self._wait_gone(timeout, "closing the windows did not quit the browser")
        self._after_stop()

    def close_windows(self, windows, gap=0.0, timeout=30):
        """Close these toplevels in this order, as a compositor does at logout. The
        last browser window closing quits the browser, so name them all."""
        self._before_stop()
        for window in windows:
            self.swaymsg(f"[con_id={window['id']}]", "kill")
            time.sleep(gap)
        self._wait_gone(timeout, "closing the windows did not quit the browser")
        self._after_stop()

    def open_link(self, url, timeout=20):
        """A link from another app: a second launch hands it to the running browser,
        which opens Little Mori by default. Returns the new toplevel."""
        before = {window["id"] for window in self.windows()}
        self._script("run.sh", url, env=self._wl_env(NOEXT="1"))

        def new_window():
            return next((w for w in self.windows() if w["id"] not in before), None)
        return require(new_window, timeout, f"no window opened for {url}")

    def sigterm(self, timeout=30):
        """SIGTERM to the browser process, as at logout."""
        self._before_stop()
        os.kill(self._pid, signal.SIGTERM)
        self._wait_gone(timeout, "browser ignored SIGTERM")
        self._after_stop()

    def kill9(self, timeout=20):
        """SIGKILL to the browser process: a crash, hang kill, or power loss."""
        self._before_stop()
        os.kill(self._pid, signal.SIGKILL)
        self._wait_gone(timeout, "browser children outlived SIGKILL")
        self._after_stop()

    def launch_and_kill(self, seconds, timeout=60):
        """Launch, then SIGKILL that many seconds after the process appears, ready or not."""
        self._start_sway()
        if pids_for_profile(self.profile):
            raise DriverError("a browser is still using this profile")
        self._rotate_log()
        self.hold_keyboard()
        self._expect_exit = False
        self._launches += 1
        self._script("run.sh", f"--remote-debugging-port={self.port}", env=self._wl_env(NOEXT="1"))
        self._pid = require(lambda: browser_pid_for_profile(self.profile), timeout,
                            "browser process did not appear")
        time.sleep(seconds)
        self.kill9()

    def wait_session_saved(self, *texts, timeout=20):
        """Wait until the session file holds each text and has stopped changing.

        Chromium batches session commands for 2.5 s, so a kill right after an
        action tests that delay and nothing else.
        """
        require(lambda: all(self.session_contains(text) for text in texts), timeout,
                f"session file never recorded {texts}")

        def settled():
            files = self.session_files()
            return files and time.time() - files[-1].stat().st_mtime > 3.5
        require(settled, timeout, "session file kept changing")

    def wait_marked_running(self, timeout=30):
        """Wait until the profile on disk says a session is in progress.

        Mori writes exit_type "Crashed" as it starts, or "CrashedInRecovery" for the
        minute after reopening a crashed session, and "Normal" on a clean exit. A kill
        before that write reads as a clean exit at the next launch, so a crash test
        must wait for it or it proves nothing.
        """
        def marked():
            try:
                kind = self.preferences().get("profile", {}).get("exit_type")
            except (OSError, ValueError):
                return False
            return kind in ("Crashed", "CrashedInRecovery")
        require(marked, timeout, 'Preferences never recorded a running exit_type')

    def teardown(self):
        """Always safe to call; leaves nothing of this test running."""
        if self._torn_down:
            return
        self._torn_down = True
        try:
            self.check_guards()
        except Exception:  # A violation is in self.violations and the guard log.
            pass
        self.check_alive()
        self.pids()
        self._expect_exit = True
        for pid in pids_for_profile(self.profile):
            _signal(pid, signal.SIGTERM)
        if not poll(lambda: not pids_for_profile(self.profile), 8):
            for pid in pids_for_profile(self.profile):
                _signal(pid, signal.SIGKILL)
            poll(lambda: not pids_for_profile(self.profile), 5)
        self._after_stop()
        self._stop_holder()
        sway = self._sway_pid()
        if sway and alive(sway) and "sway" in _cmdline(sway):
            self._script("stop.sh", env=self._env(), check=False)
            poll(lambda: not alive(sway), 5)
        self._scan_coredumps()
        self.fixture.close()

    # Health ------------------------------------------------------------------

    def _rotate_log(self):
        # run.sh truncates chrome.log on each launch: keep what a test may read.
        if self.chrome_log.exists() and self._launches:
            self._scan_log()
            self.chrome_log.rename(self.hl / f"chrome.{self._launches}.log")

    def _scan_log(self):
        if not self.chrome_log.exists() or self._launches in self._logs_scanned:
            return
        if not self._expect_exit:
            return
        self._logs_scanned.add(self._launches)
        for line in self.chrome_log.read_text(errors="replace").splitlines():
            if CRASH_LOG_RE.search(line):
                self.crashes.append(f"chrome.log (launch {self._launches}): {line[:300]}")

    def _scan_coredumps(self):
        if not self._seen_pids:
            return
        time.sleep(1)    # systemd-coredump records asynchronously.
        since = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(self._started_at - 1))
        result = subprocess.run(["coredumpctl", "--no-pager", "--json=short", "--since", since, "list"],
                                env=safe_env(), capture_output=True, text=True, timeout=30)
        if result.returncode != 0 or not result.stdout.strip():
            return
        try:
            dumps = json.loads(result.stdout)
        except ValueError:
            return
        token = f"--user-data-dir={self.profile}"
        for dump in dumps:
            if dump.get("exe") != str(self.out / "chrome"):
                continue
            mine = dump.get("pid") in self._seen_pids
            if not mine:
                info = subprocess.run(["coredumpctl", "--no-pager", "info", str(dump.get("pid"))],
                                      env=safe_env(), capture_output=True, text=True, timeout=30)
                mine = token in info.stdout
            if mine:
                self.crashes.append(
                    f"coredump: pid {dump.get('pid')} signal {dump.get('sig')} ({dump.get('exe')})")

    def check_alive(self):
        """Record an exit nobody asked for."""
        if not self._expect_exit and self._pid and not alive(self._pid):
            self.crashes.append(f"browser pid {self._pid} exited unexpectedly")
            self._expect_exit = True
            self._scan_log()

    def check_guards(self, ss_output=None):
        """Raise GuardViolation if this browser reached the real desktop."""
        pids = self.pids()
        if not pids:
            return
        # Only this sway's Wayland socket. X11 is never allowed: the browser runs
        # with --ozone-platform=wayland, and the real Xwayland is on the same path.
        allowed = set()
        if self._sway_env:
            allowed.add(f"{os.environ.get('XDG_RUNTIME_DIR', '/run/user/%d' % os.getuid())}"
                        f"/{self._sway_env['WAYLAND_DISPLAY']}")
        bad = forbidden_connections(socket_connections(ss_output or read_ss(), pids), allowed)
        if bad:
            message = f"browser {self.name} reached the real desktop: {bad}"
            self.violations.append(message)
            with open(self.hl / GUARD_LOG, "a") as log:
                log.write(message + "\n")
            raise GuardViolation(message)

    def check(self):
        """Fail fast on a crash or a guard violation."""
        self.check_alive()
        if self.crashes:
            raise DriverError(f"browser crashed: {self.crashes}")
        self.check_guards()

    # CDP ---------------------------------------------------------------------

    def _json(self, path, method="GET", check_alive=True):
        if check_alive:
            self.check_alive()
        request = urllib.request.Request(f"http://127.0.0.1:{self.port}/json/{path}", method=method)
        try:
            # The proxy variables must not apply to this.
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            with opener.open(request, timeout=5) as response:
                body = response.read().decode()
        except (urllib.error.URLError, OSError):
            return None
        try:
            return json.loads(body)
        except ValueError:
            return body

    def tabs(self) -> list[dict]:
        """Web contents the browser lists as pages. Order is not tab-strip order."""
        return [t for t in (self._json("list") or []) if t.get("type") == "page"]

    def tab_urls(self) -> list[str]:
        return sorted(t["url"] for t in self.tabs())

    def wait_tab_urls(self, urls, timeout=20):
        """Require exactly these tab URLs (any order); returns whether they came."""
        wanted = sorted(urls)
        return poll(lambda: self.tab_urls() == wanted, timeout)

    def find_tab(self, match) -> dict:
        """The tab whose URL or title contains match; DriverError if none."""
        for tab in self.tabs():
            if match in tab["url"] or match in tab["title"]:
                return tab
        raise DriverError(f"no tab matching {match!r}: {self.tab_urls()}")

    def cdp(self, tab, method, params=None):
        tab = self.find_tab(tab) if isinstance(tab, str) else tab
        return _cdp_call(tab["webSocketDebuggerUrl"], method, params)

    def eval(self, expression, tab=None):
        """Evaluate JS in a tab (URL/title substring, tab dict, or the active one)."""
        tab = self.active_tab() if tab is None else tab
        result = self.cdp(tab, "Runtime.evaluate", {
            "expression": expression, "returnByValue": True, "awaitPromise": True})
        if "exceptionDetails" in result:
            raise DriverError(f"page exception: {result['exceptionDetails']}")
        return result.get("result", {}).get("value")

    def active_tab(self) -> dict:
        """The tab the user is looking at. Background tabs report 'hidden'."""
        def visible():
            for tab in self.tabs():
                try:
                    if self.eval("document.visibilityState", tab) == "visible":
                        return tab
                except DriverError:
                    continue
            return None
        return require(visible, 10, "no visible tab")

    def activate_tab(self, match):
        self._json(f"activate/{self.find_tab(match)['id']}")

    def page_origin(self, tab=None):
        """Screen position of the page's top-left corner, measured with the real
        pointer because Mori insets the page. Moves the pointer once per viewport
        size. An overlay such as the command bar must not cover the page's centre.
        """
        width, height = self.eval("[innerWidth, innerHeight]", tab)
        if self._origin_size != (width, height):
            self.eval("""new Promise(done => { window.__moriOrigin = null;
                addEventListener('mousemove', e => { window.__moriOrigin = [e.clientX, e.clientY]; });
                done(); })""", tab)
            window = self.windows()[0]["rect"]
            # Anywhere inside the page does: the sidebar is at most a third wide.
            at = (window["x"] + window["width"] * 2 // 3, window["y"] + window["height"] // 2)
            self.move(*at, hold_ms=100)
            client = require(lambda: self.eval("window.__moriOrigin", tab), 5,
                             "the page saw no pointer motion")
            self._origin = (at[0] - client[0], at[1] - client[1])
            self._origin_size = (width, height)
        return self._origin

    def page_rect(self, tab=None):
        """(x, y, width, height) on screen of the page."""
        x, y = self.page_origin(tab)
        width, height = self.eval("[innerWidth, innerHeight]", tab)
        return x, y, width, height

    # Screenshots -------------------------------------------------------------

    def screenshot(self, path=None) -> Path:
        """A PNG, through shot.sh. Default: $HL/shot.png."""
        path = Path(path or self.hl / "shot.png")
        self._script("shot.sh", path)
        return path

    def grab(self) -> Image:
        """The screen, as pixels."""
        result = subprocess.run(["grim", "-t", "ppm", "-o", "HEADLESS-1", "-"], env=self._wl_env(),
                                capture_output=True, timeout=15)
        if result.returncode != 0:
            raise DriverError(f"grim: {result.stderr.decode(errors='replace').strip()}")
        return Image.from_ppm(result.stdout)

    def wait_color(self, region, color, timeout=10, tolerance=12) -> bool:
        """Frames present late, so give the screen time to match."""
        return bool(poll(lambda: self.grab().is_color(color, region, tolerance), timeout, 0.3))

    # Input -------------------------------------------------------------------

    def _ptr(self, *args, hold_ms=None):
        extra = {"VPTR_HOLD_MS": str(hold_ms)} if hold_ms is not None else {}
        self._script("ptr.sh", *args, env=self._wl_env(**extra), timeout=30)

    def move(self, x, y, hold_ms=None):
        self._ptr("move", x, y, hold_ms=hold_ms)

    def click(self, x, y, hold_ms=None):
        self._ptr("click", x, y, hold_ms=hold_ms)

    def right_click(self, x, y, hold_ms=None):
        self._ptr("click", x, y, "right", hold_ms=hold_ms)

    def double_click(self, x, y, hold_ms=None):
        self._ptr("dblclick", x, y, hold_ms=hold_ms)

    def scroll(self, x, y, dy, hold_ms=None):
        self._ptr("scroll", x, y, dy, hold_ms=hold_ms)

    def drag(self, x1, y1, x2, y2, hold_ms=None):
        self._ptr("drag", x1, y1, x2, y2, hold_ms=hold_ms)

    def hold_keyboard(self):
        """Keep a live virtual keyboard on the seat, and make it the current one.

        Each wtype run adds a keyboard and removes it on exit. A seat with no
        keyboard drops focus, activation and hover state, which gives false results
        in those tests. And sway clears the seat's current keyboard when the current
        one goes away, even if another remains; only a key press sets it again. A
        browser that starts in that state gets focus but no keymap, then crashes in
        XkbKeyboardLayoutEngine::UpdateModifiers. That is the harness, not Mori.

        So a new holder replaces the old one before each launch and after each
        keys()/type_text(). Its one Shift press can show up in page key events.
        """
        previous = self._holder
        self._holder = subprocess.Popen(["wtype", "-k", "Shift_L", "-s", "86400000"], env=self._wl_env(),
                                        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                        stderr=subprocess.DEVNULL)
        time.sleep(0.12)
        if self._holder.poll() is not None:
            raise DriverError("the keyboard holder exited at once")
        self._stop_process(previous)

    @staticmethod
    def _stop_process(process):
        if process and process.poll() is None:
            process.terminate()
            try:
                process.wait(3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(3)

    def _stop_holder(self):
        self._stop_process(self._holder)
        self._holder = None

    def _after_keys(self):
        if self._holder:
            self.hold_keyboard()

    def keys(self, *combos):
        """Press key combos: keys("ctrl+t"), keys("ctrl+shift+q", "Escape").
        Names are xkb keysyms (Return, Escape, Tab, Left, F5, a, 1).

        wtype invents its own keymap, so a combo whose xkb result is a NUL control
        character (Ctrl+2, Ctrl+Space) does not map back to its key in Chromium. That
        is wtype, not Mori. Use alt+2 for the tab shortcuts.
        """
        modifiers = {"ctrl": "ctrl", "shift": "shift", "alt": "alt", "super": "logo", "logo": "logo"}
        for combo in combos:
            *held, key = combo.split("+")
            args = []
            for name in held:
                args += ["-M", modifiers[name.lower()]]
            args += ["-k", key]
            for name in reversed(held):
                args += ["-m", modifiers[name.lower()]]
            self._script("key.sh", *args)
        self._after_keys()

    def type_text(self, text, delay_ms=20):
        """Type text. The delay keeps a slow UI from seeing keys out of order."""
        self._script("key.sh", "-d", delay_ms, text, timeout=60)
        self._after_keys()


def _signal(pid, signum):
    try:
        os.kill(pid, signum)
    except OSError:
        pass
