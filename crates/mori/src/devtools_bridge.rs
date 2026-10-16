//! Connects DevTools frontends to pages.
//!
//! Chromium's DevTools window cannot render offscreen, so Mori loads the
//! DevTools frontend (`devtools://devtools/bundled/devtools_app.html`) in an
//! ordinary view and points it at a WebSocket served here on 127.0.0.1.
//! Each connection is relayed to the inspected page's protocol session
//! (`WebView::send_devtools_message` / `set_devtools_sink`).
//!
//! Only frontends Mori opened can connect: every session has an
//! unguessable token in its URL path, connections must come from the
//! DevTools origin, and a token stops working when its pane closes.

use std::{
    collections::{HashMap, HashSet},
    io::ErrorKind,
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use futures::StreamExt as _;
use gpui_kit::*;
use mori_engine::{ViewId, WebView};
use tungstenite::{
    Message,
    handshake::server::{ErrorResponse, Request, Response},
};

const FRONTEND_ORIGIN: &str = "devtools://devtools";

enum BridgeEvent {
    Connected { token: String, outbound: mpsc::Sender<String> },
    Message { token: String, text: String },
    Closed { token: String },
}

struct Session {
    inspected: ViewId,
    outbound: Option<mpsc::Sender<String>>,
    /// An element to reveal once the frontend is connected.
    pending_inspect: Option<(i32, i32)>,
}

#[derive(Default)]
pub struct DevToolsBridge {
    port: Option<u16>,
    allowed: Arc<Mutex<HashSet<String>>>,
    sessions: HashMap<String, Session>,
}

impl Global for DevToolsBridge {}

impl DevToolsBridge {
    /// Start a session for `inspected` and return the frontend URL to load.
    pub fn open(cx: &mut App, inspected: ViewId) -> Option<String> {
        let port = Self::ensure_server(cx)?;
        let token = uuid::Uuid::new_v4().simple().to_string();
        let bridge = cx.global_mut::<DevToolsBridge>();
        if let Ok(mut allowed) = bridge.allowed.lock() {
            allowed.insert(token.clone());
        }
        bridge.sessions.insert(
            token.clone(),
            Session { inspected, outbound: None, pending_inspect: None },
        );
        Some(format!(
            "{FRONTEND_ORIGIN}/bundled/devtools_app.html?ws=127.0.0.1:{port}/{token}"
        ))
    }

    /// The token in a frontend URL from [`DevToolsBridge::open`].
    pub fn token_of(url: &str) -> Option<String> {
        url.rsplit('/').next().map(str::to_string).filter(|t| t.len() == 32)
    }

    /// End a session: the frontend is disconnected and the page's DevTools
    /// state (breakpoints, highlights, emulation) is reset.
    pub fn close(cx: &mut App, token: &str) {
        let bridge = cx.global_mut::<DevToolsBridge>();
        if let Ok(mut allowed) = bridge.allowed.lock() {
            allowed.remove(token);
        }
        if let Some(session) = bridge.sessions.remove(token) {
            // Dropping the sender makes the connection thread hang up.
            drop(session.outbound);
            if let Some(view) = WebView::by_id(session.inspected) {
                detach(&view);
            }
        }
    }

    /// Reveal the element at `(x, y)` (page coordinates) in the session's
    /// Elements panel, now or once the frontend has connected.
    pub fn inspect(cx: &mut App, token: &str, x: i32, y: i32) {
        let bridge = cx.global_mut::<DevToolsBridge>();
        let Some(session) = bridge.sessions.get_mut(token) else { return };
        match session.outbound.clone() {
            Some(outbound) => reveal(session.inspected, outbound, x, y, 0, cx),
            None => session.pending_inspect = Some((x, y)),
        }
    }

    fn ensure_server(cx: &mut App) -> Option<u16> {
        if let Some(port) = cx.default_global::<DevToolsBridge>().port {
            return Some(port);
        }
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .inspect_err(|e| log::error!("DevTools bridge: cannot listen: {e}"))
            .ok()?;
        let port = listener.local_addr().ok()?.port();
        let allowed = cx.global::<DevToolsBridge>().allowed.clone();
        let (events_tx, mut events_rx) = futures::channel::mpsc::unbounded::<BridgeEvent>();
        std::thread::Builder::new()
            .name("mori-devtools-bridge".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let allowed = allowed.clone();
                    let events = events_tx.clone();
                    let _ = std::thread::Builder::new()
                        .name("mori-devtools-conn".into())
                        .spawn(move || serve(stream, &allowed, &events));
                }
            })
            .ok()?;
        cx.spawn(async move |cx| {
            while let Some(event) = events_rx.next().await {
                cx.update(|cx| handle(event, cx));
            }
        })
        .detach();
        cx.global_mut::<DevToolsBridge>().port = Some(port);
        Some(port)
    }
}

