"""The session survives whatever order windows close in, and how the browser quits."""
import json
import time

from driver import expect, require, tags


FOLDER = "Workbench"


def _open_three(b):
    urls = [b.fixture.page(name) for name in ("first", "second", "third")]
    b.start(*urls)
    require(lambda: b.wait_tab_urls(urls), 20, "the opened tabs did not appear")
    return urls


def _command(b, name):
    """Run a command bar command by its name."""
    b.keys("ctrl+t")
    time.sleep(0.5)
    b.type_text(name)
    time.sleep(0.5)
    b.keys("Return")


def _select(b, combo, title):
    b.keys(combo)
    require(lambda: b.active_tab()["title"] == title, 5, f"{combo} did not select {title}")


def _new_folder(b, name):
    """Returns once the folder is named. It starts with one new tab of its own."""
    _command(b, "New Folder")
    time.sleep(0.8)
    b.type_text(name)
    b.keys("Return")
    time.sleep(0.3)


def _sidebar_content(b):
    """Three tabs, one a favorite, and a folder. Returns the URLs that should return."""
    urls = _open_three(b)
    _select(b, "alt+3", "third")
    b.keys("ctrl+shift+p")
    _select(b, "alt+2", "first")
    before = set(b.tab_urls())
    _new_folder(b, FOLDER)
    require(lambda: len(b.tabs()) == 4, 10, f"the folder did not add its tab: {b.tab_urls()}")
    b.wait_session_saved(FOLDER, "first", "second", "third")
    return b.tab_urls(), urls, before


def _assert_sidebar_back(b, expected, third):
    assert b.wait_tab_urls(expected, 20), f"tabs: {b.tab_urls()}, wanted {expected}"
    assert len(b.windows()) == 1, f"windows after relaunch: {len(b.windows())}"
    # The favorite is first in the strip, and the folder made it to this session.
    b.keys("alt+1")
    expect(lambda: b.active_tab()["url"] == third, 5, "the favorite is not first")
    expect(lambda: b.current_session_contains(FOLDER), 15, "the folder did not come back")


def _little(b, name="little"):
    url = b.fixture.page(name)
    return url, b.open_link(url)


def _main_window(b, little):
    return next(w for w in b.windows() if w["id"] != little["id"])


# F001 ------------------------------------------------------------------------

@tags("session", "little")
def test_little_open_close_main_then_little(b):
    expected, urls, _ = _sidebar_content(b)
    _, little = _little(b)
    b.close_windows([_main_window(b, little), little], gap=3)
    b.start()
    _assert_sidebar_back(b, expected, urls[2])


@tags("session", "little")
def test_little_open_close_little_then_main(b):
    expected, urls, _ = _sidebar_content(b)
    _, little = _little(b)
    b.close_windows([little, _main_window(b, little)], gap=3)
    b.start()
    _assert_sidebar_back(b, expected, urls[2])


@tags("session", "little")
def test_little_open_close_all_rapidly(b):
    """Omarchy's reboot and logout: every window, one after another."""
    expected, urls, _ = _sidebar_content(b)
    _, little = _little(b)
    b.close_windows([_main_window(b, little), little])
    b.start()
    _assert_sidebar_back(b, expected, urls[2])


@tags("session", "little")
def test_little_open_close_main_then_sigterm(b):
    """Logout where one window's close did not finish: Little is still open."""
    expected, urls, _ = _sidebar_content(b)
    _, little = _little(b)
    b.swaymsg(f"[con_id={_main_window(b, little)['id']}]", "kill")
    require(lambda: len(b.windows()) == 1, 10, "the main window did not close")
    b.sigterm()
    b.start()
    _assert_sidebar_back(b, expected, urls[2])


@tags("session")
def test_web_popup_open_close_main_then_popup(b):
    expected, urls, _ = _sidebar_content(b)
    before = {w["id"] for w in b.windows()}
    # A click: pages may not open popups without a user gesture.
    b.fixture.routes["/opener"] = (200, "text/html", "<!doctype html><title>opener</title>"
                                   "<style>html,body{height:100%;margin:0}</style><body onclick="
                                   "\"open('/page/popup','p','width=500,height=400')\">")
    opener = b.fixture.url("/opener")
    expected = sorted(expected + [opener])
    b.keys("ctrl+t")
    time.sleep(0.5)
    b.type_text(opener)
    b.keys("Return")
    require(lambda: b.wait_tab_urls(expected), 10, "the opener did not open")
    x, y = b.page_origin()
    b.click(x + 300, y + 300)
    popup = require(lambda: next((w for w in b.windows() if w["id"] not in before), None), 10,
                    "the popup did not open")
    b.wait_session_saved("opener")
    main = _main_window(b, popup)
    b.close_windows([main, popup], gap=3)
    b.start()
    assert b.wait_tab_urls(expected, 20), f"tabs: {b.tab_urls()}"
    assert len(b.windows()) == 1, f"windows: {len(b.windows())}"


