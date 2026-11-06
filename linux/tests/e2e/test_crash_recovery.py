"""An unclean exit reopens the session by itself, and never costs the user anything."""
import json
import os
import time

from driver import expect, poll, require, tags, timeout
from test_session_close import _assert_sidebar_back, _sidebar_content

# From the blue Restore button to the bubble's X, when it holds the crash-loop text.
CLOSE_FROM_RESTORE = (36, -144)
RECOVERY_WINDOW = 60


def _open_three(b):
    urls = [b.fixture.page(name) for name in ("first", "second", "third")]
    b.start(*urls)
    require(lambda: b.wait_tab_urls(urls), 20, "the opened tabs did not appear")
    b.wait_session_saved("first", "second", "third")
    return urls


def _crash(b):
    """Without the wait a kill reads as a clean exit, and the test proves nothing."""
    b.wait_marked_running()
    b.kill9()


def _crash_twice(b):
    """Crash, it reopens, crash again straight away. Ends stopped: the next start is guarded."""
    urls = _open_three(b)
    _crash(b)
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, f"the first crash did not restore: {b.tab_urls()}")
    _crash(b)
    return urls


def _backups(b):
    return sorted(b.sessions_dir.glob("Crashed Session_*"))


def _held(b):
    return sorted(b.sessions_dir.glob("Held Session_*"))


def _holding(paths, text=b"first"):
    return [path.name for path in paths if text in path.read_bytes()]


def _restore(b, urls):
    """The bubble is there, and its Restore brings back these pages, once each."""
    button = _wait_bubble(b)
    assert button, f"no bubble, and on disk: {[p.name for p in b.sessions_dir.iterdir()]}"
    b.click(*button)
    assert b.wait_tab_urls(urls, 20) or set(urls) <= set(b.tab_urls()), f"Restore: {b.tab_urls()}"
    found = [url for url in b.tab_urls() if url in urls]
    assert sorted(found) == sorted(urls), f"opened twice: {b.tab_urls()}"


def _restore_button(b):
    """Centre of the bubble's blue button on screen, or None. It sits over the middle of
    the page, and nothing else in these windows is that blue."""
    image = b.grab()
    found = []
    for y in range(200, 800, 3):
        for x in range(300, 1300, 3):
            r, g, blue = image.pixel(x, y)
            if blue > 170 and blue - r > 100:
                found.append((x, y))
    if len(found) < 100:
        return None
    return (sum(p[0] for p in found) // len(found), sum(p[1] for p in found) // len(found))


def _wait_bubble(b):
    """Its centre once it takes clicks. A dialog ignores them for its first half second."""
    button = poll(lambda: _restore_button(b), 10)
    time.sleep(0.7)
    return button


def _bubble_stays_away(b, seconds=3):
    """It would appear within moments of the window. Frames present late."""
    end = time.time() + seconds
    while time.time() < end:
        if _restore_button(b):
            return False
        time.sleep(0.3)
    return True


def _no_stray_window(b, expected):
    windows = b.windows()
    assert len(windows) == expected, f"toplevels: {[w.get('name') for w in windows]}"
    names = [w.get("name") or "" for w in windows]
    assert not any("Restore pages" in name for name in names), f"a bubble has a tile: {names}"
    # A bubble mapped as a toplevel has no app id; every Mori window has one.
    assert all(w.get("app_id") for w in windows), f"a toplevel without app id: {windows}"


def _add_tab(b, name):
    """Through the UI, so it reaches this session like any tab the user opens."""
    url = b.fixture.page(name)
    b.keys("ctrl+t")
    b.type_text(url)
    b.keys("Return")
    require(lambda: url in b.tab_urls(), 10, f"{name} did not open: {b.tab_urls()}")
    return url


# F002: restores by itself -------------------------------------------------------------------


@tags("crash", "smoke")
def test_crash_restores_sidebar_without_asking(b):
    """Favorite, folder and tabs all come back, and nothing covers the sidebar."""
    expected, urls, _ = _sidebar_content(b)
    _crash(b)
    b.start()
    _assert_sidebar_back(b, expected, urls[2])
    assert _bubble_stays_away(b), "asked about a first crash"
    assert _backups(b) and b"first" in _backups(b)[-1].read_bytes(), \
        f"the crashed session has no backup: {list(b.sessions_dir.iterdir())}"


@tags("crash")
def test_crash_restores_a_big_session(b):
    urls = [b.fixture.page(f"tab{n}") for n in range(36)]
    b.start(*urls)
    require(lambda: b.wait_tab_urls(urls), 40, "the opened tabs did not appear")
    b.wait_session_saved("tab0", "tab35", timeout=30)
    _crash(b)
    before = len(b.fixture.requests)
    b.start()
    assert b.wait_tab_urls(urls, 30), f"tabs: {len(b.tab_urls())} of {len(urls)}"
    # A crash may come from one of them, so they load a few at a time, as in any restore.
    loaded = len(b.fixture.requests) - before
    assert loaded < len(urls), f"all {loaded} pages loaded at once"
    _no_stray_window(b, 1)
    assert _bubble_stays_away(b), "asked about a first crash"
    b.screenshot(b.hl / "restored.png")


@tags("crash")
def test_crash_with_a_link_at_launch_restores_too(b):
    urls = _open_three(b)
    _crash(b)
    link = b.fixture.page("link")
    b.start(link)
    assert b.wait_tab_urls(urls + [link], 20), f"tabs: {b.tab_urls()}"
    _no_stray_window(b, 1)
    assert _bubble_stays_away(b), "asked about a first crash"


@tags("crash")
def test_work_after_a_crash_is_saved(b):
    """Variant B: this used to run with saving off, and went backwards at the next launch."""
    urls = _open_three(b)
    _crash(b)
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, f"did not restore: {b.tab_urls()}")
    work = _add_tab(b, "work")
    b.wait_session_saved("work")
    assert b.current_session_contains("work"), "saving is off after a crash"
    b.quit_clean()
    assert b.preferences()["profile"]["exit_type"] == "Normal", "the crash mark stayed"
    b.start()
    assert b.wait_tab_urls(urls + [work], 20), f"work was lost: {b.tab_urls()}"
    assert _bubble_stays_away(b), "asked after a clean exit"


