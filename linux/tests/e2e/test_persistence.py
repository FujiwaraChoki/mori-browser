"""Do the user's tabs survive a restart?"""
from driver import expect, require, tags


def _open_three(b):
    urls = [b.fixture.page(name) for name in ("first", "second", "third")]
    b.start(*urls)
    require(lambda: b.wait_tab_urls(urls), 20, "the opened tabs did not appear")
    return urls


def _select(b, combo, title):
    b.keys(combo)
    require(lambda: b.active_tab()["title"] == title, 5, f"{combo} did not select {title}")


def _assert_tabs_return(b, urls):
    assert b.wait_tab_urls(urls, 20), f"tabs after restart: {b.tab_urls()}, wanted {sorted(urls)}"
    assert len(b.windows()) == 1, f"expected one window after restart: {len(b.windows())}"
    # Order, in the strip: Alt+N walks it. /json/list does not keep it.
    for number, url in enumerate(urls, 1):
        b.keys(f"alt+{number}")
        expect(lambda: b.active_tab()["url"] == url, 5, f"tab {number} is not {url}")


@tags("persistence", "smoke")
def test_tabs_return_after_clean_quit(b):
    urls = _open_three(b)
    _select(b, "alt+2", "second")
    b.wait_session_saved("first", "second", "third")
    b.quit_clean()
    b.start()
    assert b.wait_tab_urls(urls, 20), f"tabs after restart: {b.tab_urls()}"
    expect(lambda: b.active_tab()["url"] == urls[1], 5, "the active tab did not come back")
    _assert_tabs_return(b, urls)


def _open_late_tab(b, urls):
    """A tab opened through the UI, younger than the session's 2.5 s save delay."""
    late = b.fixture.page("late")
    b.wait_session_saved("first", "second", "third")
    b.keys("ctrl+t")
    b.type_text(late)
    b.keys("Return")
    require(lambda: b.wait_tab_urls(urls + [late]), 10, "the late tab did not open")
    return urls + [late]


@tags("persistence")
def test_tabs_return_after_sigterm(b):
    """SIGTERM is logout or shutdown."""
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    b.sigterm()
    b.start()
    _assert_tabs_return(b, urls)


@tags("persistence")
def test_tab_opened_just_before_clean_quit_returns(b):
    urls = _open_late_tab(b, _open_three(b))
    b.quit_clean()
    b.start()
    assert b.wait_tab_urls(urls, 20), f"tabs after quit: {b.tab_urls()}"


@tags("persistence")
def test_tab_opened_just_before_sigterm_returns(b):
    urls = _open_late_tab(b, _open_three(b))
    b.sigterm()
    b.start()
    require(lambda: b.tabs(), 20, "no tabs after SIGTERM")
    assert b.wait_tab_urls(urls, 8), f"tabs after SIGTERM: {b.tab_urls()}"


@tags("persistence")
def test_tabs_do_not_multiply_over_restarts(b):
    urls = _open_three(b)
    for _ in range(3):
        b.restart()
        assert b.wait_tab_urls(urls, 20), f"tabs after restart: {b.tab_urls()}"


@tags("persistence")
def test_closed_tab_stays_closed_after_restart(b):
    urls = _open_three(b)
    _select(b, "alt+3", "third")
    b.keys("ctrl+w")
    require(lambda: b.wait_tab_urls(urls[:2]), 10, "Ctrl+W did not close the tab")
    b.restart()
    assert b.wait_tab_urls(urls[:2], 20), f"tabs after restart: {b.tab_urls()}"


@tags("persistence", "crash")
def test_tabs_return_after_kill9(b):
    urls = _open_three(b)
    b.wait_session_saved("first", "second", "third")
    # Without these a kill looks like a clean exit, and the test passes for nothing.
    b.wait_marked_running()
    b.kill9()
    b.start()
    assert b.wait_tab_urls(urls, 15), f"tabs after kill -9: {b.tab_urls()}, wanted {sorted(urls)}"
