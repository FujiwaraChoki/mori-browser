//! The assistant conversation: a session talks to a local Codex app server
//! and exposes browser tools to it.
//!
//! [`AssistantSession`] is a GPUI entity owned by the assistant panel. On the
//! first message it starts `codex app-server` (see [`server`]), initializes
//! the connection and opens a thread whose `dynamicTools` are Mori's browser
//! tools (see [`tools`]). Agent text streams into the conversation as it
//! arrives; tool calls appear as [`Role::Tool`] lines and, unless they are
//! harmless, wait for the user's Allow/Deny via [`AssistantSession::pending_approval`].
//!
//! All IO happens on the connection thread; this entity only reacts to
//! events, so the main thread never blocks. Dropping the session stops the
//! app server.
//!
//! Environment switches: `MORI_ENABLE_CODEX_ASSISTANT=0` disables the
//! assistant, `MORI_CODEX_DYNAMIC_TOOLS=0` starts threads without tools.

mod protocol;
mod scripts;
mod server;
mod tools;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};

use futures::{
    FutureExt as _, StreamExt as _,
    channel::{mpsc::UnboundedReceiver, oneshot},
    future::{Either, select},
};
use gpui_kit::*;
use serde_json::{Value, json};

use self::{
    server::{CodexClient, ServerEvent},
    tools::{ToolCtx, ToolOutcome},
};
use crate::browser::Browser;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Tool,
    Error,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

impl Message {
    fn new(role: Role, text: impl Into<String>) -> Self {
        Message {
            role,
            text: text.into(),
        }
    }
}

const APPROVAL_POLICY: &str = "on-request";
const SANDBOX: &str = "workspace-write";
const SERVICE_NAME: &str = "mori_browser";
/// A turn with no server activity for this long is considered stalled. The
/// clock is paused while an approval card is up.
const TURN_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const WATCHDOG_TICK: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const TURN_START_TIMEOUT: Duration = Duration::from_secs(15);
const THREAD_START_TIMEOUT: Duration = Duration::from_secs(20);

const ENV_DISABLED: &str = "Mori's local Codex assistant is disabled. Relaunch without MORI_ENABLE_CODEX_ASSISTANT=0 to turn it back on.";
const SETTINGS_DISABLED: &str = "Mori's AI integration is turned off in Settings.";
const DISCONNECTED: &str = "The local Codex app server disconnected.";

fn env_enabled() -> bool {
    std::env::var("MORI_ENABLE_CODEX_ASSISTANT").map_or(true, |v| v.trim() != "0")
}

fn env_tools_enabled() -> bool {
    std::env::var("MORI_CODEX_DYNAMIC_TOOLS").map_or(true, |v| v.trim() != "0")
}

/// `<data dir>/assistant`, the app server's working directory (and the only
/// place its `workspace-write` sandbox may write to).
fn working_dir() -> PathBuf {
    let dir = mori_core::paths::data_dir().join("assistant");
    if let Err(err) = std::fs::create_dir_all(&dir) {
        log::warn!("could not create {}: {err}", dir.display());
    }
    dir
}

/// A tool call waiting for Allow/Deny.
struct PendingApproval {
    text: String,
    reply: oneshot::Sender<bool>,
}

struct Connection {
    client: CodexClient,
    events: Task<()>,
}

pub struct AssistantSession {
    browser: WeakEntity<Browser>,
    window: Option<AnyWindowHandle>,
    messages: Vec<Message>,
    busy: bool,
    status: Option<String>,
    approval: Option<PendingApproval>,

    connection: Option<Connection>,
    thread_id: Option<String>,
    uses_dynamic_tools: bool,
    /// Set once an app server rejected `dynamicTools`; later threads skip them.
    dynamic_tools_unsupported: bool,