@tags("crash")
def test_second_crash_keeps_later_work(b):
    """Saving on means a crash after a crash loses at most the last seconds."""
    urls = _open_three(b)
    _crash(b)
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, f"did not restore: {b.tab_urls()}")
    work = _add_tab(b, "work")
    b.wait_session_saved("work")
    _crash(b)
    b.start()
    button = _wait_bubble(b)
    assert button, "two crashes inside a minute should ask"
    b.click(*button)
    assert b.wait_tab_urls(urls + [work], 20), f"work was lost: {b.tab_urls()}"


@tags("crash")
def test_little_after_a_crash_does_not_replace_the_session(b):
    """A link from another app used to count as an answer and start a new session."""
    urls = _open_three(b)
    _crash(b)
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, f"did not restore: {b.tab_urls()}")
    b.open_link(b.fixture.page("little"))
    _no_stray_window(b, 2)
    time.sleep(3.5)
    b.quit_clean()
    b.start()
    assert b.wait_tab_urls(urls, 20), f"tabs: {b.tab_urls()}"
    _no_stray_window(b, 1)


@tags("crash")
def test_reopen_after_a_crash_restore_does_not_clone(b):
    """The crashed session is open again, so it is not also in the reopen list."""
    urls = _open_three(b)
    _crash(b)
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, f"did not restore: {b.tab_urls()}")
    b.keys("alt+3")
    b.keys("ctrl+w")
    require(lambda: b.wait_tab_urls(urls[:2]), 10, f"Ctrl+W: {b.tab_urls()}")
    b.keys("ctrl+shift+t")
    assert b.wait_tab_urls(urls, 10), f"Ctrl+Shift+T: {b.tab_urls()}"
    b.keys("ctrl+shift+t")
    time.sleep(2)
    assert sorted(b.tab_urls()) == sorted(urls), f"reopened a clone: {b.tab_urls()}"
    assert len(b.windows()) == 1, f"windows: {len(b.windows())}"


# The guard -----------------------------------------------------------------------------------


