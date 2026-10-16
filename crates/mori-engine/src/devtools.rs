//! The Chrome DevTools Protocol session CEF keeps for each browser.
//!
//! Mori uses it two ways:
//! - [`evaluate`]: run JavaScript and read the result back browser-side, so
//!   page script cannot forge it.
//! - A raw message pipe ([`set_sink`] / [`send`]) that the chrome bridges to
//!   a DevTools frontend. Chromium's own DevTools window cannot render
//!   offscreen, so Mori loads the frontend in an ordinary view and connects
//!   it to this session.
//!
//! Both share the session: Mori's own requests use ids from a range the
//! frontend never reaches, and their replies are kept from the frontend.
//!
//! Calls of bindings Mori installs (`Runtime.addBinding`, usually limited to
//! an isolated world page script cannot reach) arrive as
//! [`crate::ViewEvent::BindingCalled`] and are kept from the frontend too.

use std::rc::{Rc, Weak};

use cef::*;

use crate::view::ViewInner;

pub(crate) type EvalCallback = Box<dyn FnOnce(Result<serde_json::Value, String>)>;
/// Receives raw protocol messages for a connected DevTools frontend.
pub type Sink = Box<dyn Fn(&[u8])>;

/// First message id used by Mori's own requests. The frontend counts up
/// from 1.
const OWN_ID_BASE: i32 = 1 << 30;

pub(crate) fn attach(view: &Rc<ViewInner>) {
    let Some(host) = view.host() else { return };
    let mut observer = Observer::new(Rc::downgrade(view));
    let registration = host.add_dev_tools_message_observer(Some(&mut observer));
    *view.devtools_registration.borrow_mut() = registration;
}

fn own_id() -> i32 {
    OWN_ID_BASE + (crate::view::next_token() % (i32::MAX - OWN_ID_BASE) as u64) as i32
}

/// `user_gesture` makes Blink treat the evaluation as user activation (popups,
/// autoplay, clipboard writes); background scripts must pass `false`.
pub(crate) fn evaluate(
    view: &Rc<ViewInner>,
    expression: &str,
    user_gesture: bool,
    callback: EvalCallback,
) {
    let params = serde_json::json!({
        "expression": expression,
        "returnByValue": true,
        "awaitPromise": true,
        "userGesture": user_gesture,
    });
    call(
        view,
        "Runtime.evaluate",
        params,
        Box::new(move |result| callback(result.and_then(eval_value))),
    );
}

/// Call a protocol method; `callback` gets its `result` object.
pub(crate) fn call(view: &Rc<ViewInner>, method: &str, params: serde_json::Value, callback: EvalCallback) {
    let Some(host) = view.host() else {
        callback(Err("page is not ready".into()));
        return;
    };
    let id = own_id();
    let message = serde_json::json!({ "id": id, "method": method, "params": params });
    view.evals.borrow_mut().insert(id, callback);
    let bytes = message.to_string().into_bytes();
    if host.send_dev_tools_message(Some(&bytes)) != 1 {
        // End the borrow before the callback, which may evaluate again.
        let cb = view.evals.borrow_mut().remove(&id);
        if let Some(cb) = cb {
            cb(Err("DevTools message rejected".into()));
        }
    }
}

/// Send every protocol message the session produces that Mori did not ask
/// for (replies to the frontend, events) to `sink`; `None` detaches.
pub(crate) fn set_sink(view: &ViewInner, sink: Option<Sink>) {
    *view.devtools_sink.borrow_mut() = sink;
}

/// Send a raw protocol message (from a frontend) into the session.
pub(crate) fn send(view: &ViewInner, message: &[u8]) -> bool {
    view.host()
        .is_some_and(|host| host.send_dev_tools_message(Some(message)) == 1)
}

/// A `Runtime.bindingCalled` event: the binding's name and its payload.
fn binding_call(message: &[u8]) -> Option<(String, String)> {
    const METHOD: &[u8] = b"Runtime.bindingCalled";
    // Most messages are replies and other events: skip them unparsed.
    if !message.windows(METHOD.len()).any(|w| w == METHOD) {
        return None;
    }
    #[derive(serde::Deserialize)]
    struct Params {
        name: String,
        payload: String,
    }
    #[derive(serde::Deserialize)]
    struct Event {
        method: String,
        params: Params,
    }
    let event: Event = serde_json::from_slice(message).ok()?;
    (event.method == "Runtime.bindingCalled").then_some((event.params.name, event.params.payload))
}

