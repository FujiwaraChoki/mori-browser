"""Does the browser start, draw, and take real input?"""
from driver import DriverError, Image, close_color, expect, require, tags

PINK = (0xff, 0x00, 0xaa)
BLUE = (0x00, 0xaa, 0xff)


def _ready(b, *names):
    """Start with a tab per colour name, and wait for them to load."""
    colors = {"alpha": "ff00aa", "charlie": "00aaff", "delta": "33cc33"}
    urls = [b.fixture.page(name, colors[name]) for name in names]
    b.start(*urls)
    require(lambda: b.wait_tab_urls(urls), 20, "the opened tabs did not appear")
    return urls


def _trusted(events, kind):
    return [e for e in events if e["type"] == kind and e["trusted"]]


@tags("smoke")
def test_launch_draws_page_and_sidebar(b):
    _ready(b, "alpha")
    assert len(b.windows()) == 1, f"expected one window: {b.windows()}"
    assert b.windows()[0]["rect"]["width"] == 1440
    assert b.find_tab("alpha")["title"] == "alpha"

    rect = b.page_rect()
    middle = (rect[0] + 100, rect[1] + 200, rect[2] - 200, rect[3] - 300)
    assert b.wait_color(middle, PINK), f"page not drawn: {b.grab().mean(middle)}"
    # Native Views chrome: a sidebar left of the page, with something drawn on it.
    assert rect[0] > 100, f"page starts at {rect[0]}: no room for a sidebar"
    sidebar = b.grab().crop((0, 0, rect[0] - 2, rect[3]))
    assert not close_color(sidebar.mean(), PINK, 40), "the page covers the sidebar"
    corner = sidebar.pixel(sidebar.width - 1, sidebar.height // 2)
    flat = Image(sidebar.width, sidebar.height, bytes(corner) * (sidebar.width * sidebar.height))
    assert sidebar.diff_fraction(flat) > 0.005, "the sidebar is blank"


@tags("smoke")
def test_pointer_reaches_page(b):
    _ready(b, "alpha")
    x, y = b.page_origin()

    b.click(x + 300, y + 300)
    click = expect(lambda: _trusted(b.eval("events"), "click"), 5, "click did not arrive")[-1]
    assert (click["x"], click["y"]) == (300, 300), click

    b.double_click(x + 320, y + 320)
    expect(lambda: _trusted(b.eval("events"), "dblclick"), 5, "double click did not arrive")

    b.drag(x + 400, y + 400, x + 500, y + 450)

    def drag_ended():
        events = b.eval("events")
        ups = _trusted(events, "mouseup")
        return ups and ups[-1]["x"] == 500 and (_trusted(events, "mousedown")[-1], ups[-1])
    down, up = expect(drag_ended, 5, "drag did not end at its target")
    assert (down["x"], down["y"]) == (400, 400), down
    assert (up["x"], up["y"]) == (500, 450), up


@tags("smoke")
def test_right_click_menu_opens_and_closes(b):
    _ready(b, "alpha")
    x, y = b.page_origin()
    probe = (x + 500, y + 300, 120, 120)   # Where the menu opens: down and right.
    assert b.wait_color(probe, PINK), "page not drawn"

    b.right_click(x + 500, y + 300)
    menu = expect(lambda: _trusted(b.eval("events"), "contextmenu"), 5, "no contextmenu event")[-1]
    assert (menu["x"], menu["y"]) == (500, 300), menu
    expect(lambda: not b.grab().is_color(PINK, probe, 3), 8, "no menu drawn over the page")

    b.keys("Escape")
    assert b.wait_color(probe, PINK, tolerance=3), "Escape did not close the menu"


@tags("smoke")
def test_scroll_wheel_scrolls_page(b):
    b.fixture.routes["/tall"] = (200, "text/html",
                                 "<!doctype html><title>tall</title><body style='height:5000px'>tall")
    url = b.fixture.url("/tall")
    b.start(url)
    require(lambda: b.wait_tab_urls([url]), 20, "tab did not appear")
    x, y = b.page_origin()
    b.scroll(x + 300, y + 300, 15)
    expect(lambda: b.eval("scrollY") > 0, 5, "wheel did not scroll the page")


@tags("smoke")
def test_keyboard_types_into_page(b):
    _ready(b, "alpha")
    x, y = b.page_origin()
    b.click(x + 50, y + 50)     # The fixture's input field.
    expect(lambda: b.eval("document.activeElement.id") == "field", 5, "the field did not take focus")
    b.type_text("Hello mori 42")
    expect(lambda: b.eval("document.getElementById('field').value") == "Hello mori 42", 5,
           "typed text did not arrive")
    keys = [e for e in b.eval("events") if e["type"] == "keydown"]
    assert keys and all(e["trusted"] for e in keys)


@tags("smoke")
def test_command_bar_opens_a_tab_and_ctrl_w_closes_it(b):
    urls = _ready(b, "alpha")
    typed = b.fixture.page("typed", "33cc33")
    base = b.grab()
    b.keys("ctrl+t")
    # The command bar is native UI: it can only be seen, not asked for.
    rect = b.page_rect()
    expect(lambda: b.grab().diff_fraction(base, rect) > 0.02, 5, "the command bar did not open")
    assert len(b.windows()) == 1, "the command bar mapped as its own window"

    b.type_text(typed)
    b.keys("Return")
    assert b.wait_tab_urls(urls + [typed], 10), f"typed URL did not open a tab: {b.tab_urls()}"
    assert b.active_tab()["title"] == "typed"
    assert b.wait_color((rect[0] + 100, rect[1] + 200, 300, 300), (0x33, 0xcc, 0x33)), "new tab not drawn"

    b.keys("ctrl+w")
    assert b.wait_tab_urls(urls, 10), f"Ctrl+W left: {b.tab_urls()}"


@tags("smoke")
def test_clicking_a_sidebar_row_switches_tab(b):
    _ready(b, "alpha", "charlie")
    assert b.active_tab()["title"] == "alpha"
    rect = b.page_rect()
    probe = (rect[0] + 100, rect[1] + 200, 300, 300)
    assert b.wait_color(probe, PINK), "first tab not drawn"

    # The selected row is the sidebar's brightest band; the next tab sits under it.
    shot = b.grab()
    column = rect[0] // 2
    bright = [y for y in range(shot.height) if min(shot.pixel(column, y)) > 245]
    if not bright:
        raise DriverError("no selected row found in the sidebar: has the theme changed?")
    row_height = bright[-1] - bright[0] + 1
    b.click(column, bright[-1] + row_height // 2 + 6)

    expect(lambda: b.active_tab()["title"] == "charlie", 5, "the click did not switch tab")
    assert b.wait_color(probe, BLUE), "the second tab's page was not drawn"
    after = b.grab()
    assert after.diff_fraction(shot, (0, bright[0], rect[0] - 2, row_height)) > 0.05, \
        "the sidebar selection did not move"


@tags("smoke")
def test_alt_number_selects_tab_in_strip_order(b):
    _ready(b, "alpha", "charlie", "delta")
    for combo, title in (("alt+3", "delta"), ("alt+1", "alpha"), ("alt+2", "charlie")):
        b.keys(combo)
        expect(lambda: b.active_tab()["title"] == title, 5, f"{combo} did not select {title}")