@tags("crash")
def test_crash_loop_asks_and_restores_once(b):
    urls = _crash_twice(b)
    b.start()
    assert b.wait_tab_urls(["chrome://newtab/"], 20), f"reopened into the loop: {b.tab_urls()}"
    button = _wait_bubble(b)
    assert button, "no bubble on a crash loop"
    b.screenshot(b.hl / "guard.png")
    x, _ = button
    assert x > 186 + 300, f"the bubble is by the sidebar: {button}"
    _no_stray_window(b, 1)
    b.click(*button)
    assert b.wait_tab_urls(urls, 20), f"Restore: {b.tab_urls()}"
    expect(lambda: not _restore_button(b), 5, "the bubble stayed")
    _no_stray_window(b, 1)


@tags("crash")
def test_crash_loop_restore_that_crashes_asks_again(b):
    _crash_twice(b)
    b.start()
    b.click(*require(lambda: _wait_bubble(b), 10, "no bubble"))
    require(lambda: "chrome://newtab/" not in b.tab_urls(), 20, f"Restore: {b.tab_urls()}")
    _crash(b)
    b.start()
    assert b.wait_tab_urls(["chrome://newtab/"], 20), f"reopened into the loop: {b.tab_urls()}"
    assert _wait_bubble(b), "no bubble after Restore crashed"


@tags("crash")
def test_crash_loop_link_shows_no_second_bubble(b):
    """Unanswered: the link opens Little Mori and no 'Restore pages?' tile, ever."""
    _crash_twice(b)
    b.start()
    require(lambda: _restore_button(b), 10, "no bubble")
    b.open_link(b.fixture.page("little1"))
    time.sleep(1)
    _no_stray_window(b, 2)
    b.screenshot(b.hl / "link-unanswered.png")
    second = b.open_link(b.fixture.page("little2"))
    time.sleep(1)
    _no_stray_window(b, 3)
    assert second.get("app_id")


@tags("crash")
def test_crash_loop_link_after_restore_does_not_duplicate(b):
    """Answered: this bubble's Restore used to open a second copy of the whole session."""
    urls = _crash_twice(b)
    b.start()
    b.click(*require(lambda: _wait_bubble(b), 10, "no bubble"))
    require(lambda: b.wait_tab_urls(urls), 20, f"Restore: {b.tab_urls()}")
    little = b.fixture.page("little")
    b.open_link(little)
    time.sleep(1)
    _no_stray_window(b, 2)
    assert sorted(b.tab_urls()) == sorted(urls + [little]), f"tabs: {b.tab_urls()}"


@tags("crash")
def test_crash_loop_dismissed_loses_nothing(b):
    """X, then work, then quit. The work returns; the old session is one Ctrl+Shift+T away and
    in its backup."""
    urls = _crash_twice(b)
    b.start()
    x, y = require(lambda: _wait_bubble(b), 10, "no bubble")
    b.click(x + CLOSE_FROM_RESTORE[0], y + CLOSE_FROM_RESTORE[1])
    expect(lambda: not _restore_button(b), 5, "X did not close the bubble")
    work = _add_tab(b, "work")
    b.wait_session_saved("work")
    assert b.current_session_contains("work"), "saving is off after dismissing"
    b.quit_clean()
    b.start()
    assert work in b.tab_urls() or b.wait_tab_urls([work, "chrome://newtab/"], 20), \
        f"work was lost: {b.tab_urls()}"
    assert _bubble_stays_away(b), "asked again after a clean exit"
    assert any(b"first" in path.read_bytes() for path in _backups(b)), "no backup"
    assert not (set(b.tab_urls()) & set(urls)), f"restored unasked: {b.tab_urls()}"
    b.keys("ctrl+shift+t")
    expect(lambda: set(urls) <= set(b.tab_urls()), 10, f"Ctrl+Shift+T: {b.tab_urls()}")


@tags("crash")
def test_crash_loop_ignored_bubble_loses_nothing(b):
    """Variant B with the bubble left open: saving is never off, and it stays an open question."""
    urls = _crash_twice(b)
    b.start()
    require(lambda: _restore_button(b), 10, "no bubble")
    work = _add_tab(b, "work")
    b.wait_session_saved("work")
    assert b.current_session_contains("work"), "saving is off while the bubble is open"
    b.quit_clean()
    b.start()
    expect(lambda: work in b.tab_urls(), 20, f"work was lost: {b.tab_urls()}")
    _restore(b, urls)
    expect(lambda: work in b.tab_urls(), 5, f"work is gone after Restore: {b.tab_urls()}")