/// The `id` of a protocol reply, if the message is one.
fn message_id(message: &[u8]) -> Option<i64> {
    #[derive(serde::Deserialize)]
    struct Probe {
        id: Option<i64>,
    }
    serde_json::from_slice::<Probe>(message).ok()?.id
}

/// A method reply: the `result` object, or the error message.
fn decode(success: bool, result: &[u8]) -> Result<serde_json::Value, String> {
    let value: serde_json::Value =
        serde_json::from_slice(result).map_err(|e| format!("bad DevTools reply: {e}"))?;
    if !success {
        return Err(value
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("DevTools call failed")
            .to_string());
    }
    Ok(value)
}

/// The value of a `Runtime.evaluate` result, or the exception it threw.
fn eval_value(value: serde_json::Value) -> Result<serde_json::Value, String> {
    if let Some(details) = value.get("exceptionDetails") {
        let text = details
            .pointer("/exception/description")
            .or_else(|| details.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("script threw");
        return Err(text.to_string());
    }
    Ok(value
        .pointer("/result/value")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}

wrap_dev_tools_message_observer! {
    pub(crate) struct Observer {
        view: Weak<ViewInner>,
    }

    impl DevToolsMessageObserver {
        fn on_dev_tools_message(
            &self,
            _browser: Option<&mut Browser>,
            message: Option<&[u8]>,
        ) -> ::std::os::raw::c_int {
            let (Some(view), Some(message)) = (self.view.upgrade(), message) else { return 0 };
            if message_id(message).is_some_and(|id| id >= OWN_ID_BASE as i64) {
                // Mori's own request: handled in `on_dev_tools_method_result`.
                return 0;
            }
            if let Some((name, payload)) = binding_call(message) {
                // Mori's own channel (it may carry what the user typed):
                // not for an attached frontend.
                view.push(crate::ViewEvent::BindingCalled { name, payload: crate::view::BindingPayload(payload) });
                return 1;
            }
            let sink = view.devtools_sink.borrow();
            match sink.as_ref() {
                Some(sink) => {
                    sink(message);
                    1
                }
                None => 0,
            }
        }

        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut Browser>,
            message_id: ::std::os::raw::c_int,
            success: ::std::os::raw::c_int,
            result: Option<&[u8]>,
        ) {
            let Some(view) = self.view.upgrade() else { return };
            let Some(cb) = view.evals.borrow_mut().remove(&message_id) else { return };
            cb(decode(success == 1, result.unwrap_or_default()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_reply_ids() {
        assert_eq!(message_id(br#"{"id":7,"result":{}}"#), Some(7));
        assert_eq!(message_id(br#"{"method":"Page.loadEventFired","params":{}}"#), None);
        assert_eq!(message_id(b"not json"), None);
    }

    #[test]
    fn reads_binding_calls() {
        let event = br#"{"method":"Runtime.bindingCalled","params":{"name":"__mori","payload":"{\"a\":1}","executionContextId":3}}"#;
        assert_eq!(binding_call(event), Some(("__mori".into(), r#"{"a":1}"#.into())));
        assert_eq!(binding_call(br#"{"id":4,"result":{"value":"Runtime.bindingCalled"}}"#), None);
        assert_eq!(binding_call(br#"{"method":"Page.loadEventFired","params":{}}"#), None);
        // The payload can hold a password: never printed.
        let payload = crate::view::BindingPayload("secret".into());
        assert_eq!(format!("{payload:?}"), "BindingPayload(6 bytes)");
    }

    #[test]
    fn decodes_replies() {
        assert_eq!(decode(false, br#"{"message":"nope"}"#), Err("nope".into()));
        let ok = decode(true, br#"{"result":{"type":"number","value":3}}"#).unwrap();
        assert_eq!(eval_value(ok), Ok(serde_json::json!(3)));
        let threw = serde_json::json!({"exceptionDetails":{"text":"Uncaught","exception":{"description":"Error: x"}}});
        assert_eq!(eval_value(threw), Err("Error: x".into()));
    }

    #[test]
    fn own_ids_stay_above_the_frontend_range() {
        for _ in 0..1000 {
            assert!(own_id() >= OWN_ID_BASE);
        }
    }
}
