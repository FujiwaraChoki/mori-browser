"""The test browser must not be able to reach the real desktop, and the checks that
say so must themselves work."""
import json
import os
import re
import socket
import subprocess
import sys
from pathlib import Path

import driver
import run
from driver import E2E_DIR, Image, expect, require, tags


@tags("smoke", "guard")
def test_browser_has_no_real_desktop_connection(b):
    url = b.fixture.page("guard")
    b.start(url)
    require(lambda: b.wait_tab_urls([url]), 20, "tab did not appear")
    b.keys("ctrl+t")
    b.keys("Escape")

    connections = driver.socket_connections(driver.read_ss(), b.pids())
    runtime = os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")
    paths = {path for _, path in connections}

    # The check must see something, or finding nothing proves nothing.
    own = f"{runtime}/{b._sway_env['WAYLAND_DISPLAY']}"
    assert own in paths, f"ss did not show the browser's own Wayland socket: {paths}"

    for path in paths:
        assert not re.fullmatch(r"/run/user/\d+/bus", path), f"reached the session bus: {path}"
    assert driver.forbidden_connections(connections, {own}) == []
    b.check_guards()

    assert b._sway_env["WAYLAND_DISPLAY"] != driver.real_wayland_display(), "using the real display"
    launch = (b.hl / "launch.sh").read_text()
    for guard in ("DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent", "GTK_USE_PORTAL=0",
                  "unset HYPRLAND_INSTANCE_SIGNATURE", f"HOME={b.home}", str(b.hl / "guard-bin")):
        assert guard in launch, f"run.sh no longer sets {guard}"
    assert f"--user-data-dir={b.profile}" in launch
    # When the browser does try the bus, it gets the dead address.
    expect(lambda: "/nonexistent/mori-harness-no-bus" in b.chrome_log.read_text(errors="replace"), 15,
           "chrome.log does not show the browser using the dead bus address")


def test_ss_detector_finds_a_bus_connection():
    ss = "\n".join([
        "u_str LISTEN 0 4096 /run/user/1000/bus 100 * 0 users:((\"dbus-broker\",pid=5,fd=3))",
        "u_str ESTAB  0 0    /run/user/1000/bus 101 * 201 users:((\"dbus-broker\",pid=5,fd=9))",
        "u_str ESTAB  0 0    * 201 * 101 users:((\"chrome\",pid=77,fd=12))",
        "u_str ESTAB  0 0    * 301 * 401 users:((\"other\",pid=88,fd=12))",
        "u_str ESTAB  0 0    /run/user/1000/bus 401 * 301 users:((\"dbus-broker\",pid=5,fd=10))",
        "u_str ESTAB  0 0    /run/user/1000/wayland-3 501 * 601 users:((\"sway\",pid=6,fd=10))",
        "u_str ESTAB  0 0    * 601 * 501 users:((\"chrome\",pid=77,fd=13))",
        "u_str ESTAB  0 0    * 602 /run/user/1000/wayland-1 502 users:((\"chrome\",pid=78,fd=13))",
    ])
    found = driver.socket_connections(ss, {77})
    assert (77, "/run/user/1000/bus") in found, found
    assert driver.forbidden_connections(found, {"/run/user/1000/wayland-3"}) == [(77, "/run/user/1000/bus")]
    # Somebody else's connection is not the browser's.
    assert driver.forbidden_connections(driver.socket_connections(ss, {99})) == []
    # Another compositor's socket: a peer path right on the client's row.
    assert driver.forbidden_connections(driver.socket_connections(ss, {78}),
                                        {"/run/user/1000/wayland-3"}) == [(78, "/run/user/1000/wayland-1")]


def test_ss_detector_works_on_a_live_socket():
    """Real ss output, on a scratch socket: not the real bus, which stays untouched."""
    directory = Path(os.environ.get("TMPDIR", "/tmp")) / f"mori-e2e-detector-{os.getpid()}"
    directory.mkdir()
    path = directory / "bus"
    server = socket.socket(socket.AF_UNIX)
    server.bind(str(path))
    server.listen(1)
    client = subprocess.Popen(
        [sys.executable, "-c",
         "import socket,sys,time\n"
         "s=socket.socket(socket.AF_UNIX);s.connect(sys.argv[1]);print('up',flush=True);time.sleep(30)",
         str(path)], stdout=subprocess.PIPE, text=True)
    try:
        client.stdout.readline()
        accepted, _ = server.accept()
        patterns = (re.compile(re.escape(str(path)) + "$"),)
        def seen():
            return driver.forbidden_connections(
                driver.socket_connections(driver.read_ss(), {client.pid}), patterns=patterns)
        assert expect(seen, 5, "ss did not show the connection") == [(client.pid, str(path))]
        accepted.close()
    finally:
        client.kill()
        client.wait()
        server.close()
        path.unlink()
        directory.rmdir()