@tags("crash")
def test_repeat_guard_restore_brings_the_pages_back(b):
    """The empty guarded session is the newest file. Restore means the pages, not that."""
    urls = _crash_twice(b)
    b.start()
    require(lambda: _restore_button(b), 10, "no bubble")
    b.wait_session_saved("newtab")
    _crash(b)
    b.start()
    _restore(b, urls)
    assert len(b.windows()) == 1, f"windows: {len(b.windows())}"
    b.keys("ctrl+shift+t")
    time.sleep(2)
    assert sorted(b.tab_urls()) == sorted(urls), f"reopened a clone: {b.tab_urls()}"


@tags("crash")
def test_guard_killed_around_its_first_save(b):
    """Its first save comes about 3 s in, and used to beat the old session's other copy to disk."""
    urls = _crash_twice(b)
    for seconds in (2.8, 3.0, 3.2, 3.3, 3.4, 3.5, 3.6, 3.8):
        b.launch_and_kill(seconds)
        assert _holding(_held(b)), f"after {seconds}: {[p.name for p in b.sessions_dir.iterdir()]}"
    b.start()
    _restore(b, urls)


@tags("crash")
@timeout(300)
def test_long_crash_chain_keeps_the_session(b):
    """A chain that used to lose every copy of the session."""
    urls = _open_three(b)
    _crash(b)
    for seconds in (0.8, 4, 3.2, 0.5, 1.6, 4, 2.6, 5, 4, 0.2, 4, 0.05, 2.6, 1.2):
        b.launch_and_kill(seconds)
    b.start()
    if set(urls) <= set(b.tab_urls()):
        return
    _restore(b, urls)


@tags("crash")
@timeout(240)
def test_blank_guard_sessions_do_not_push_out_the_backup(b):
    urls = _crash_twice(b)
    for seconds in (4, 3.5, 3.5, 3.5, 3.5, 3.5):
        b.launch_and_kill(seconds)
    assert _holding(_held(b)), f"not held: {[p.name for p in b.sessions_dir.iterdir()]}"
    assert len(_backups(b)) <= 3
    assert all(_holding([path]) for path in _backups(b)), "a blank session took a backup's place"
    b.start()
    x, y = require(lambda: _wait_bubble(b), 10, "no bubble")
    b.click(x + CLOSE_FROM_RESTORE[0], y + CLOSE_FROM_RESTORE[1])
    expect(lambda: not _held(b), 10, f"still held: {_held(b)}")
    assert _holding(_backups(b)), f"no backup: {[p.name for p in _backups(b)]}"
    b.keys("ctrl+shift+t")
    expect(lambda: set(urls) <= set(b.tab_urls()), 10, f"Ctrl+Shift+T: {b.tab_urls()}")


@tags("crash")
@timeout(240)
def test_ignored_guard_then_a_late_crash_still_offers_the_pages(b):
    """A minute in, a crash is no loop, and this launch's own session reopens. That one is blank,
    so without the offer this was a blank sidebar with no way back in sight."""
    urls = _crash_twice(b)
    b.start()
    require(lambda: _restore_button(b), 10, "no bubble")
    time.sleep(RECOVERY_WINDOW + 2)
    assert b.preferences()["profile"]["exit_type"] == "Crashed", "the recovery window never ended"
    b.kill9()
    b.start()
    _restore(b, urls)


@tags("crash")
def test_quitting_a_guard_at_once_still_offers_the_pages(b):
    urls = _crash_twice(b)
    b.start()
    b.quit_clean()
    b.start()
    _restore(b, urls)


@tags("crash")
def test_work_in_a_guard_then_a_crash_restores_both(b):
    urls = _crash_twice(b)
    b.start()
    require(lambda: _restore_button(b), 10, "no bubble")
    work = _add_tab(b, "work")
    b.wait_session_saved("work")
    _crash(b)
    b.start()
    _restore(b, urls)
    expect(lambda: work in b.tab_urls(), 10, f"work was lost: {b.tab_urls()}")