    /// Bumped by send/interrupt/reset; async work started under an older
    /// value discards its result.
    generation: u64,
    stop_requested: bool,
    active_turn_id: Option<String>,
    ignored_turn_ids: HashSet<String>,
    /// Agent message item id → index of its bubble in `messages`.
    agent_items: HashMap<String, usize>,
    /// The bubble deltas without an item id go to.
    loose_assistant: Option<usize>,
    turn_has_text: bool,
    last_activity: Instant,
    watchdog_epoch: u64,
    turn_task: Option<Task<()>>,
}

impl AssistantSession {
    pub fn new(browser: WeakEntity<Browser>, cx: &mut Context<Self>) -> Self {
        // Created from the browser window's own update, so that window is
        // active; fall back to the first window.
        let window = cx.active_window().or_else(|| cx.windows().first().copied());
        AssistantSession {
            browser,
            window,
            messages: Vec::new(),
            busy: false,
            status: None,
            approval: None,
            connection: None,
            thread_id: None,
            uses_dynamic_tools: false,
            dynamic_tools_unsupported: false,
            generation: 0,
            stop_requested: false,
            active_turn_id: None,
            ignored_turn_ids: HashSet::new(),
            agent_items: HashMap::new(),
            loose_assistant: None,
            turn_has_text: false,
            last_activity: Instant::now(),
            watchdog_epoch: 0,
            turn_task: None,
        }
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// A turn is in progress (connecting, thinking, or running tools).
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// Progress text while busy ("Starting Codex…", "Thinking…").
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// The description of a tool call awaiting Allow/Deny.
    pub fn pending_approval(&self) -> Option<&str> {
        self.approval.as_ref().map(|a| a.text.as_str())
    }

    /// Send a user message, starting Codex and a thread if needed.
    ///
    /// This may be called from inside the browser's own update, so nothing
    /// here touches the `Browser` synchronously.
    pub fn send(&mut self, text: String, cx: &mut Context<Self>) {
        let text = text.trim().to_string();
        if text.is_empty() || self.busy {
            return;
        }
        self.messages.push(Message::new(Role::User, text.clone()));
        if !env_enabled() {
            self.messages.push(Message::new(Role::Error, ENV_DISABLED));
            cx.notify();
            return;
        }
        self.generation += 1;
        let generation = self.generation;
        self.stop_requested = false;
        self.active_turn_id = None;
        self.begin_turn_output();
        self.busy = true;
        self.status = Some("Starting Codex…".into());
        self.bump();
        cx.notify();

        self.turn_task = Some(cx.spawn(async move |this, cx| {
            if let Err(err) = Self::run_send(&this, generation, text, cx).await {
                this.update(cx, |s, cx| {
                    if s.generation == generation && s.busy {
                        s.fail_turn(err, cx);
                    }
                })
                .ok();
            }
        }));
    }

    /// Start a new conversation (a new Codex thread on the next message).
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            self.stop_active_turn();
        }
        self.generation += 1;
        self.turn_task = None;
        self.deny_pending_approval();
        self.messages.clear();
        self.thread_id = None;
        self.stop_requested = false;
        self.begin_turn_output();
        self.end_turn();
        cx.notify();
    }

    /// Stop the running turn.
    pub fn interrupt(&mut self, cx: &mut Context<Self>) {
        if !self.busy {
            return;
        }
        self.stop_requested = true;
        self.generation += 1;
        self.turn_task = None;
        self.stop_active_turn();
        self.deny_pending_approval();
        if !self.turn_has_text {
            self.messages
                .push(Message::new(Role::Assistant, "Stopped."));
        }
        self.end_turn();
        cx.notify();
    }

    /// Allow or deny the pending tool call.
    pub fn resolve_approval(&mut self, allow: bool, cx: &mut Context<Self>) {
        if let Some(approval) = self.approval.take() {
            let _ = approval.reply.send(allow);
        }
        self.bump();
        if self.busy {
            self.status = Some(if allow { "Working…" } else { "Thinking…" }.into());
        }
        cx.notify();
    }

    // ---- Turn lifecycle -------------------------------------------------------

    async fn run_send(
        this: &WeakEntity<Self>,
        generation: u64,
        text: String,
        cx: &mut AsyncApp,
    ) -> Result<(), String> {
        let browser = this
            .read_with(cx, |s, _| s.browser.clone())
            .map_err(|_| DISCONNECTED.to_string())?;
        if !browser
            .read_with(cx, |b, _| b.settings.ai_integration_enabled)
            .unwrap_or(true)
        {
            return Err(SETTINGS_DISABLED.into());
        }
        let client = Self::ensure_connected(this, generation, cx).await?;
        let thread_id = Self::ensure_thread(this, &client, generation, cx).await?;
        let uses_tools = this
            .update(cx, |s, cx| {
                s.status = Some("Thinking…".into());
                cx.notify();
                s.uses_dynamic_tools
            })
            .map_err(|_| DISCONNECTED.to_string())?;
        let params = json!({
            "threadId": thread_id,
            "input": [{ "type": "text", "text": tools::prompt_for(&text, uses_tools) }],
            "approvalPolicy": APPROVAL_POLICY,
        });
        let response = call(&client, "turn/start", params, TURN_START_TIMEOUT, cx).await?;
        let turn_id = protocol::turn_identifier(&response);
        this.update(cx, |s, cx| {
            if s.generation != generation {
                // Stopped while turn/start was in flight: stop that turn too.
                if let Some(turn_id) = turn_id {
                    s.ignored_turn_ids.insert(turn_id.clone());
                    s.interrupt_remote(&thread_id, &turn_id);
                }
                return;
            }
            if s.active_turn_id.is_none() {
                s.active_turn_id = turn_id;
            }
            s.arm_watchdog(cx);
        })
        .map_err(|_| DISCONNECTED.to_string())
    }

    /// The live connection, starting the app server and initializing it if
    /// there is none.
    async fn ensure_connected(
        this: &WeakEntity<Self>,
        generation: u64,
        cx: &mut AsyncApp,
    ) -> Result<CodexClient, String> {
        let existing = this
            .read_with(cx, |s, _| s.connection.as_ref().map(|c| c.client.clone()))
            .map_err(|_| DISCONNECTED.to_string())?;
        if let Some(client) = existing {
            return Ok(client);
        }
        let connected = server::connect(working_dir())
            .await
            .map_err(|e| e.to_string())?;
        let client = connected.client;
        this.update(cx, |s, cx| {
            if s.generation == generation {
                s.status = Some("Connecting to Codex…".into());
                cx.notify();
            }
        })
        .ok();
        call(
            &client,
            "initialize",
            json!({
                "clientInfo": { "name": "Mori", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": true, "requestAttestation": false },
            }),
            REQUEST_TIMEOUT,
            cx,
        )
        .await
        .map_err(|e| format!("Couldn't initialize the Codex app server: {e}"))?;
        client.notify("initialized", json!({}));
        this.update(cx, |s, cx| {
            let events = s.spawn_event_pump(connected.events, cx);
            s.connection = Some(Connection {
                client: client.clone(),
                events,
            });
            s.thread_id = None;
        })
        .map_err(|_| DISCONNECTED.to_string())?;
        Ok(client)
    }

    /// The current thread, starting one (with browser tools when possible).
    async fn ensure_thread(
        this: &WeakEntity<Self>,
        client: &CodexClient,
        generation: u64,
        cx: &mut AsyncApp,
    ) -> Result<String, String> {
        let (existing, unsupported) = this
            .read_with(cx, |s, _| {
                (s.thread_id.clone(), s.dynamic_tools_unsupported)
            })
            .map_err(|_| DISCONNECTED.to_string())?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let mut params = json!({
            "cwd": working_dir().to_string_lossy(),
            "approvalPolicy": APPROVAL_POLICY,
            "sandbox": SANDBOX,
            "personality": "friendly",
            "serviceName": SERVICE_NAME,
        });
        let want_tools = env_tools_enabled() && !unsupported;
        let mut uses_tools = false;
        let mut note = None;
        let response = if want_tools {
            let mut with_tools = params.clone();
            with_tools["dynamicTools"] = tools::dynamic_tools();
            match call(client, "thread/start", with_tools, THREAD_START_TIMEOUT, cx).await {
                Ok(r) => {
                    uses_tools = true;
                    r
                }
                Err(e) if protocol::is_dynamic_tools_unsupported_error(&e) => {
                    log::warn!("Codex app server rejected dynamicTools: {e}");
                    note = Some(
                        "This Codex version doesn't support Mori's browser tools, so the assistant can't see or use your tabs. Update Codex to enable them.",
                    );
                    this.update(cx, |s, _| s.dynamic_tools_unsupported = true)
                        .ok();
                    call(
                        client,
                        "thread/start",
                        params.take(),
                        THREAD_START_TIMEOUT,
                        cx,
                    )
                    .await?
                }
                Err(e) => return Err(e),
            }
        } else {
            call(client, "thread/start", params, THREAD_START_TIMEOUT, cx).await?
        };
        let id = response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or("thread/start did not return a thread id.")?
            .to_string();
        this.update(cx, |s, cx| {
            if let Some(note) = note {
                s.messages.push(Message::new(Role::Tool, note));
            }
            // Keep the thread even if the user stopped meanwhile; it's
            // reused by the next message.
            if s.generation == generation || s.thread_id.is_none() {
                s.thread_id = Some(id.clone());
                s.uses_dynamic_tools = uses_tools;
            }
            cx.notify();
        })
        .map_err(|_| DISCONNECTED.to_string())?;
        Ok(id)
    }

    fn begin_turn_output(&mut self) {
        self.agent_items.clear();
        self.loose_assistant = None;
        self.turn_has_text = false;
    }

    /// Back to idle (busy off, watchdog off).
    fn end_turn(&mut self) {
        self.busy = false;
        self.status = None;
        self.active_turn_id = None;
        self.watchdog_epoch += 1;
        self.deny_pending_approval();
    }

    fn fail_turn(&mut self, message: String, cx: &mut Context<Self>) {
        self.messages.push(Message::new(Role::Error, message));
        self.end_turn();
        cx.notify();
    }

    /// Successful end of a turn; recovers the answer from the thread if no
    /// text streamed in.
    fn complete_turn(&mut self, cx: &mut Context<Self>) {
        if self.turn_has_text {
            self.end_turn();
            cx.notify();
            return;
        }
        let (Some(client), Some(thread_id)) = (self.client(), self.thread_id.clone()) else {
            self.messages.push(Message::new(Role::Assistant, "Done."));
            self.end_turn();
            cx.notify();
            return;
        };
        let generation = self.generation;
        self.status = Some("Reading Codex…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let params = json!({ "threadId": thread_id, "includeTurns": true });
            let text = call(&client, "thread/read", params, REQUEST_TIMEOUT, cx)
                .await
                .ok()
                .and_then(|r| protocol::latest_agent_message(&r));
            this.update(cx, |s, cx| {
                if s.generation != generation || !s.busy {
                    return;
                }
                let text = text
                    .map(|t| protocol::strip_session_summary(&t))
                    .filter(|t| !t.trim().is_empty());
                s.messages.push(Message::new(
                    Role::Assistant,
                    text.unwrap_or_else(|| "Done.".into()),
                ));
                s.end_turn();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Mark the active turn as abandoned and ask the server to stop it.
    fn stop_active_turn(&mut self) {
        if let Some(turn_id) = self.active_turn_id.take() {
            self.ignored_turn_ids.insert(turn_id.clone());
            if let Some(thread_id) = self.thread_id.clone() {
                self.interrupt_remote(&thread_id, &turn_id);
            }
        }
    }

    /// Fire-and-forget `turn/interrupt`.
    fn interrupt_remote(&self, thread_id: &str, turn_id: &str) {
        if let Some(client) = self.client() {
            // The reply is irrelevant; dropping the receiver discards it.
            drop(client.request(
                "turn/interrupt",
                json!({ "threadId": thread_id, "turnId": turn_id }),
            ));
        }
    }

    fn client(&self) -> Option<CodexClient> {
        self.connection.as_ref().map(|c| c.client.clone())
    }

    fn bump(&mut self) {
        self.last_activity = Instant::now();
    }

    // ---- Watchdog -------------------------------------------------------------

    fn arm_watchdog(&mut self, cx: &mut Context<Self>) {
        self.watchdog_epoch += 1;
        let epoch = self.watchdog_epoch;
        self.bump();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(WATCHDOG_TICK).await;
                let keep = this
                    .update(cx, |s, cx| s.watchdog_epoch == epoch && s.check_idle(cx))
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    /// Returns whether the watchdog should keep running.
    fn check_idle(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.busy {
            return false;
        }
        if self.approval.is_some() {
            // Paused while the user decides.
            self.bump();
            return true;
        }
        if self.last_activity.elapsed() < TURN_IDLE_TIMEOUT {
            return true;
        }
        self.stop_active_turn();
        self.generation += 1;
        self.fail_turn(
            "Codex stopped responding for 90 seconds, so this request was cancelled. Please try again.".into(),
            cx,
        );
        false
    }

    // ---- Server events --------------------------------------------------------

    fn spawn_event_pump(
        &self,
        mut events: UnboundedReceiver<ServerEvent>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this.update(cx, |s, cx| s.handle_event(event, cx)).is_err() {
                    break;
                }
            }
        })
    }

    fn handle_event(&mut self, event: ServerEvent, cx: &mut Context<Self>) {
        match event {
            ServerEvent::Notification { method, params } => {
                self.handle_notification(&method, &params, cx)
            }
            ServerEvent::Request { id, method, params } => {
                self.handle_server_request(id, &method, params, cx)
            }
            ServerEvent::Closed(reason) => {
                log::warn!("Codex app server connection closed: {reason}");
                // This runs inside the pump task; let it finish on its own.
                if let Some(connection) = self.connection.take() {
                    connection.events.detach();
                }
                self.thread_id = None;
                if self.busy {
                    self.generation += 1;
                    self.fail_turn(
                        format!("{DISCONNECTED} Send your message again to restart it."),
                        cx,
                    );
                }
            }
        }
    }

    fn handle_notification(&mut self, method: &str, params: &Value, cx: &mut Context<Self>) {
        if let (Some(theirs), Some(ours)) = (
            protocol::thread_identifier(params),
            self.thread_id.as_deref(),
        ) && theirs != ours
        {
            return;
        }
        let turn_id = protocol::turn_identifier(params);
        if self.should_ignore(method, turn_id.as_deref()) {
            return;
        }
        if !self.busy {
            return;
        }
        // Any notification — reasoning deltas included — is a sign of life.
        self.bump();
        match method {
            "item/agentMessage/delta" => {
                if let Some(delta) =
                    protocol::find_string("delta", params).filter(|d| !d.is_empty())
                {
                    let item = params.get("itemId").and_then(Value::as_str);
                    self.append_agent_text(item, delta);
                }
            }
            "item/completed" => {
                if let Some((item, text)) = protocol::completed_agent_message(params) {
                    let text = protocol::strip_session_summary(text);
                    if !text.trim().is_empty() {
                        self.set_agent_text(item, text);
                    }
                }
            }
            "turn/started" => {
                if turn_id.is_some() {
                    self.active_turn_id = turn_id;
                }
                if self.approval.is_none() {
                    self.status = Some("Working…".into());
                }
            }
            "turn/completed" => {
                if let Some(error) = protocol::turn_error_message(params) {
                    self.fail_turn(format!("Codex failed: {error}"), cx);
                    return;
                }
                self.complete_turn(cx);
                return;
            }
            "error" => {
                let message = protocol::error_message(params);
                if params.get("willRetry").and_then(Value::as_bool) == Some(true) {
                    self.status = Some(format!("Retrying… ({})", tools::clip(&message, 80)));
                } else {
                    self.fail_turn(format!("Codex failed: {message}"), cx);
                    return;
                }
            }
            _ => return,
        }
        cx.notify();
    }

    /// Notifications of stopped turns are dropped (port of the Swift client).
    fn should_ignore(&mut self, method: &str, turn_id: Option<&str>) -> bool {
        if let Some(turn_id) = turn_id
            && self.ignored_turn_ids.contains(turn_id)
        {
            if method == "turn/completed" || method == "error" {
                self.ignored_turn_ids.remove(turn_id);
            }
            return true;
        }
        if !self.stop_requested {
            return false;
        }
        if method == "turn/started"
            && let Some(turn_id) = turn_id
        {
            // A turn that started after the user hit stop: stop it as well.
            self.ignored_turn_ids.insert(turn_id.to_string());
            if let Some(thread_id) = self.thread_id.clone() {
                self.interrupt_remote(&thread_id, turn_id);
            }
        }
        true
    }

    fn assistant_bubble(&mut self, item: Option<&str>) -> usize {
        let existing = match item {
            Some(item) => self.agent_items.get(item).copied(),
            None => self.loose_assistant,
        };
        if let Some(index) = existing.filter(|i| *i < self.messages.len()) {
            return index;
        }
        self.messages
            .push(Message::new(Role::Assistant, String::new()));
        let index = self.messages.len() - 1;
        match item {
            Some(item) => {
                self.agent_items.insert(item.to_string(), index);
            }
            None => self.loose_assistant = Some(index),
        }
        index
    }

    fn append_agent_text(&mut self, item: Option<&str>, delta: &str) {
        let index = self.assistant_bubble(item);
        self.messages[index].text.push_str(delta);
        self.turn_has_text = true;
    }

    /// The authoritative full text of a finished agent message.
    fn set_agent_text(&mut self, item: Option<&str>, text: String) {
        let index = self.assistant_bubble(item);
        self.messages[index].text = text;
        self.turn_has_text = true;
    }

    // ---- Server requests (tool calls, approvals) ------------------------------

    fn handle_server_request(
        &mut self,
        id: Value,
        method: &str,
        params: Value,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.client() else { return };
        self.bump();
        match method {
            "item/tool/call" => self.start_tool_call(client, id, params, cx),
            // The assistant is a browser agent: it gets no shell or file
            // access beyond what its sandbox allows without asking.
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                client.respond(&id, json!({ "decision": "decline" }));
                self.note_declined(method, cx);
            }
            "execCommandApproval" | "applyPatchApproval" => {
                client.respond(&id, json!({ "decision": "denied" }));
                self.note_declined(method, cx);
            }
            "mcpServer/elicitation/request" => client.respond(&id, json!({ "action": "decline" })),
            other => {
                log::debug!("unsupported app-server request {other}");
                client.respond_error(&id, -32601, &format!("Mori does not support {other}."));
            }
        }
    }

    fn note_declined(&mut self, method: &str, cx: &mut Context<Self>) {
        let what = if method.contains("ile") || method.contains("atch") {
            "change files"
        } else {
            "run a command"
        };
        self.messages.push(Message::new(
            Role::Tool,
            format!("Declined Codex's request to {what}"),
        ));
        self.loose_assistant = None;
        cx.notify();
    }

    fn start_tool_call(
        &mut self,
        client: CodexClient,
        id: Value,
        params: Value,
        cx: &mut Context<Self>,
    ) {
        let same_thread = match (
            protocol::thread_identifier(&params),
            self.thread_id.as_deref(),
        ) {
            (Some(theirs), Some(ours)) => theirs == ours,
            _ => true,
        };
        if !self.busy || self.stop_requested || !same_thread {
            client.respond(&id, ToolOutcome::stopped().rpc_result());
            return;
        }
        let tool = params
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let args = params
            .get("arguments")
            .filter(|a| a.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let generation = self.generation;
        self.messages
            .push(Message::new(Role::Tool, tools::running_label(&tool, &args)));
        let index = self.messages.len() - 1;
        // Text after the tool call goes into a new bubble.
        self.loose_assistant = None;
        self.status = Some("Using the browser…".into());
        if self.window.is_none() {
            self.window = cx.active_window().or_else(|| cx.windows().first().copied());
        }
        let ctx = ToolCtx {
            browser: self.browser.clone(),
            window: self.window,
        };
        cx.notify();

        cx.spawn(async move |this, cx| {
            let outcome = Self::run_tool(&this, generation, &tool, &args, &ctx, cx).await;
            this.update(cx, |s, cx| {
                s.finish_tool_call(generation, index, &id, outcome, cx)
            })
            .ok();
        })
        .detach();
    }

    async fn run_tool(
        this: &WeakEntity<Self>,
        generation: u64,
        tool: &str,
        args: &Value,
        ctx: &ToolCtx,
        cx: &mut AsyncApp,
    ) -> ToolOutcome {
        if let Some(approval) = tools::approval_for(tool, args, ctx, cx).await {
            let auto = approval.read_only
                && ctx
                    .browser
                    .read_with(cx, |b, _| b.settings.agent_auto_approve_safe_reads)
                    .unwrap_or(false);
            if !auto {
                let Ok(Some(decision)) =
                    this.update(cx, |s, cx| s.ask_approval(generation, approval.text, cx))
                else {
                    return ToolOutcome::stopped();
                };
                if !decision.await.unwrap_or(false) {
                    return ToolOutcome::denied(tool, args);
                }
            }
        }
        let current = this
            .read_with(cx, |s, _| {
                s.generation == generation && s.busy && !s.stop_requested
            })
            .unwrap_or(false);
        if !current {
            return ToolOutcome::stopped();
        }
        tools::execute(tool, args, ctx, cx).await
    }

    /// Show the approval card; resolves when the user decides (or the turn
    /// ends, which denies).
    fn ask_approval(
        &mut self,
        generation: u64,
        text: String,
        cx: &mut Context<Self>,
    ) -> Option<oneshot::Receiver<bool>> {
        if generation != self.generation || !self.busy {
            return None;
        }
        self.deny_pending_approval();
        let (reply, rx) = oneshot::channel();
        self.approval = Some(PendingApproval { text, reply });
        self.status = Some("Waiting for your approval…".into());
        cx.notify();
        Some(rx)
    }

    fn deny_pending_approval(&mut self) {
        if let Some(approval) = self.approval.take() {
            let _ = approval.reply.send(false);
        }
    }

    fn finish_tool_call(
        &mut self,
        generation: u64,
        index: usize,
        id: &Value,
        outcome: ToolOutcome,
        cx: &mut Context<Self>,
    ) {
        let current = generation == self.generation && self.busy && !self.stop_requested;
        let outcome = if current {
            outcome
        } else {
            ToolOutcome::stopped()
        };
        if current && let Some(message) = self.messages.get_mut(index) {
            message.text = outcome.summary.clone();
        }
        if let Some(client) = self.client() {
            client.respond(id, outcome.rpc_result());
        }
        if current {
            self.bump();
            self.status = Some("Thinking…".into());
            cx.notify();
        }
    }
}

impl Drop for AssistantSession {
    fn drop(&mut self) {
        // Unblocks any tool call parked on an approval; dropping the
        // connection stops the app server (SIGTERM, then SIGKILL).
        self.deny_pending_approval();
        if let Some(connection) = self.connection.take() {
            drop(connection);
        }
    }
}

/// Send a request and wait for its answer, up to `timeout`.
async fn call(
    client: &CodexClient,
    method: &str,
    params: Value,
    timeout: Duration,
    cx: &AsyncApp,
) -> Result<Value, String> {
    let reply = client.request(method, params);
    let timer = cx.background_executor().timer(timeout);
    match select(reply, timer.boxed_local()).await {
        Either::Left((Ok(Ok(value)), _)) => Ok(value),
        Either::Left((Ok(Err(error)), _)) => Err(error.message),
        Either::Left((Err(_), _)) => Err(DISCONNECTED.into()),
        Either::Right(_) => Err(format!(
            "Timed out waiting for the Codex app server ({method})."
        )),
    }
}
