//! Evaluate JavaScript and read the result back through the DevTools
//! protocol. Results travel browser-side, so page script cannot forge them.

use std::rc::{Rc, Weak};

use cef::*;

use crate::view::ViewInner;

pub(crate) type EvalCallback = Box<dyn FnOnce(Result<serde_json::Value, String>)>;

pub(crate) fn attach(view: &Rc<ViewInner>) {
    let Some(host) = view.host() else { return };
    let mut observer = Observer::new(Rc::downgrade(view));
    let registration = host.add_dev_tools_message_observer(Some(&mut observer));
    *view.devtools_registration.borrow_mut() = registration;
}

pub(crate) fn evaluate(view: &Rc<ViewInner>, expression: &str, callback: EvalCallback) {
    let Some(host) = view.host() else {
        callback(Err("page is not ready".into()));
        return;
    };
    let id = (crate::view::next_token() % i32::MAX as u64) as i32;
    let message = serde_json::json!({
        "id": id,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true,
        }
    });
    view.evals.borrow_mut().insert(id, callback);
    let bytes = message.to_string().into_bytes();
    if host.send_dev_tools_message(Some(&bytes)) != 1
        && let Some(cb) = view.evals.borrow_mut().remove(&id)
    {
        cb(Err("DevTools message rejected".into()));
    }
}

fn decode(success: bool, result: &[u8]) -> Result<serde_json::Value, String> {
    let value: serde_json::Value =
        serde_json::from_slice(result).map_err(|e| format!("bad DevTools reply: {e}"))?;
    if !success {
        return Err(value
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("evaluation failed")
            .to_string());
    }
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