@tags("session")
def test_two_windows_closed_together_both_return(b):
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    _command(b, "New Window")
    require(lambda: len(b.windows()) == 2, 10, "no second window")
    second = b.fixture.page("other")
    b.keys("ctrl+l")
    time.sleep(0.3)
    b.type_text(second)
    b.keys("Return")
    require(lambda: b.find_tab("other"), 10, "the second window's tab did not load")
    b.wait_session_saved("other")
    windows = b.windows()
    b.close_windows(windows, gap=1)
    b.start()
    wanted = sorted(urls + [second])
    assert b.wait_tab_urls(wanted, 20), f"tabs: {b.tab_urls()}, wanted {wanted}"
    assert len(b.windows()) == 2, f"windows: {len(b.windows())}"


@tags("session")
def test_window_closed_just_before_sigterm_returns(b):
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    _command(b, "New Window")
    require(lambda: len(b.windows()) == 2, 10, "no second window")
    second = b.fixture.page("other")
    b.keys("ctrl+l")
    time.sleep(0.3)
    b.type_text(second)
    b.keys("Return")
    require(lambda: b.find_tab("other"), 10, "the second window's tab did not load")
    b.wait_session_saved("other")
    focused = next(w for w in b.windows() if w.get("focused"))
    b.swaymsg(f"[con_id={focused['id']}]", "kill")
    require(lambda: len(b.windows()) == 1, 10, "the window did not close")
    b.sigterm()
    b.start()
    wanted = sorted(urls + [second])
    assert b.wait_tab_urls(wanted, 20), f"tabs: {b.tab_urls()}, wanted {wanted}"
    assert len(b.windows()) == 2, f"windows: {len(b.windows())}"


def _open_window(b, name):
    """A new window, focused, showing one fixture page."""
    count = len(b.windows())
    _command(b, "New Window")
    require(lambda: len(b.windows()) == count + 1, 10, "no new window")
    url = b.fixture.page(name)
    b.keys("ctrl+l")
    time.sleep(0.3)
    b.type_text(url)
    b.keys("Return")
    require(lambda: b.find_tab(name), 10, f"{name}'s tab did not load")
    b.wait_session_saved(name)
    return url


def _close_focused_window(b):
    count = len(b.windows())
    focused = next(w for w in b.windows() if w.get("focused"))
    b.swaymsg(f"[con_id={focused['id']}]", "kill")
    require(lambda: len(b.windows()) == count - 1, 10, "the window did not close")


def _reopen_window(b):
    count = len(b.windows())
    b.keys("ctrl+shift+t")
    require(lambda: len(b.windows()) == count + 1, 10, "the window did not reopen")


def _assert_no_clones(b, wanted, windows):
    wanted = sorted(wanted)
    assert b.wait_tab_urls(wanted, 20), f"tabs: {b.tab_urls()}, wanted {wanted}"
    time.sleep(2)   # A clone arrives after the first copy.
    assert b.tab_urls() == wanted, f"tabs: {b.tab_urls()}, wanted {wanted}"
    assert len(b.windows()) == windows, f"windows: {len(b.windows())}, wanted {windows}"


@tags("session")
def test_window_reopened_then_sigterm_does_not_clone(b):
    """Its close was still deferred when it reopened, so it was in the session twice."""
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    second = _open_window(b, "other")
    _close_focused_window(b)
    time.sleep(1)
    _reopen_window(b)
    b.sigterm()
    b.start()
    _assert_no_clones(b, urls + [second], 2)
    # And the session written after that restart is just as clean.
    time.sleep(3)
    b.sigterm()
    b.start()
    _assert_no_clones(b, urls + [second], 2)


@tags("session")
def test_window_reopened_then_all_closed_does_not_clone(b):
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    second = _open_window(b, "other")
    _close_focused_window(b)
    time.sleep(1)
    _reopen_window(b)
    b.close_windows(b.windows(), gap=0.5)
    b.start()
    _assert_no_clones(b, urls + [second], 2)


