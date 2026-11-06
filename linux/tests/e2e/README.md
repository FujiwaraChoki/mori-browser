# Mori end-to-end tests

A real browser build, with real pointer and keyboard input, in an isolated headless sway. Python
standard library plus `sway`, `wtype`, `grim`, `ss` and `coredumpctl`. Tests the last build at
`ungoogled-chromium-linux/build/src/out/Default`; run `linux/build.sh` first, or set `MORI_OUT`.

```
linux/tests/e2e/run.py              # everything
linux/tests/e2e/run.py -k smoke     # a tag, or part of a test id; repeat -k for OR
linux/tests/e2e/run.py --list
linux/tests/e2e/run.py -j 8 --timeout 180 --root /some/dir --json results.json
```

Exit code: 0 all good, 1 something failed, 2 the suite could not run. The verdicts:

| Verdict | Meaning | Run |
|---|---|---|
| PASS, SKIP | | ok |
| XFAIL | a known bug, failing on an assertion as marked | ok |
| XPASS | marked `@xfail` but passes: remove the marker | fails |
| FAIL | assertion, crash, guard violation, timeout, or something left running | fails |
| ERROR | the test could not be carried out (`DriverError`); `@xfail` does not hide this | fails |

Each test gets its own directory under `ungoogled-chromium-linux/e2e-runs/<time>/` (or
`$MORI_HARNESS_RUN/e2e-runs/`), with `chrome.log`, `test.log` and, on failure, `failure.png`. The
last five runs are kept; older ones go to the trash with `gio trash`, which refuses tmpfs, so under
`/tmp` they stay until you remove them. About 100 MB per full run.

## Safety

Browsers start only through `linux/harness/start.sh` and `run.sh`, so run.sh's guards always apply
(no session bus, portal, real home or helper apps). The driver also removes `WAYLAND_DISPLAY`,
`DISPLAY`, `HYPRLAND_*` and `DBUS_SESSION_BUS_ADDRESS` before it calls any script.

`run.py` enforces these from outside the test, so a test cannot skip them:

- With `ss`, every two seconds, and at each browser start and stop: no test browser process may be
  connected to `/run/user/*/bus`, at-spi, Hyprland's sockets, any Wayland socket but its own sway's,
  or an X11 display.
- `desktop-blocked.log` may hold only what the test allows with `@allow_blocked(regex)`.
- Crashes (`chrome.log`, coredumps of the test's processes, an exit nobody asked for) fail the test,
  even one marked `@xfail`.
- A browser or sway still running after the test fails it, and is stopped.

`test_guards.py` tests those checks, using `selfcheck/` for cases that fail on purpose.

## Adding a test

Add a `test_*` function to a `test_*.py` here. Name a parameter `b` to get a `Browser`, not yet
started and torn down for you; leave it out for a test that needs no browser. Each test runs in its
own process, with its own sway, profile, debug port and fixture server.

```python
from driver import expect, require, tags, xfail

@tags("smoke")
def test_click_reaches_the_page(b):
    url = b.fixture.page("hello", "ff00aa")           # 127.0.0.1, solid colour, records input
    b.start(url)
    require(lambda: b.wait_tab_urls([url]), 20, "setup: tab did not appear")
    x, y = b.page_origin()
    b.click(x + 300, y + 300)
    expect(lambda: b.eval("events.some(e => e.type == 'click')"), 5, "click did not arrive")
    assert b.wait_color((x + 100, y + 200, 50, 50), (0xff, 0, 0xaa)), "page not drawn"
```

- Use `assert` or `expect` for what you are testing, and `require` or `raise DriverError` for setup.
  Only assertions count toward `@xfail`, so a broken setup cannot pass as the known bug.
- Native Views chrome has no DOM. Assert on it with `b.grab()`: `.mean(region)`, `.is_color(...)`,
  `.diff_fraction(other, region)`, `.crop(...)`, `.save_png(...)`. `b.wait_color` retries, because
  frames present late.
- `b.screenshot(path)` writes a PNG to look at.

The `Browser` API, all in `driver.py`:

- Lifecycle: `start`, `restart(how=)`, `quit_clean`, `close_windows`, `sigterm`, `kill9`,
  `launch_and_kill`, `teardown`.
- Links from other apps: `open_link` (opens Little Mori, returns its toplevel).
- Tabs and pages: `tabs`, `tab_urls`, `wait_tab_urls`, `active_tab`, `activate_tab`, `eval`, `cdp`.
- Input: `click`, `right_click`, `double_click`, `move`, `scroll`, `drag`, `keys`, `type_text`,
  `hold_keyboard`.
- Compositor: `windows`, `window_tree`, `outputs`, `swaymsg`.
- Profile: `profile`, `default_profile`, `sessions_dir`, `session_files`, `session_contains`,
  `current_session_contains`,
  `preferences`, `local_state`, `downloads`, `home`, `chrome_log`, `blocked_log`.
- Page position: `page_origin` and `page_rect`. Mori insets the page, so `page_origin` measures it
  with the pointer; call it before opening an overlay.

## Traps

- `start()` keeps a live virtual keyboard on the seat (`hold_keyboard`). Without one, focus and hover
  tests give false results. It must also be the seat's *current* keyboard when a browser launches, or
  Chromium crashes in `XkbKeyboardLayoutEngine::UpdateModifiers`. A bare `wtype -s N` holder is not
  enough; `hold_keyboard` handles it. Its Shift press can appear in a page's key events.
- A dialog ignores clicks for its first half second. Wait after you first see it.
- wtype cannot send combos that xkb turns into NUL, such as Ctrl+2. Use Alt+2 for tabs.
- `/json/list` is not in tab-strip order. To check order, press Alt+N and read `active_tab()`.
- Session commands are saved after 2.5 s, and `exit_type` is written some seconds after launch. Before
  a `kill9`, call `wait_session_saved(...)` and `wait_marked_running()`, or it tests only those delays.
- `quit_clean` closes the windows through the compositor, not Quit Mori. To close them in a set
  order, use `close_windows`.
- A window closed while others stay open keeps its place in the session for 15 s. To test a close
  that really sticks, wait that long plus the save delay.
- The session rebuilds itself from open windows a few seconds after launch, which can hide bugs in
  what is written later. A page's state holds its URL in UTF-16, so search for titles that are not
  in the URL. A page with an `<input>` gets extra session writes from its form state.
- After a restart, `b.chrome_log` is the current launch; earlier launches are `chrome.N.log`.