fn handle(event: BridgeEvent, cx: &mut App) {
    match event {
        BridgeEvent::Connected { token, outbound } => {
            let bridge = cx.global_mut::<DevToolsBridge>();
            let Some(session) = bridge.sessions.get_mut(&token) else { return };
            let Some(view) = WebView::by_id(session.inspected) else { return };
            session.outbound = Some(outbound.clone());
            let inspected = session.inspected;
            let pending = session.pending_inspect.take();
            let sink = outbound.clone();
            view.set_devtools_sink(Some(Box::new(move |message: &[u8]| {
                let _ = sink.send(String::from_utf8_lossy(message).into_owned());
            })));
            if let Some((x, y)) = pending {
                reveal(inspected, outbound, x, y, 0, cx);
            }
        }
        BridgeEvent::Message { token, text } => {
            let bridge = cx.global::<DevToolsBridge>();
            if let Some(view) = bridge.sessions.get(&token).and_then(|s| WebView::by_id(s.inspected)) {
                view.send_devtools_message(text.as_bytes());
            }
        }
        BridgeEvent::Closed { token } => {
            let bridge = cx.global_mut::<DevToolsBridge>();
            if let Some(session) = bridge.sessions.get_mut(&token) {
                session.outbound = None;
                if let Some(view) = WebView::by_id(session.inspected) {
                    detach(&view);
                }
            }
        }
    }
}

/// Stop relaying to a frontend and undo what it left switched on in the
/// page: a paused debugger would freeze it, emulation would distort it.
fn detach(view: &WebView) {
    view.set_devtools_sink(None);
    for method in [
        "Debugger.disable",
        "Overlay.hideHighlight",
        "Overlay.disable",
        "Emulation.clearDeviceMetricsOverride",
        "Emulation.setTouchEmulationEnabled",
        "Emulation.setEmulatedMedia",
        "DOM.disable",
        "CSS.disable",
        "Network.disable",
        "Profiler.disable",
        "Log.disable",
    ] {
        let params = match method {
            "Emulation.setTouchEmulationEnabled" => serde_json::json!({ "enabled": false }),
            "Emulation.setEmulatedMedia" => serde_json::json!({ "media": "", "features": [] }),
            _ => serde_json::json!({}),
        };
        view.devtools_call(method, params, |_| {});
    }
}

/// Ask the page which node is at `(x, y)` and tell the frontend to select
/// it. The frontend enables the DOM domain shortly after connecting, so an
/// early attempt is retried a few times.
fn reveal(inspected: ViewId, outbound: mpsc::Sender<String>, x: i32, y: i32, attempt: u32, cx: &mut App) {
    let Some(view) = WebView::by_id(inspected) else { return };
    let params = serde_json::json!({ "x": x, "y": y, "includeUserAgentShadowDOM": false });
    let async_cx = cx.to_async();
    view.devtools_call("DOM.getNodeForLocation", params, move |result| {
        match result.ok().and_then(|r| r.get("backendNodeId").and_then(|id| id.as_i64())) {
            Some(node) => {
                let event = serde_json::json!({
                    "method": "Overlay.inspectNodeRequested",
                    "params": { "backendNodeId": node },
                });
                let _ = outbound.send(event.to_string());
            }
            None if attempt < 10 => {
                async_cx
                    .spawn(async move |cx| {
                        cx.background_executor().timer(Duration::from_millis(300)).await;
                        cx.update(|cx| reveal(inspected, outbound, x, y, attempt + 1, cx));
                    })
                    .detach();
            }
            None => {}
        }
    });
}

/// One frontend connection: relay frames both ways until either side goes.
fn serve(stream: TcpStream, allowed: &Mutex<HashSet<String>>, events: &futures::channel::mpsc::UnboundedSender<BridgeEvent>) {
    let mut token = String::new();
    // The callback's signature is tungstenite's.
    #[allow(clippy::result_large_err)]
    let check = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        let origin_ok = request
            .headers()
            .get("Origin")
            .is_none_or(|o| o.as_bytes() == FRONTEND_ORIGIN.as_bytes());
        let path = request.uri().path().trim_start_matches('/').to_string();
        let known = allowed.lock().is_ok_and(|a| a.contains(&path));
        if origin_ok && known {
            token = path;
            Ok(response)
        } else {
            let mut reject = ErrorResponse::new(None);
            *reject.status_mut() = tungstenite::http::StatusCode::FORBIDDEN;
            Err(reject)
        }
    };
    let Ok(mut socket) = tungstenite::accept_hdr(stream, check) else { return };
    let _ = socket.get_ref().set_read_timeout(Some(Duration::from_millis(10)));
    let (outbound_tx, outbound_rx) = mpsc::channel::<String>();
    if events
        .unbounded_send(BridgeEvent::Connected { token: token.clone(), outbound: outbound_tx })
        .is_err()
    {
        return;
    }
    'relay: loop {
        loop {
            match outbound_rx.try_recv() {
                Ok(text) => {
                    if socket.send(Message::text(text)).is_err() {
                        break 'relay;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    let _ = socket.flush();
                    break 'relay;
                }
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let _ = events.unbounded_send(BridgeEvent::Message { token: token.clone(), text: text.to_string() });
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                let _ = socket.flush();
            }
            Err(_) => break,
        }
    }
    let _ = events.unbounded_send(BridgeEvent::Closed { token });
}

#[cfg(test)]
mod tests {
    use super::DevToolsBridge;

    #[test]
    fn token_from_frontend_url() {
        let url = "devtools://devtools/bundled/devtools_app.html?ws=127.0.0.1:4242/0123456789abcdef0123456789abcdef";
        assert_eq!(DevToolsBridge::token_of(url).as_deref(), Some("0123456789abcdef0123456789abcdef"));
        assert_eq!(DevToolsBridge::token_of("https://example.com/"), None);
    }
}