@tags("session")
def test_window_with_two_tabs_reopened_then_sigterm_does_not_clone(b):
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    second = _open_window(b, "other")
    b.keys("ctrl+t")
    time.sleep(0.5)
    third = b.fixture.page("another")
    b.type_text(third)
    b.keys("Return")
    require(lambda: b.find_tab("another"), 10, "the tab did not load")
    b.wait_session_saved("another")
    _close_focused_window(b)
    time.sleep(1)
    _reopen_window(b)
    b.sigterm()
    b.start()
    _assert_no_clones(b, urls + [second, third], 2)


@tags("session")
def test_reopening_one_window_keeps_the_other_deferred_close(b):
    """Only the reopened window's close is written. The other still returns."""
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    kept = _open_window(b, "kept")
    _close_focused_window(b)
    reopened = _open_window(b, "reopened")
    _close_focused_window(b)
    _reopen_window(b)
    assert b.find_tab("reopened"), f"reopened the wrong window: {b.tab_urls()}"
    b.sigterm()
    b.start()
    _assert_no_clones(b, urls + [kept, reopened], 3)


@tags("session", "little")
def test_reopen_with_only_little_left_does_not_clone(b):
    """Both windows closed together while Little stays, so both are pending, then both reopen."""
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    second = _open_window(b, "other")
    _, little = _little(b)
    time.sleep(1)
    for window in [w for w in b.windows() if w["id"] != little["id"]]:
        b.swaymsg(f"[con_id={window['id']}]", "kill")
        time.sleep(0.5)
    require(lambda: len(b.windows()) == 1, 10, "the windows did not close")
    time.sleep(1)
    b.keys("ctrl+shift+t")
    time.sleep(3)
    b.keys("ctrl+shift+t")
    time.sleep(3)
    b.sigterm()
    b.start()
    _assert_no_clones(b, urls + [second], 2)


@tags("session", "slow")
def test_window_closed_long_before_quit_stays_closed(b):
    """The other side of the deferral: a real close is a close, and can still be reopened."""
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    _command(b, "New Window")
    require(lambda: len(b.windows()) == 2, 10, "no second window")
    second = b.fixture.page("other")
    b.keys("ctrl+l")
    time.sleep(0.3)
    b.type_text(second)
    b.keys("Return")
    require(lambda: b.find_tab("other"), 10, "the second window's tab did not load")
    b.wait_session_saved("other")
    focused = next(w for w in b.windows() if w.get("focused"))
    b.swaymsg(f"[con_id={focused['id']}]", "kill")
    require(lambda: len(b.windows()) == 1, 10, "the window did not close")
    time.sleep(15 + 2.5 + 2)
    b.quit_clean()
    b.start()
    assert b.wait_tab_urls(urls, 20), f"tabs: {b.tab_urls()}, wanted {urls}"
    b.keys("ctrl+shift+t")
    assert b.wait_tab_urls(sorted(urls + [second]), 10), f"not reopened: {b.tab_urls()}"


# F019 ------------------------------------------------------------------------

@tags("session", "little")
def test_little_does_not_return_after_sigterm(b):
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    _little(b)
    time.sleep(3)
    b.sigterm()
    b.start()
    assert b.wait_tab_urls(urls, 20), f"tabs: {b.tab_urls()}"
    assert len(b.windows()) == 1, f"windows after SIGTERM: {len(b.windows())}"


@tags("session", "little")
def test_open_in_mori_with_only_little_open_restores_the_session(b):
    """Only Little is left, so a normal window brings the session back, and the tab joins it."""
    expected, urls, _ = _sidebar_content(b)
    little_url, little = _little(b)
    b.swaymsg(f"[con_id={_main_window(b, little)['id']}]", "kill")
    require(lambda: len(b.windows()) == 1, 10, "the main window did not close")
    time.sleep(1.5)
    window = b.windows()[0]["rect"]
    b.click(window["x"] + window["width"] - 100, window["y"] + 22)  # Open in Mori
    wanted = sorted(expected + [little_url])
    require(lambda: b.tabs(), 5, "no tabs")
    assert b.wait_tab_urls(wanted, 15), f"tabs: {b.tab_urls()}, wanted {wanted}"
    time.sleep(3.5)
    b.sigterm()
    b.start()
    assert b.wait_tab_urls(wanted, 20), f"Open in Mori was not saved: {b.tab_urls()}, {wanted}"


# F158 ------------------------------------------------------------------------

@tags("session", "smoke")
def test_quit_from_command_bar_keeps_all_windows(b):
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    _command(b, "New Window")
    require(lambda: len(b.windows()) == 2, 10, "no second window")
    b.keys("ctrl+l")
    other = b.fixture.page("other")
    time.sleep(0.3)
    b.type_text(other)
    b.keys("Return")
    require(lambda: b.find_tab("other"), 10, "the second window's tab did not load")
    b.wait_session_saved("other")
    b._before_stop()
    _command(b, "Quit Mori")
    b._wait_gone(20, "Quit Mori did not quit")
    b._after_stop()
    b.start()
    wanted = sorted(urls + [other])
    assert b.wait_tab_urls(wanted, 20), f"tabs: {b.tab_urls()}"
    assert len(b.windows()) == 2, f"windows: {len(b.windows())}"