@tags("crash")
def test_killed_right_after_restore_loses_nothing(b):
    """The held session lets go only once this session's file has its pages."""
    urls = _crash_twice(b)
    b.start()
    b.click(*require(lambda: _wait_bubble(b), 10, "no bubble"))
    require(lambda: set(urls) <= set(b.tab_urls()), 20, f"Restore: {b.tab_urls()}")
    b.kill9()
    b.start()
    _restore(b, urls)


@tags("crash")
def test_restore_lets_go_of_the_held_session(b):
    urls = _crash_twice(b)
    b.start()
    _restore(b, urls)
    expect(lambda: not _held(b), 10, f"still held: {_held(b)}")
    assert b.current_session_contains("first"), "released before this session had the pages"
    assert _holding(_backups(b)), f"no backup: {[p.name for p in _backups(b)]}"
    b.quit_clean()
    b.start()
    assert b.wait_tab_urls(urls, 20), f"tabs: {b.tab_urls()}"
    assert _bubble_stays_away(b), "asked again after answering"


@tags("crash")
def test_killed_right_after_dismissing_loses_nothing(b):
    urls = _crash_twice(b)
    b.start()
    x, y = require(lambda: _wait_bubble(b), 10, "no bubble")
    b.click(x + CLOSE_FROM_RESTORE[0], y + CLOSE_FROM_RESTORE[1])
    b.kill9()
    b.start()
    if _wait_bubble(b):
        _restore(b, urls)
    else:
        b.keys("ctrl+shift+t")
        expect(lambda: set(urls) <= set(b.tab_urls()), 10, f"Ctrl+Shift+T: {b.tab_urls()}")


@tags("crash")
def test_a_crash_loop_with_nothing_to_restore_does_not_ask(b):
    """Restore would do nothing."""
    b.start()
    b.wait_session_saved("newtab")
    _crash(b)
    b.start()
    _crash(b)
    b.start()
    assert _bubble_stays_away(b, 5), "offered to restore nothing"
    assert not _held(b) and not _backups(b), f"kept a blank session: {_held(b)} {_backups(b)}"


@tags("crash")
def test_crash_during_startup_counts(b):
    """The running mark is on disk at once. If it waited for the next timed write, a session
    that crashes as it loads would read as a clean exit, and reopen forever."""
    urls = _open_three(b)
    b.quit_clean()
    b.start()
    assert poll(lambda: b.preferences()["profile"]["exit_type"] == "Crashed", 3), \
        f"exit_type: {b.preferences()['profile']['exit_type']}"
    b.kill9()
    b.start()
    assert poll(lambda: b.preferences()["profile"]["exit_type"] == "CrashedInRecovery", 3), \
        f"exit_type: {b.preferences()['profile']['exit_type']}"
    b.kill9()
    b.start()
    assert _wait_bubble(b), "no guard for crashes during startup"
    assert not (set(b.tab_urls()) & set(urls)), f"reopened into the loop: {b.tab_urls()}"


@tags("crash")
def test_clean_exit_ends_the_recovery_window(b):
    urls = _open_three(b)
    _crash(b)
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, f"did not restore: {b.tab_urls()}")
    b.quit_clean()
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, f"did not restore: {b.tab_urls()}")
    _crash(b)
    b.start()
    assert b.wait_tab_urls(urls, 20), f"tabs: {b.tab_urls()}"
    assert _bubble_stays_away(b), "a clean exit in between is no crash loop"


@tags("crash")
@timeout(240)
def test_crash_after_a_stable_minute_is_no_loop(b):
    urls = _open_three(b)
    _crash(b)
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, f"did not restore: {b.tab_urls()}")
    assert b.preferences()["profile"]["exit_type"] == "CrashedInRecovery"
    time.sleep(RECOVERY_WINDOW + 2)
    assert b.preferences()["profile"]["exit_type"] == "Crashed", "the recovery window never ended"
    b.kill9()
    b.start()
    assert b.wait_tab_urls(urls, 20), f"a crash a minute later is no loop: {b.tab_urls()}"
    assert _bubble_stays_away(b), "asked about a crash a minute later"


# A startup that does not reopen the session anyway ---------------------------------------------


