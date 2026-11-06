#!/usr/bin/env python3
"""Run Mori's end-to-end tests in parallel. See README.md.

    linux/tests/e2e/run.py                 everything
    linux/tests/e2e/run.py -k smoke        by tag or by part of the test's id
    linux/tests/e2e/run.py --list

Exit code: 0 when every selected test passed, was skipped, or failed as marked by
@xfail; 1 otherwise; 2 when the suite could not run. A test also fails if its browser
reached the real desktop, wrote unexpected lines to desktop-blocked.log, crashed, or
was left running. Those checks live here, so a test cannot skip them.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import importlib.util
import inspect
import json
import os
import random
import re
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
import traceback
from pathlib import Path

E2E_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(E2E_DIR))

import driver  # noqa: E402
from driver import Browser, Skip, safe_env  # noqa: E402

DEFAULT_TIMEOUT = 120
PORT_RANGE = (21000, 29000)
KEEP_RUNS = 5
RESULT_FILE = "e2e-result.json"
# Lines any test may cause in desktop-blocked.log. Use @allow_blocked for one test.
GLOBAL_ALLOWED_BLOCKED: tuple[str, ...] = ()
OK_STATUSES = {"pass", "xfail", "skip"}


# --- Discovery ---------------------------------------------------------------

def load_module(path: Path):
    spec = importlib.util.spec_from_file_location(f"e2e_{path.stem}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def discover(tests_dir: Path) -> list[dict]:
    tests = []
    for path in sorted(tests_dir.glob("test_*.py")):
        module = load_module(path)
        for name, fn in vars(module).items():
            if name.startswith("test_") and inspect.isfunction(fn) and fn.__module__ == module.__name__:
                tests.append({
                    "id": f"{path.name}::{name}",
                    "path": str(path),
                    "func": name,
                    "tags": list(getattr(fn, "e2e_tags", ())),
                    "xfail": getattr(fn, "e2e_xfail", None),
                    "timeout": getattr(fn, "e2e_timeout", None),
                    "line": inspect.getsourcelines(fn)[1],
                })
    return tests


def select(tests, filters):
    if not filters:
        return tests
    return [t for t in tests if any(f == tag for f in filters for tag in t["tags"])
            or any(f in t["id"] for f in filters)]


# --- One test, in its own process --------------------------------------------

def run_child(args) -> int:
    """Run one test and write its result. Teardown always runs."""
    path, func = args.child.rsplit("::", 1)
    hl = Path(args.root) / args.name
    hl.mkdir(parents=True, exist_ok=True)
    result = {"status": "error", "detail": "the test did not finish", "crashes": [], "violations": [],
              "allowed_blocked": []}
    result_path = hl / RESULT_FILE

    def write():
        temp = result_path.with_suffix(".tmp")
        temp.write_text(json.dumps(result))
        temp.replace(result_path)

    def on_term(signum, frame):
        raise SystemExit(f"stopped by signal {signum}: the test timed out")
    signal.signal(signal.SIGTERM, on_term)

    browser = None
    try:
        module = load_module(Path(path))
        fn = getattr(module, func)
        result["allowed_blocked"] = list(getattr(fn, "e2e_allow_blocked", ()))
        write()
        wants_browser = "b" in inspect.signature(fn).parameters
        marked = getattr(fn, "e2e_xfail", None)
        try:
            if wants_browser:
                browser = Browser(args.name, args.port, args.root)
                fn(browser)
                browser.check()
            else:
                fn()
            result.update(status="xpass" if marked else "pass",
                          detail=f"passes but is marked xfail: {marked}" if marked else "")
        except Skip as skip:
            result.update(status="skip", detail=str(skip))
        except AssertionError as error:
            result.update(status="xfail" if marked else "fail",
                          detail=(f"{marked} | " if marked else "") + _describe(error))
            _failure_shot(browser, hl)
        except BaseException as error:  # noqa: B902  SystemExit from the timeout too.
            result.update(status="error", detail=_describe(error))
            _failure_shot(browser, hl)
    except BaseException as error:  # noqa: B902
        result.update(status="error", detail=_describe(error))
    finally:
        if browser:
            try:
                browser.teardown()
            except BaseException as error:  # noqa: B902
                result.update(status="error", detail=f"teardown failed: {_describe(error)}")
            result["crashes"] = browser.crashes
            result["violations"] = browser.violations
        write()
    return 0


def _describe(error) -> str:
    return "".join(traceback.format_exception(error)).strip()


def _failure_shot(browser, hl):
    try:
        if browser:
            browser.grab().save_png(hl / "failure.png")
    except Exception:  # A screenshot is a bonus: sway may be what failed.
        pass


# --- Guards ----------------------------------------------------------------

class GuardSampler(threading.Thread):
    """Watches every test browser from outside the test, every couple of seconds."""

    def __init__(self, root: Path, patterns=driver.FORBIDDEN_SOCKETS):
        super().__init__(daemon=True)
        self.patterns = patterns
        self.root = root
        self.profile = re.compile(re.escape(f"--user-data-dir={root}/") + r"([^/]+)/profile(?: |$)")
        self.halt = threading.Event()
        self.violations: dict[str, set] = {}
        self.samples = 0
        self.error = None

    def run(self):
        while not self.halt.wait(2):
            try:
                self.sample()
            except Exception as error:
                self.error = f"guard sampler: {error}"
                return

    def _test_processes(self):
        """{instance name: pids}, by the profile path under this run's root."""
        found: dict[str, set] = {}
        for entry in os.listdir("/proc"):
            if entry.isdigit():
                match = self.profile.search(driver._cmdline(entry))
                if match:
                    found.setdefault(match[1], set()).update(driver.process_tree(int(entry)))
        return found

    def _own_socket(self, name):
        try:
            text = (self.root / name / "env.sh").read_text()
            display = re.search(r"WAYLAND_DISPLAY=(\S+)", text)[1]
        except (OSError, TypeError):
            return set()
        return {f"{os.environ.get('XDG_RUNTIME_DIR', '/run/user/%d' % os.getuid())}/{display}"}

    def sample(self):
        instances = self._test_processes()
        if not instances:
            return
        output = driver.read_ss()
        self.samples += 1
        for name, pids in instances.items():
            bad = driver.forbidden_connections(driver.socket_connections(output, pids),
                                               self._own_socket(name), self.patterns)
            if bad:
                self.violations.setdefault(name, set()).update(bad)

    def stop(self):
        self.halt.set()
        self.join(10)
        try:
            self.sample()
        except Exception as error:
            self.error = f"guard sampler: {error}"