NO_NETWORK = ("--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1",)


@tags("session", "smoke")
def test_command_bar_searches_do_not_quit(b):
    """Commands rank above search and Enter runs the first row. So nothing short of asking
    for Quit may match it: the prefix of a search, or a word it has in common with it."""
    url = b.fixture.page("first")
    b.start(url, args=NO_NETWORK)
    require(lambda: b.wait_tab_urls([url]), 20, "the opened tab did not appear")
    for query in ("q", "qu", "qui", "quite", "quit smoking", "exit", "windows", "all", "close all",
                  "mori quit now"):
        before = len(b.tabs())
        b.keys("ctrl+t")
        time.sleep(0.5)
        b.type_text(query)
        time.sleep(0.8)
        b.keys("Return")
        time.sleep(1.5)
        assert b.pids(), f"{query!r} and Enter quit the browser"
        b.check_alive()
        assert len(b.tabs()) >= before, f"{query!r} closed a tab: {b.tab_urls()}"


@tags("session")
def test_quit_from_command_bar_by_word(b):
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    b._before_stop()
    _command(b, "quit")
    b._wait_gone(20, "the word quit did not quit")
    b._after_stop()
    b.start()
    assert b.wait_tab_urls(urls, 20), f"tabs: {b.tab_urls()}"


@tags("session")
def test_quit_shortcut_needs_two_presses(b):
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    b.keys("ctrl+shift+q")
    time.sleep(4)   # Longer than the confirmation lasts.
    assert b.pids(), "one press quit the browser"
    b.check_alive()
    b.keys("ctrl+shift+q")
    time.sleep(1)
    assert b.tabs(), "the browser went away"
    b._before_stop()
    b.keys("ctrl+shift+q")
    b._wait_gone(20, "the second press did not quit")
    b._after_stop()
    b.start()
    assert b.wait_tab_urls(urls, 20), f"tabs: {b.tab_urls()}"


# F012 ------------------------------------------------------------------------

@tags("session")
def test_folder_made_just_before_sigterm_returns(b):
    _open_three(b)
    b.wait_session_saved("first", "second", "third")
    _new_folder(b, FOLDER)
    b.sigterm()
    assert b.session_contains(FOLDER), "the folder was not flushed at SIGTERM"
    b.start()
    expect(lambda: b.current_session_contains(FOLDER), 15, "the folder did not come back")


@tags("session")
def test_sidebar_width_dragged_just_before_sigterm_returns(b):
    _open_three(b)
    b.wait_session_saved("first", "second", "third")
    x, y = b.page_origin()
    middle = y + 300
    # The resize grip is on the sidebar's edge, just left of the page.
    b.drag(x - 4, middle, x + 96, middle)
    b._origin_size = None
    moved, _ = b.page_origin()
    require(lambda: abs(moved - x) > 40, 1, f"the drag did not resize: {x} to {moved}")
    b.sigterm()
    b.start()
    require(lambda: b.tabs(), 20, "no tabs")
    time.sleep(1)
    b._origin_size = None
    back, _ = b.page_origin()
    assert abs(back - moved) <= 3, f"the page starts at {back}, it was dragged to {moved}, from {x}"


# F067 ------------------------------------------------------------------------

@tags("session")
def test_active_tab_moved_stays_active(b):
    urls = _open_three(b)
    _select(b, "alt+3", "third")
    b.keys("ctrl+shift+Prior")
    b.wait_session_saved("first", "second", "third")
    b.sigterm()
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, "tabs did not return")
    expect(lambda: b.active_tab()["url"] == urls[2], 8,
           f"the moved active tab is not active: {b.active_tab()['url']}")
    # And the move itself survived: third is now second.
    b.keys("alt+2")
    expect(lambda: b.active_tab()["url"] == urls[2], 5, "the move did not survive")


@tags("session")
def test_active_tab_made_favorite_stays_active(b):
    urls = _open_three(b)
    _select(b, "alt+3", "third")
    b.keys("ctrl+shift+p")
    b.wait_session_saved("first", "second", "third")
    b.sigterm()
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, "tabs did not return")
    expect(lambda: b.active_tab()["url"] == urls[2], 8,
           f"the favorite is not active: {b.active_tab()['url']}")


