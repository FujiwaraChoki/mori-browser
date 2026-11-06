"""Cases for test_guards.py::test_runner_fails_when_it_must. Some fail on purpose.
Not found by a normal run: run.py only looks at the top level of its tests directory."""
import os
import time

import driver
from driver import DriverError, Skip, allow_blocked, timeout, xfail


def test_passes():
    pass


def test_assertion():
    assert False, "on purpose"


def test_driver_error():
    raise DriverError("on purpose")


def test_skip():
    raise Skip("on purpose")


@xfail("known")
def test_marked_and_fails():
    assert False, "on purpose"


@xfail("known")
def test_marked_and_passes():
    pass


@xfail("known")
def test_marked_but_setup_broke():
    raise DriverError("on purpose")


@xfail("known")
def test_marked_but_crashed(b):
    b.crashes.append("on purpose")
    assert False, "on purpose"


def test_unexpected_blocked_line(b):
    b.blocked_log.write_text("12:00:00 xdg-open https://example.com\n")


@allow_blocked(r"xdg-settings check")
def test_allowed_blocked_line(b):
    b.blocked_log.write_text("12:00:00 xdg-settings check default-web-browser\n")


def test_guard_violation_logged(b):
    (b.hl / driver.GUARD_LOG).write_text("on purpose\n")


@timeout(2)
def test_times_out():
    time.sleep(120)


def test_dies_leaving_browser_running(b):
    b.start(b.fixture.page("left"))
    os._exit(0)
