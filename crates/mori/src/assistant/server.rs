//! The local Codex app server: finding the `codex` binary, running
//! `codex app-server --listen ws://127.0.0.1:<port>`, and talking to it.
//!
//! Each connection gets one dedicated OS thread. It spawns the server, waits
//! for `GET /readyz`, opens the WebSocket and then pumps frames both ways, so
//! no network or process IO ever runs on the GPUI main thread. The main
//! thread holds a [`CodexClient`] (requests in, oneshot replies out) and
//! drains [`ServerEvent`]s from an unbounded channel.
//!
//! The child process is terminated when the [`CodexClient`] (and with it the
//! [`ServerProcess`]) is dropped: SIGTERM to its process group, then SIGKILL
//! after two seconds.

use std::{
    collections::{HashMap, VecDeque},
    ffi::{OsStr, OsString},
    fmt,
    hash::{BuildHasher, Hasher},
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    rc::Rc,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use futures::channel::{mpsc as async_mpsc, oneshot};
use serde_json::Value;
use tungstenite::{Message as WsMessage, WebSocket};

use super::protocol::{self, Incoming, RpcError};

/// Ports the app server is started on (one is picked at random).
pub const PORT_RANGE: std::ops::RangeInclusive<u16> = 42200..=42999;
/// How long to wait for `/readyz` after spawning.
const READY_TIMEOUT: Duration = Duration::from_secs(20);
const READY_POLL: Duration = Duration::from_millis(250);
/// Read timeout of the socket; also the latency of outbound frames.
const IO_POLL: Duration = Duration::from_millis(15);
/// Grace period between SIGTERM and SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// Lines of server stderr kept for error messages.
const STDERR_TAIL: usize = 12;

// ---- Locating codex -----------------------------------------------------------

/// Where to look for `codex`, in order: `$CODEX_BIN`, every `$PATH` entry,
/// then the usual per-user install locations (npm/bun/mise), which a
/// desktop-launched Mori may not have on its `PATH`.
pub fn codex_candidates(
    codex_bin: Option<&OsStr>,
    path_var: Option<&OsStr>,
    home: Option<&Path>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(bin) = codex_bin.filter(|b| !b.is_empty()) {
        out.push(PathBuf::from(bin));
    }
    if let Some(path) = path_var {
        out.extend(
            std::env::split_paths(path)
                .filter(|d| !d.as_os_str().is_empty())
                .map(|d| d.join("codex")),
        );
    }
    if let Some(home) = home {
        for dir in [".local/bin", ".bun/bin", ".npm-global/bin"] {
            out.push(home.join(dir).join("codex"));
        }
    }
    out.push(PathBuf::from("/usr/local/bin/codex"));
    if let Some(home) = home {
        out.push(home.join(".local/share/mise/shims/codex"));
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

/// The first executable `codex` from [`codex_candidates`].
pub fn find_codex() -> Option<PathBuf> {
    codex_candidates(
        std::env::var_os("CODEX_BIN").as_deref(),
        std::env::var_os("PATH").as_deref(),
        dirs::home_dir().as_deref(),
    )
    .into_iter()
    .find(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `PATH` for the child: the codex binary's own directory and common
/// Node/codex install dirs first (the npm build is a `#!/usr/bin/env node`
/// script), then the inherited entries, de-duplicated.
pub fn spawn_path(codex: &Path, home: Option<&Path>, inherited: Option<&OsStr>) -> OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(parent) = codex.parent() {
        dirs.push(parent.to_path_buf());
    }
    if let Some(home) = home {
        for dir in [
            ".local/bin",
            ".bun/bin",
            ".npm-global/bin",
            ".volta/bin",
            ".local/share/mise/shims",
        ] {
            dirs.push(home.join(dir));
        }
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    if let Some(inherited) = inherited {
        dirs.extend(std::env::split_paths(inherited));
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
    std::env::join_paths(dirs)
        .unwrap_or_else(|_| inherited.map(OsStr::to_os_string).unwrap_or_default())
}

// ---- Errors -------------------------------------------------------------------

/// Why a connection to the app server could not be established.
#[derive(Debug)]
pub enum ConnectError {
    /// No `codex` binary was found.
    BinaryMissing,
    /// The binary could not be started.
    Spawn(String),
    /// The server exited before becoming ready.
    Exited(String),
    /// `/readyz` never answered.
    NotReady,
    /// The WebSocket handshake failed.
    WebSocket(String),
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectError::BinaryMissing => write!(
                f,
                "Mori's assistant runs on the local Codex CLI, which isn't installed. \
                 Install it with `npm i -g @openai/codex` (or `mise use -g codex`), \
                 sign in with `codex login`, then try again. \
                 To use a specific binary, set CODEX_BIN."
            ),
            ConnectError::Spawn(e) => write!(f, "Couldn't start the Codex app server: {e}"),
            ConnectError::Exited(tail) if tail.is_empty() => {
                write!(f, "The Codex app server exited while starting.")
            }
            ConnectError::Exited(tail) => {
                write!(f, "The Codex app server exited while starting:\n{tail}")
            }
            ConnectError::NotReady => {
                write!(
                    f,
                    "Timed out waiting for the local Codex app server to start."
                )
            }
            ConnectError::WebSocket(e) => {
                write!(f, "Couldn't connect to the local Codex app server: {e}")
            }
        }
    }
}

// ---- Process ------------------------------------------------------------------

/// The running `codex app-server` child. Dropping it terminates the server.
pub struct ServerProcess {
    child: Option<Child>,
    port: u16,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
}

impl ServerProcess {
    fn spawn(binary: &Path, port: u16, cwd: &Path) -> Result<ServerProcess, ConnectError> {
        let home = dirs::home_dir();
        let mut command = Command::new(binary);
        command
            .arg("app-server")
            .arg("--listen")
            .arg(format!("ws://127.0.0.1:{port}"))
            .env(
                "PATH",
                spawn_path(binary, home.as_deref(), std::env::var_os("PATH").as_deref()),
            )
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        {
            use std::os::unix::process::CommandExt;
            // Own process group, so wrapper scripts and their children are
            // signalled together.
            command.process_group(0);
            #[cfg(target_os = "linux")]
            // SAFETY: prctl is async-signal-safe and touches no Rust state.
            unsafe {
                command.pre_exec(|| {
                    // If Mori dies without running destructors, the server
                    // must not outlive it.
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
        }
        let mut child = command
            .spawn()
            .map_err(|e| ConnectError::Spawn(e.to_string()))?;

        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(stderr) = child.stderr.take() {
            let tail = stderr_tail.clone();
            thread::Builder::new()
                .name("codex-stderr".into())
                .spawn(move || {
                    for line in BufReader::new(stderr).lines() {
                        let Ok(line) = line else { break };
                        log::debug!("codex app-server: {line}");
                        if let Ok(mut tail) = tail.lock() {
                            if tail.len() == STDERR_TAIL {
                                tail.pop_front();
                            }
                            tail.push_back(line);
                        }
                    }
                })
                .ok();
        }
        Ok(ServerProcess {
            child: Some(child),
            port,
            stderr_tail,
        })
    }

    fn has_exited(&mut self) -> bool {
        self.child
            .as_mut()
            .is_none_or(|c| matches!(c.try_wait(), Ok(Some(_)) | Err(_)))
    }

    fn stderr_tail(&self) -> String {
        self.stderr_tail
            .lock()
            .map(|t| t.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }

    /// Poll `GET /readyz` until it answers 2xx, the child exits, or the
    /// timeout passes.
    fn wait_ready(&mut self) -> Result<(), ConnectError> {
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            if self.has_exited() {
                // Give the stderr thread a moment to collect the reason.
                thread::sleep(Duration::from_millis(100));
                return Err(ConnectError::Exited(self.stderr_tail()));
            }
            if readyz(self.port) {
                return Ok(());
            }
            thread::sleep(READY_POLL);
        }
        Err(ConnectError::NotReady)
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            terminate(child);
        }
    }
}

/// SIGTERM the child's process group, then SIGKILL it if it is still alive
/// after [`KILL_GRACE`]. Never blocks the caller.
fn terminate(mut child: Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let Ok(pid) = i32::try_from(child.id()) else {
        let _ = child.kill();
        return;
    };
    // SAFETY: plain syscall; `-pid` addresses the group created by
    // `process_group(0)`, whose id equals the child's pid.
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    let reaper = thread::Builder::new()
        .name("codex-reaper".into())
        .spawn(move || {
            let deadline = Instant::now() + KILL_GRACE;
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
            // SAFETY: as above.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
        });
    if reaper.is_err() {
        // SAFETY: as above.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

/// One `GET /readyz` over a raw TCP socket.
fn readyz(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let request =
        format!("GET /readyz HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut head = [0u8; 64];
    let mut len = 0;
    while len < head.len() {
        match stream.read(&mut head[len..]) {
            Ok(0) => break,
            Ok(n) => {
                len += n;
                if head[..len].contains(&b'\n') {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    is_success_status_line(&head[..len])
}

/// Whether an HTTP response starts with a 2xx status line.
pub fn is_success_status_line(head: &[u8]) -> bool {
    let line = String::from_utf8_lossy(head);
    let mut parts = line.split_whitespace();
    parts.next().is_some_and(|v| v.starts_with("HTTP/"))
        && parts
            .next()
            .is_some_and(|code| code.len() == 3 && code.starts_with('2'))
}

/// A pseudo-random port in [`PORT_RANGE`] that is currently free.
fn pick_port() -> u16 {
    let span = u64::from(PORT_RANGE.end() - PORT_RANGE.start()) + 1;
    let mut fallback = *PORT_RANGE.start();
    for _ in 0..16 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(Instant::now().elapsed().as_nanos());
        let port = PORT_RANGE.start() + (hasher.finish() % span) as u16;
        fallback = port;
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    fallback
}

// ---- Connection ---------------------------------------------------------------

/// What the server sent that the session must handle.
#[derive(Debug)]
pub enum ServerEvent {
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// The connection is gone; no further events follow.
    Closed(String),
}

enum Outbound {
    Request {
        id: i64,
        frame: String,
        reply: oneshot::Sender<Result<Value, RpcError>>,
    },
    Frame(String),
}

/// What a successful [`connect`] hands back to the main thread.
pub struct Connected {
    pub client: CodexClient,
    pub events: async_mpsc::UnboundedReceiver<ServerEvent>,
}

struct Parts {
    out: mpsc::Sender<Outbound>,
    process: ServerProcess,
    events: async_mpsc::UnboundedReceiver<ServerEvent>,
}

/// Start an app server and connect to it on a new thread. `cwd` is the
/// server's working directory.
pub async fn connect(cwd: PathBuf) -> Result<Connected, ConnectError> {
    let (tx, rx) = oneshot::channel();
    thread::Builder::new()
        .name("codex-app-server".into())
        .spawn(move || run_connection(cwd, tx))
        .map_err(|e| ConnectError::Spawn(e.to_string()))?;
    let parts = rx
        .await
        .map_err(|_| ConnectError::Spawn("the connection thread stopped".into()))??;
    Ok(Connected {
        client: CodexClient {
            inner: Rc::new(ClientInner {
                out: parts.out,
                next_id: std::cell::Cell::new(1),
                _process: parts.process,
            }),
        },
        events: parts.events,
    })
}

fn run_connection(cwd: PathBuf, reply: oneshot::Sender<Result<Parts, ConnectError>>) {
    let Some(binary) = find_codex() else {
        let _ = reply.send(Err(ConnectError::BinaryMissing));
        return;
    };
    log::info!("starting Codex app server: {}", binary.display());

    // A port race with another process shows up as an early exit; retry.
    let mut last_error = ConnectError::NotReady;
    let mut ready = None;
    for _ in 0..3 {
        let port = pick_port();
        let mut process = match ServerProcess::spawn(&binary, port, &cwd) {
            Ok(p) => p,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        };
        match process.wait_ready() {
            Ok(()) => {
                ready = Some(process);
                break;
            }
            Err(e @ ConnectError::Exited(_)) => last_error = e,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        }
        if reply.is_canceled() {
            return;
        }
    }
    let Some(process) = ready else {
        let _ = reply.send(Err(last_error));
        return;
    };

    let socket = match open_socket(process.port) {
        Ok(s) => s,
        Err(e) => {
            let _ = reply.send(Err(e));
            return;
        }
    };
    let (out_tx, out_rx) = mpsc::channel();
    let (event_tx, event_rx) = async_mpsc::unbounded();
    if reply
        .send(Ok(Parts {
            out: out_tx,
            process,
            events: event_rx,
        }))
        .is_err()
    {
        // Nobody is waiting any more; dropping the parts stops the server.
        return;
    }
    let reason = pump(socket, out_rx, &event_tx);
    log::info!("Codex app server connection closed: {reason}");
    let _ = event_tx.unbounded_send(ServerEvent::Closed(reason));
}

fn open_socket(port: u16) -> Result<WebSocket<TcpStream>, ConnectError> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| ConnectError::WebSocket(e.to_string()))?;
    let _ = stream.set_nodelay(true);
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| ConnectError::WebSocket(e.to_string()))?;
    let (socket, _) = tungstenite::client(format!("ws://127.0.0.1:{port}/"), stream)
        .map_err(|e| ConnectError::WebSocket(e.to_string()))?;
    socket
        .get_ref()
        .set_read_timeout(Some(IO_POLL))
        .map_err(|e| ConnectError::WebSocket(e.to_string()))?;
    Ok(socket)
}

/// Shuttle frames until either side goes away. Returns why it stopped.
fn pump(
    mut socket: WebSocket<TcpStream>,
    out_rx: mpsc::Receiver<Outbound>,
    events: &async_mpsc::UnboundedSender<ServerEvent>,
) -> String {
    let mut pending: HashMap<i64, oneshot::Sender<Result<Value, RpcError>>> = HashMap::new();
    let mut last_prune = Instant::now();
    loop {
        // Outbound first, so requests go out within one poll interval.
        loop {
            let frame = match out_rx.try_recv() {
                Ok(Outbound::Request { id, frame, reply }) => {
                    pending.insert(id, reply);
                    frame
                }
                Ok(Outbound::Frame(frame)) => frame,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    let _ = socket.flush();
                    return "client dropped".into();
                }
            };
            if let Err(e) = socket.send(WsMessage::text(frame)) {
                return format!("send failed: {e}");
            }
        }

        let text = match socket.read() {
            Ok(WsMessage::Text(text)) => text.to_string(),
            Ok(WsMessage::Binary(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            Ok(WsMessage::Close(_)) => return "server closed the connection".into(),
            Ok(_) => continue,
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // Idle: flush queued control frames (pongs) and tidy up.
                let _ = socket.flush();
                if last_prune.elapsed() > Duration::from_secs(10) {
                    pending.retain(|_, reply| !reply.is_canceled());
                    last_prune = Instant::now();
                }
                continue;
            }
            Err(e) => return e.to_string(),
        };

        let delivered = match protocol::parse_frame(&text) {
            Some(Incoming::Response { id, result }) => {
                if let Some(reply) = pending.remove(&id) {
                    let _ = reply.send(result);
                }
                true
            }
            Some(Incoming::Notification { method, params }) => events
                .unbounded_send(ServerEvent::Notification { method, params })
                .is_ok(),
            Some(Incoming::Request { id, method, params }) => events
                .unbounded_send(ServerEvent::Request { id, method, params })
                .is_ok(),
            None => {
                log::debug!("ignoring unparseable app-server frame");
                true
            }
        };
        if !delivered {
            let _ = socket.close(None);
            let _ = socket.flush();
            return "session dropped".into();
        }
    }
}

// ---- Client -------------------------------------------------------------------

struct ClientInner {
    out: mpsc::Sender<Outbound>,
    next_id: std::cell::Cell<i64>,
    /// Kept alive as long as any client handle exists.
    _process: ServerProcess,
}

/// Main-thread handle to a connected app server. Cheap to clone; the server
/// is stopped when the last clone is dropped.
#[derive(Clone)]
pub struct CodexClient {
    inner: Rc<ClientInner>,
}

/// The reply to [`CodexClient::request`]; resolves to `Err(Canceled)` if the
/// connection closes first.
pub type Reply = oneshot::Receiver<Result<Value, RpcError>>;

impl CodexClient {
    /// Send a request; the returned receiver yields the server's answer.
    pub fn request(&self, method: &str, params: Value) -> Reply {
        let id = self.inner.next_id.get();
        self.inner.next_id.set(id + 1);
        let (reply, rx) = oneshot::channel();
        let frame = protocol::request_frame(id, method, params);
        // A send error drops `reply`, which resolves `rx` as canceled.
        let _ = self.inner.out.send(Outbound::Request { id, frame, reply });
        rx
    }

    /// Send a client notification.
    pub fn notify(&self, method: &str, params: Value) {
        let _ = self
            .inner
            .out
            .send(Outbound::Frame(protocol::notification_frame(
                method, params,
            )));
    }

    /// Answer a server request.
    pub fn respond(&self, id: &Value, result: Value) {
        let _ = self
            .inner
            .out
            .send(Outbound::Frame(protocol::response_frame(id, result)));
    }

    /// Reject a server request.
    pub fn respond_error(&self, id: &Value, code: i64, message: &str) {
        let _ = self
            .inner
            .out
            .send(Outbound::Frame(protocol::error_frame(id, code, message)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_order() {
        let home = Path::new("/home/u");
        let found = codex_candidates(
            Some(OsStr::new("/opt/custom/codex")),
            Some(OsStr::new("/usr/bin::/home/u/.local/bin")),
            Some(home),
        );
        let found: Vec<_> = found
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            found,
            [
                "/opt/custom/codex",
                "/usr/bin/codex",
                "/home/u/.local/bin/codex",
                "/home/u/.bun/bin/codex",
                "/home/u/.npm-global/bin/codex",
                "/usr/local/bin/codex",
                "/home/u/.local/share/mise/shims/codex",
            ]
        );
    }

    #[test]
    fn candidates_without_env() {
        let found = codex_candidates(Some(OsStr::new("")), None, None);
        assert_eq!(found, [PathBuf::from("/usr/local/bin/codex")]);
    }

    #[test]
    fn spawn_path_prepends_and_dedupes() {
        let path = spawn_path(
            Path::new("/home/u/.local/share/mise/installs/codex/latest/bin/codex"),
            Some(Path::new("/home/u")),
            Some(OsStr::new("/usr/bin:/home/u/.bun/bin:/usr/bin")),
        );
        let parts: Vec<_> = std::env::split_paths(&path)
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            parts[0],
            "/home/u/.local/share/mise/installs/codex/latest/bin"
        );
        assert_eq!(parts[1], "/home/u/.local/bin");
        assert_eq!(parts.iter().filter(|p| *p == "/usr/bin").count(), 1);
        assert_eq!(parts.iter().filter(|p| *p == "/home/u/.bun/bin").count(), 1);
        assert_eq!(parts.last().map(String::as_str), Some("/usr/bin"));
    }

    #[test]
    fn status_lines() {
        assert!(is_success_status_line(b"HTTP/1.1 200 OK\r\n"));
        assert!(is_success_status_line(b"HTTP/1.0 204 No Content"));
        assert!(!is_success_status_line(b"HTTP/1.1 503 Service Unavailable"));
        assert!(!is_success_status_line(b"garbage"));
        assert!(!is_success_status_line(b""));
    }

    #[test]
    fn picked_port_is_in_range() {
        assert!(PORT_RANGE.contains(&pick_port()));
    }

    /// Manual protocol check against the real `codex` binary:
    /// `cargo test -p mori --bin mori smoke -- --ignored --nocapture`.
    /// Set `MORI_SMOKE_TURN=1` to also run one tiny "say hi" turn.
    #[test]
    #[ignore = "spawns the real codex app-server"]
    fn smoke_real_app_server() {
        use futures::StreamExt as _;
        use serde_json::json;

        futures::executor::block_on(async {
            let dir = std::env::temp_dir().join(format!("mori-codex-smoke-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let Connected { client, mut events } = connect(dir.clone()).await.expect("connect");
            let init = client
                .request(
                    "initialize",
                    json!({
                        "clientInfo": { "name": "Mori", "version": "0" },
                        "capabilities": { "experimentalApi": true, "requestAttestation": false },
                    }),
                )
                .await
                .expect("reply")
                .expect("initialize");
            eprintln!("initialize -> {init}");
            client.notify("initialized", json!({}));

            let models = client
                .request("model/list", json!({}))
                .await
                .expect("reply")
                .expect("model/list");
            let names: Vec<_> = models["data"]
                .as_array()
                .map(|d| {
                    d.iter()
                        .filter_map(|m| m["model"].as_str().or(m["id"].as_str()))
                        .collect()
                })
                .unwrap_or_default();
            eprintln!("models: {names:?}");

            let thread = client
                .request(
                    "thread/start",
                    json!({
                        "cwd": dir.to_string_lossy(),
                        "approvalPolicy": "on-request",
                        "sandbox": "workspace-write",
                        "personality": "friendly",
                        "serviceName": "mori_browser",
                        "dynamicTools": super::super::tools::dynamic_tools(),
                    }),
                )
                .await
                .expect("reply")
                .expect("thread/start");
            let thread_id = thread["thread"]["id"]
                .as_str()
                .expect("thread id")
                .to_string();
            eprintln!("thread/start -> {thread_id} (model {})", thread["model"]);

            if std::env::var("MORI_SMOKE_TURN").as_deref() == Ok("1") {
                let turn = client
                    .request(
                        "turn/start",
                        json!({
                            "threadId": thread_id,
                            "input": [{ "type": "text", "text": "Say hi in two words. Do not call any tools." }],
                            "approvalPolicy": "on-request",
                        }),
                    )
                    .await
                    .expect("reply")
                    .expect("turn/start");
                eprintln!("turn/start -> {}", turn["turn"]["id"]);
                let mut text = String::new();
                while let Some(event) = events.next().await {
                    match event {
                        ServerEvent::Notification { method, params } => {
                            if method == "item/agentMessage/delta" {
                                text.push_str(params["delta"].as_str().unwrap_or_default());
                            } else {
                                eprintln!(
                                    "<- {method} {}",
                                    super::super::tools::clip(&params.to_string(), 300)
                                );
                            }
                            if method == "turn/completed" || method == "error" {
                                break;
                            }
                        }
                        ServerEvent::Request { id, method, params } => {
                            eprintln!("<- request {method} {params}");
                            client.respond_error(&id, -32601, "smoke test");
                        }
                        ServerEvent::Closed(reason) => panic!("closed: {reason}"),
                    }
                }
                eprintln!("agent said: {text:?}");
                assert!(!text.trim().is_empty());
            }
            drop(client);
            let _ = std::fs::remove_dir(&dir);
        });
    }
}