# F032 ------------------------------------------------------------------------

def _pages_stay(b, urls, seconds=3):
    time.sleep(seconds)
    assert b.tab_urls() == sorted(urls), f"tabs changed: {b.tab_urls()}"
    assert len(b.windows()) == 1, f"windows: {len(b.windows())}"


@tags("session", "smoke")
def test_reopen_after_restart_does_not_clone_the_window(b):
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    b.quit_clean()
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, "tabs did not return")
    time.sleep(1)
    b.keys("ctrl+shift+t")
    _pages_stay(b, urls)
    # Not in a later session either: consumed for good, even with nothing to replace it.
    time.sleep(3)
    b.sigterm()
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, "tabs did not return")
    time.sleep(1)
    b.keys("ctrl+shift+t")
    _pages_stay(b, urls)


@tags("session")
def test_reopen_after_many_restarts_reaches_the_closed_tab(b):
    urls = _open_three(b)
    _select(b, "alt+3", "third")
    b.keys("ctrl+w")
    require(lambda: b.wait_tab_urls(urls[:2]), 10, "Ctrl+W did not close the tab")
    b.wait_session_saved("first", "second")
    for _ in range(2):
        b.quit_clean()
        b.start()
        require(lambda: b.wait_tab_urls(urls[:2]), 20, "tabs did not return")
        time.sleep(4)
    b.keys("ctrl+shift+t")
    assert b.wait_tab_urls(urls, 8), f"the closed tab did not reopen: {b.tab_urls()}"
    assert len(b.windows()) == 1, f"windows: {len(b.windows())}"


@tags("session")
def test_window_entry_stays_when_startup_does_not_restore(b):
    """With a New Tab startup, the reopen list is the only way back to that window."""
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    b.quit_clean()
    prefs_path = b.default_profile / "Preferences"
    prefs = json.loads(prefs_path.read_text())
    prefs.setdefault("session", {})["restore_on_startup"] = 5
    prefs_path.write_text(json.dumps(prefs))
    b.start()
    require(lambda: b.tabs(), 20, "no tabs")
    time.sleep(2)
    require(lambda: not (set(b.tab_urls()) & set(urls)), 3,
            f"the session was restored anyway: {b.tab_urls()}")
    b.keys("ctrl+shift+t")
    expect(lambda: set(urls) <= set(b.tab_urls()), 10, f"not reopened: {b.tab_urls()}")


# F180 ------------------------------------------------------------------------

def _static_pages(b, names):
    """Pages with a <title> and nothing else, so nothing rewrites their session entry later:
    the commit is recorded before the title arrives. No script, unlike fixture.page()."""
    urls = {}
    for n, name in enumerate(names):
        path = f"/static/{n}"
        b.fixture.routes[path] = (200, "text/html",
                                  f"<!doctype html><title>{name}</title><p>{name}")
        urls[b.fixture.url(path)] = name
    return urls


@tags("session")
def test_title_of_a_static_page_reaches_the_session(b):
    """An unloaded restored tab shows what the session has, so with no title, its URL."""
    titles = _static_pages(b, ["Heading 1", "Heading 2", "Heading 3"])
    urls = list(titles)
    b.start(*urls)
    require(lambda: b.wait_tab_urls(urls), 20, "tabs did not appear")
    require(lambda: all(t["title"] == titles[t["url"]] for t in b.tabs()), 15, "no titles came")
    for title in titles.values():
        expect(lambda: b.current_session_contains(title, wide=True), 8, f"no title on disk: {title}")
    # Nothing answers quickly now, so right after the restart no tab has loaded.
    for path in [f"/static/{n}" for n in range(3)]:
        b.fixture.delays[path] = 8
    b.sigterm()
    b.start()
    require(lambda: b.wait_tab_urls(urls), 5, "tabs did not return")
    found = {t["url"]: t["title"] for t in b.tabs()}
    assert found == titles, f"titles after restart: {found}"


@tags("session")
def test_title_changed_by_a_page_reaches_the_session(b):
    """Long after the load, when nothing else about the page changes."""
    titles = _static_pages(b, ["Before 1", "Before 2"])
    urls = list(titles)
    b.start(*urls)
    require(lambda: b.wait_tab_urls(urls), 20, "tabs did not appear")
    b.wait_session_saved()
    time.sleep(6)
    for n, tab in enumerate(b.tabs()):
        b.eval(f"document.title='After {n}'", tab)
    for n in range(len(urls)):
        expect(lambda: b.current_session_contains(f"After {n}", wide=True), 8, f"After {n} not on disk")