def _new_tab_startup(b):
    b.quit_clean()
    path = b.default_profile / "Preferences"
    prefs = json.loads(path.read_text())
    prefs.setdefault("session", {})["restore_on_startup"] = 5
    path.write_text(json.dumps(prefs))


@tags("crash")
def test_new_tab_startup_still_offers_the_session_after_a_crash(b):
    urls = _open_three(b)
    _new_tab_startup(b)
    b.start(*urls)
    require(lambda: b.wait_tab_urls(urls + ["chrome://newtab/"]) or b.wait_tab_urls(urls), 20,
            f"tabs: {b.tab_urls()}")
    b.wait_session_saved("first")
    _crash(b)
    b.start()
    assert b.wait_tab_urls(["chrome://newtab/"], 20), f"reopened against the setting: {b.tab_urls()}"
    button = _wait_bubble(b)
    assert button, "no offer to restore"
    b.click(*button)
    assert b.wait_tab_urls(urls, 20), f"Restore: {b.tab_urls()}"
    little = b.fixture.page("little")
    b.open_link(little)
    time.sleep(1)
    _no_stray_window(b, 2)
    assert sorted(b.tab_urls()) == sorted(urls + [little]), f"tabs: {b.tab_urls()}"


# Backups ---------------------------------------------------------------------------------------


@tags("crash")
def test_backups_are_limited_in_number_and_age(b):
    urls = _open_three(b)
    _crash(b)
    newest = max(path.name for path in b.session_files())
    number = int(newest.split("_")[1])
    source = b.session_files()[-1].read_bytes()
    old = b.sessions_dir / f"Crashed Session_{number - 100}"
    old.write_bytes(source)
    month_ago = time.time() - 31 * 86400
    os.utime(old, (month_ago, month_ago))
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, f"did not restore: {b.tab_urls()}")
    expect(lambda: not old.exists(), 10, "a backup outlived its 30 days")
    b.quit_clean()
    for i in range(1, 5):
        (b.sessions_dir / f"Crashed Session_{number - i}").write_bytes(source)
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, f"did not restore: {b.tab_urls()}")
    _crash(b)
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, f"did not restore: {b.tab_urls()}")
    expect(lambda: len(_backups(b)) == 3, 10, f"backups: {[p.name for p in _backups(b)]}")
    kept = [path.name for path in _backups(b)]
    assert f"Crashed Session_{number - 4}" not in kept, f"kept the older ones: {kept}"


@tags("crash")
def test_clearing_history_deletes_the_held_session(b):
    urls = _crash_twice(b)
    b.start()
    require(lambda: _held(b), 10, "nothing held")
    tab = b.find_tab("chrome://newtab")
    settings = "chrome://settings/clearBrowserData"
    b.cdp(tab, "Page.navigate", {"url": settings})
    require(lambda: settings in b.tab_urls(), 10, f"no settings page: {b.tab_urls()}")
    tab = b.find_tab("chrome://settings")
    b.eval("chrome.send('initializeClearBrowsingData', ['cb0'])", tab)
    b.eval("chrome.send('clearBrowsingData', ['cb1', ['browser.clear_data.browsing_history'], 4])",
           tab)
    expect(lambda: not _held(b), 15, f"still held: {_held(b)}")


@tags("crash")
def test_clearing_history_deletes_the_backups(b):
    """They hold URLs: a backup must not outlive the history it copies."""
    urls = _open_three(b)
    _crash(b)
    b.start()
    require(lambda: b.wait_tab_urls(urls), 20, f"did not restore: {b.tab_urls()}")
    require(lambda: _backups(b), 10, "no backup to delete")
    settings = "chrome://settings/clearBrowserData"
    tab = b.find_tab(urls[0])
    b.cdp(tab, "Page.navigate", {"url": settings})
    require(lambda: settings in b.tab_urls(), 10, f"no settings page: {b.tab_urls()}")
    tab = b.find_tab("chrome://settings")
    # What the dialog does. Its handler takes messages once told to start. 4 is all time.
    b.eval("chrome.send('initializeClearBrowsingData', ['cb0'])", tab)
    b.eval("chrome.send('clearBrowsingData', ['cb1', ['browser.clear_data.browsing_history'], 4])",
           tab)
    expect(lambda: not _backups(b), 15, f"backups remain: {_backups(b)}")
