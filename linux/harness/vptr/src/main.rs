//! Test only. Moves and clicks a virtual pointer in a (headless) wlroots
//! compositor through zwlr_virtual_pointer_v1.
//!
//!   vptr move X Y          absolute, in output pixels of a WxH output
//!   vptr click X Y [right] move there and click
//!   vptr scroll X Y DY     move there and scroll by DY
//!
//! The output size comes from VPTR_SIZE ("2400x1500", physical pixels).

use std::{env, thread, time::Duration};

use wayland_client::{
    protocol::{wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<ZwlrVirtualPointerManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_seat" => state.seat = Some(registry.bind(name, version.min(7), qh, ())),
                "zwlr_virtual_pointer_manager_v1" => {
                    state.manager = Some(registry.bind(name, version.min(2), qh, ()))
                }
                _ => {}
            }
        }
    }
}

macro_rules! ignore {
    ($t:ty) => {
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    };
}
ignore!(wl_seat::WlSeat);
ignore!(ZwlrVirtualPointerManagerV1);
ignore!(ZwlrVirtualPointerV1);

fn now_ms() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() & 0xffff_ffff) as u32
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let (w, h) = env::var("VPTR_SIZE")
        .ok()
        .and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap())))
        .unwrap_or((2400u32, 1500u32));
    let conn = Connection::connect_to_env().expect("wayland");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state).unwrap();
    let manager = state.manager.as_ref().expect("no virtual pointer support");
    let pointer = manager.create_virtual_pointer(state.seat.as_ref(), &qh, ());
    let x: u32 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
    let y: u32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
    let flush = |queue: &mut wayland_client::EventQueue<State>, state: &mut State| {
        queue.roundtrip(state).unwrap();
    };
    // Glide in from a nearby point in small steps, as a real mouse does;
    // a single jump from a pointer that then disappears can arrive as just
    // enter/leave.
    let from_x = x.saturating_add(40).min(w - 1);
    for step in 0..=8u32 {
        let px = from_x - (from_x - x) * step / 8;
        pointer.motion_absolute(now_ms(), px, y, w, h);
        pointer.frame();
        flush(&mut queue, &mut state);
        thread::sleep(Duration::from_millis(16));
    }
    thread::sleep(Duration::from_millis(60));
    match args.get(1).map(String::as_str) {
        Some(kind @ ("click" | "dblclick")) => {
            let button = if args.get(4).map(String::as_str) == Some("right") { 0x111 } else { 0x110 };
            for i in 0..(if kind == "dblclick" { 2 } else { 1 }) {
                if i > 0 {
                    thread::sleep(Duration::from_millis(80));
                }
                pointer.button(now_ms(), button, wayland_client::protocol::wl_pointer::ButtonState::Pressed);
                pointer.frame();
                flush(&mut queue, &mut state);
                thread::sleep(Duration::from_millis(40));
                pointer.button(now_ms(), button, wayland_client::protocol::wl_pointer::ButtonState::Released);
                pointer.frame();
                flush(&mut queue, &mut state);
            }
        }
        // drag X Y X2 Y2: press at X Y, glide to X2 Y2, release.
        Some("drag") => {
            let x2: u32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(x);
            let y2: u32 = args.get(5).and_then(|v| v.parse().ok()).unwrap_or(y);
            let left = 0x110;
            pointer.button(now_ms(), left, wayland_client::protocol::wl_pointer::ButtonState::Pressed);
            pointer.frame();
            flush(&mut queue, &mut state);
            for step in 1..=20i64 {
                let px = x as i64 + (x2 as i64 - x as i64) * step / 20;
                let py = y as i64 + (y2 as i64 - y as i64) * step / 20;
                pointer.motion_absolute(now_ms(), px as u32, py as u32, w, h);
                pointer.frame();
                flush(&mut queue, &mut state);
                thread::sleep(Duration::from_millis(16));
            }
            thread::sleep(Duration::from_millis(60));
            pointer.button(now_ms(), left, wayland_client::protocol::wl_pointer::ButtonState::Released);
            pointer.frame();
            flush(&mut queue, &mut state);
        }
        Some("scroll") => {
            let dy: f64 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(0.0);
            pointer.axis(now_ms(), wayland_client::protocol::wl_pointer::Axis::VerticalScroll, dy);
            pointer.frame();
            flush(&mut queue, &mut state);
        }
        _ => {}
    }
    // Keep the pointer alive briefly so the compositor delivers the events.
    thread::sleep(Duration::from_millis(400));
    pointer.destroy();
    flush(&mut queue, &mut state);
}