def force_cleanup(hl: Path) -> list[str]:
    """Stop whatever a dead test left behind. Matches by exact profile and pid."""
    left = []
    profile = hl / "profile"
    pids = driver.pids_for_profile(profile)
    if pids:
        left.append(f"left a browser running: {pids}")
        for pid in pids:
            driver._signal(pid, signal.SIGKILL)
    try:
        sway = int((hl / "sway.pid").read_text())
    except (OSError, ValueError):
        sway = None
    if sway and driver.alive(sway) and f"sway -c {driver.HARNESS}/sway.conf" in driver._cmdline(sway):
        left.append(f"left sway running: {sway}")
        driver._signal(sway, signal.SIGTERM)
    return left


# --- Running -----------------------------------------------------------------

class Ports:
    def __init__(self):
        self.lock = threading.Lock()
        self.used = set()

    def take(self) -> int:
        with self.lock:
            for _ in range(2000):
                port = random.randrange(*PORT_RANGE)
                if port in self.used:
                    continue
                with socket.socket() as probe:
                    probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                    try:
                        probe.bind(("127.0.0.1", port))
                    except OSError:
                        continue
                self.used.add(port)
                return port
        raise RuntimeError("no free port")


def run_test(test, index, args, root: Path, ports: Ports) -> dict:
    name = f"t{index:03d}-{test['func'].removeprefix('test_')[:40]}"
    hl = root / name
    hl.mkdir(parents=True, exist_ok=True)
    limit = test["timeout"] or args.timeout
    started = time.monotonic()
    command = [sys.executable, str(E2E_DIR / "run.py"), "--child", f"{test['path']}::{test['func']}",
               "--name", name, "--port", str(ports.take()), "--root", str(root)]
    timed_out = False
    with open(hl / "test.log", "w") as log:
        env = safe_env(MORI_E2E_REAL_WAYLAND=driver.real_wayland_display() or "")
        process = subprocess.Popen(command, env=env, stdin=subprocess.DEVNULL, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            process.wait(limit)
        except subprocess.TimeoutExpired:
            timed_out = True
            _signal_group(process.pid, signal.SIGTERM)
            try:
                process.wait(30)    # Long enough for its teardown.
            except subprocess.TimeoutExpired:
                _signal_group(process.pid, signal.SIGKILL)
                process.wait(10)
    # Whatever survives in its group, such as a keyboard holder.
    _signal_group(process.pid, signal.SIGKILL)

    outcome = {"id": test["id"], "name": name, "dir": str(hl), "seconds": time.monotonic() - started,
               "status": "error", "problems": []}
    allowed = list(GLOBAL_ALLOWED_BLOCKED)
    try:
        result = json.loads((hl / RESULT_FILE).read_text())
        outcome["status"], detail = result["status"], result["detail"]
        allowed += result["allowed_blocked"]
        outcome["problems"] += [f"crash: {c}" for c in result["crashes"]]
    except (OSError, ValueError, KeyError) as error:
        detail = f"no result from the test process (exit {process.returncode}): {error}"
    outcome["detail"] = detail
    if process.returncode != 0:
        outcome["problems"].append(f"test process exited with {process.returncode}")
    if timed_out:
        outcome["problems"].append(f"timed out after {limit}s")
    outcome["problems"] += force_cleanup(hl)
    outcome["problems"] += [f"unexpected desktop-blocked.log line: {line}"
                            for line in driver.blocked_log_problems(hl / "desktop-blocked.log", allowed)]
    guard_log = hl / driver.GUARD_LOG
    if guard_log.exists():
        outcome["problems"] += [f"guard: {line}" for line in guard_log.read_text().splitlines()]
    if outcome["problems"]:
        outcome["status"] = "fail"
    return outcome


def _signal_group(pgid, signum):
    try:
        os.killpg(pgid, signum)
    except OSError:
        pass


def preflight() -> str | None:
    missing = [tool for tool in ("sway", "swaymsg", "wtype", "grim", "ss", "coredumpctl")
               if not shutil.which(tool)]
    if missing:
        return f"missing tools: {', '.join(missing)}"
    out = Path(os.environ.get("MORI_OUT") or driver.DEFAULT_OUT)
    if not (out / "chrome").exists():
        return f"no browser build at {out}/chrome (run linux/build.sh, or set MORI_OUT)"
    vptr = driver.HARNESS / "vptr"
    if not (vptr / "target/release/vptr").exists():
        # ptr.sh would otherwise build it from several tests at once.
        built = subprocess.run(["cargo", "build", "--release", "--offline", "-q"], cwd=vptr)
        if built.returncode != 0:
            return "could not build linux/harness/vptr"
    return None


def make_root(args) -> Path:
    if args.root:
        root = Path(args.root).resolve()
        root.mkdir(parents=True, exist_ok=True)
        return root
    base = os.environ.get("MORI_HARNESS_RUN")
    runs = (Path(base) if base else driver.REPO / "ungoogled-chromium-linux") / "e2e-runs"
    runs.mkdir(parents=True, exist_ok=True)
    old = sorted(p for p in runs.iterdir() if p.is_dir())
    for stale in old[:max(0, len(old) - KEEP_RUNS + 1)]:
        subprocess.run(["gio", "trash", str(stale)], env=safe_env(), check=False,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    root = runs / time.strftime("%Y%m%d-%H%M%S")
    while root.exists():
        time.sleep(1)
        root = runs / time.strftime("%Y%m%d-%H%M%S")
    root.mkdir()
    return root


def default_jobs() -> int:
    return max(1, min(4, (os.cpu_count() or 4) // 4))


def main() -> int:
    parser = argparse.ArgumentParser(description="Mori end-to-end tests")
    parser.add_argument("-k", action="append", default=[], metavar="FILTER",
                        help="a tag, or part of a test id; repeat for OR")
    parser.add_argument("--list", action="store_true", help="list the tests and exit")
    parser.add_argument("-j", "--jobs", type=int, default=default_jobs())
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT, help="seconds per test")
    parser.add_argument("--root", help="run directory (default: a new one under e2e-runs)")
    parser.add_argument("--tests-dir", default=str(E2E_DIR), help=argparse.SUPPRESS)
    parser.add_argument("--json", help="also write the results here")
    parser.add_argument("--child", help=argparse.SUPPRESS)
    parser.add_argument("--name", help=argparse.SUPPRESS)
    parser.add_argument("--port", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.child:
        return run_child(args)

    try:
        tests = select(discover(Path(args.tests_dir)), args.k)
    except Exception:
        traceback.print_exc()
        return 2
    if args.list:
        for test in tests:
            notes = (f" [{','.join(test['tags'])}]" if test["tags"] else "")
            notes += f" (xfail: {test['xfail']})" if test["xfail"] else ""
            print(test["id"] + notes)
        return 0
    if not tests:
        print("no tests selected", file=sys.stderr)
        return 2
    problem = preflight()
    if problem:
        print(f"cannot run: {problem}", file=sys.stderr)
        return 2

    root = make_root(args)
    ports = Ports()
    sampler = GuardSampler(root)
    sampler.start()
    started = time.monotonic()
    print(f"{len(tests)} tests, {args.jobs} at a time, in {root}", flush=True)
    outcomes = []
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        futures = [pool.submit(run_test, test, index, args, root, ports)
                   for index, test in enumerate(tests)]
        for future in concurrent.futures.as_completed(futures):
            outcome = future.result()
            outcomes.append(outcome)
            print(f"{outcome['status'].upper():6} {outcome['id']} ({outcome['seconds']:.1f}s)", flush=True)
    sampler.stop()

    for outcome in outcomes:
        problems = sampler.violations.get(outcome["name"])
        if problems:
            outcome["status"] = "fail"
            outcome["problems"].append(f"guard: reached the real desktop: {sorted(problems)}")
    failed = [o for o in outcomes if o["status"] not in OK_STATUSES]
    for outcome in sorted(failed, key=lambda o: o["id"]):
        print(f"\n--- {outcome['status'].upper()} {outcome['id']}\n    in {outcome['dir']}")
        for problem in outcome["problems"]:
            print(f"    {problem}")
        if outcome["detail"]:
            print("    " + outcome["detail"].replace("\n", "\n    "))
    # A guard that stopped watching must not read as a clean run.
    fatal = [sampler.error] if sampler.error else []
    for message in fatal:
        print(f"\n{message}")

    counts = {}
    for outcome in outcomes:
        counts[outcome["status"]] = counts.get(outcome["status"], 0) + 1
    summary = ", ".join(f"{count} {status}" for status, count in sorted(counts.items()))
    print(f"\n{summary} in {time.monotonic() - started:.1f}s; "
          f"{sampler.samples} guard samples; results in {root}")
    if args.json:
        Path(args.json).write_text(json.dumps(
            {"outcomes": sorted(outcomes, key=lambda o: o["id"]), "samples": sampler.samples}, indent=1))
    return 1 if failed or fatal else 0


if __name__ == "__main__":
    sys.exit(main())