def test_guard_sampler_catches_a_browser_on_a_forbidden_socket():
    """The runner's sampler, from process lookup to verdict, on a stand-in browser."""
    directory = Path(os.environ.get("TMPDIR", "/tmp")) / f"mori-e2e-sampler-{os.getpid()}"
    directory.mkdir()
    path = directory / "bus"
    server = socket.socket(socket.AF_UNIX)
    server.bind(str(path))
    server.listen(1)
    stand_in = subprocess.Popen(
        [sys.executable, "-c",
         "import socket,sys,time\n"
         "s=socket.socket(socket.AF_UNIX);s.connect(sys.argv[1]);print('up',flush=True);time.sleep(30)",
         str(path), f"--user-data-dir={directory}/fake/profile"], stdout=subprocess.PIPE, text=True)
    try:
        stand_in.stdout.readline()
        accepted, _ = server.accept()
        sampler = run.GuardSampler(directory, (re.compile(re.escape(str(path)) + "$"),))

        def caught():
            sampler.sample()
            return sampler.violations
        assert expect(caught, 5, "the sampler missed it") == {"fake": {(stand_in.pid, str(path))}}
        accepted.close()
    finally:
        stand_in.kill()
        stand_in.wait()
        server.close()
        path.unlink()
        directory.rmdir()


def test_blocked_log_check():
    log = Path(os.environ.get("TMPDIR", "/tmp")) / f"mori-e2e-blocked-{os.getpid()}.log"
    assert driver.blocked_log_problems(log) == []
    log.write_text("12:00:00 xdg-open https://example.com\n12:00:01 xdg-settings check x\n")
    try:
        assert len(driver.blocked_log_problems(log)) == 2
        assert driver.blocked_log_problems(log, [r"xdg-settings"]) == ["12:00:00 xdg-open https://example.com"]
    finally:
        log.unlink()


def test_safe_env_drops_the_real_desktop():
    keep = dict(os.environ)
    try:
        os.environ.update(WAYLAND_DISPLAY="wayland-1", DISPLAY=":0", HYPRLAND_INSTANCE_SIGNATURE="x",
                          DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/1/bus", SWAYSOCK="/x")
        env = driver.safe_env()
        for name in ("WAYLAND_DISPLAY", "DISPLAY", "HYPRLAND_INSTANCE_SIGNATURE",
                     "DBUS_SESSION_BUS_ADDRESS", "SWAYSOCK"):
            assert name not in env, name
        assert "PATH" in env
    finally:
        os.environ.clear()
        os.environ.update(keep)


@tags("guard")
def test_image_helpers(b):
    red, blue = b"\xff\x00\x00", b"\x00\x00\xff"
    image = Image.from_ppm(b"P6\n4 2\n255\n" + red * 2 + blue * 2 + red * 2 + blue * 2)
    assert (image.width, image.height) == (4, 2)
    assert image.pixel(3, 1) == (0, 0, 255)
    assert image.mean((0, 0, 2, 2)) == (255, 0, 0)
    assert image.mean() == (127.5, 0, 127.5)
    assert image.is_color((0, 0, 255), (2, 0, 2, 2))
    assert image.crop((1, 0, 2, 1)).rgb == red + blue
    other = Image(4, 2, red * 8)
    assert image.diff_fraction(other) == 0.5
    assert image.diff_fraction(other, (0, 0, 2, 2)) == 0
    png = b.hl / "image.png"
    image.save_png(png)
    assert png.read_bytes().startswith(b"\x89PNG")


@tags("guard")
def test_runner_fails_when_it_must(b):
    """Run selfcheck/ through run.py: each case must get the verdict below."""
    report = b.hl / "selfcheck.json"
    run = subprocess.run(
        [sys.executable, str(E2E_DIR / "run.py"), "--tests-dir", str(E2E_DIR / "selfcheck"),
         "--root", str(b.hl / "selfcheck"), "--json", str(report), "-j", "8", "--timeout", "60"],
        capture_output=True, text=True, env=driver.safe_env(), timeout=110)
    assert run.returncode == 1, f"run.py exit {run.returncode}:\n{run.stdout}\n{run.stderr}"
    outcomes = {o["id"].split("::")[1]: o for o in json.loads(report.read_text())["outcomes"]}
    wanted = {
        "test_passes": "pass",
        "test_assertion": "fail",
        "test_driver_error": "error",
        "test_skip": "skip",
        "test_marked_and_fails": "xfail",
        "test_marked_and_passes": "xpass",
        "test_marked_but_setup_broke": "error",
        "test_marked_but_crashed": "fail",
        "test_unexpected_blocked_line": "fail",
        "test_allowed_blocked_line": "pass",
        "test_guard_violation_logged": "fail",
        "test_times_out": "fail",
        "test_dies_leaving_browser_running": "fail",
    }
    actual = {name: outcome["status"] for name, outcome in outcomes.items()}
    assert actual == wanted, f"got {actual}"
    assert any("timed out" in p for p in outcomes["test_times_out"]["problems"])
    assert any("left a browser" in p for p in outcomes["test_dies_leaving_browser_running"]["problems"])
    assert any("desktop-blocked" in p for p in outcomes["test_unexpected_blocked_line"]["problems"])
    assert any("guard:" in p for p in outcomes["test_guard_violation_logged"]["problems"])
    profile = Path(outcomes["test_dies_leaving_browser_running"]["dir"]) / "profile"
    assert driver.pids_for_profile(profile) == [], "the runner left the abandoned browser running"
