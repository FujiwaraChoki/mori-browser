//! Reports from the form agent ([`super::scripts::FORM_AGENT_SCRIPT`]).
//!
//! The agent runs in an isolated world of the page's main frame and calls a
//! DevTools binding only that world can see, so page script can neither
//! read what it collects nor send reports of its own. Payloads are still
//! checked here: anything malformed, oversized or from a non-web origin is
//! dropped.

use serde::Deserialize;

use super::{Secret, login_origin};

/// Name of the DevTools binding the agent reports through.
pub const AGENT_BINDING: &str = "__moriPasswords";
/// Name of the isolated world the agent (and the fill scripts) run in.
pub const AGENT_WORLD: &str = "mori-passwords";

const MAX_USERNAME: usize = 512;
const MAX_PASSWORD: usize = 4096;

/// Which kind of login field has the focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FieldKind {
    Username,
    Password,
    /// A field for choosing a new password (sign-up, password change).
    NewPassword,
}

/// A field's box in CSS pixels, relative to the page's viewport.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct FieldRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl FieldRect {
    fn is_valid(&self) -> bool {
        [self.x, self.y, self.w, self.h].iter().all(|v| v.is_finite() && v.abs() < 1e6) && self.w >= 0. && self.h >= 0.
    }
}

/// Something the agent saw on the page.
#[derive(Clone, Debug, PartialEq)]
pub enum FormReport {
    /// What login forms the page shows now (sent on load and on change).
    Forms { origin: String, login: bool, new_password: bool },
    /// A login field got the focus; `user` when the user put it there
    /// (clicked or tabbed), rather than the page.
    Focus { origin: String, field: FieldKind, rect: FieldRect, max_length: Option<u32>, user: bool },
    /// The focused login field moved (scrolling, layout).
    Moved { rect: FieldRect },
    /// The focus left the login field for something else in the page.
    Blur,
    /// The user started typing in the focused login field.
    Typing,
    /// A username was submitted on its own (the first step of a two-step
    /// sign-in); its password follows on the next page.
    Username { origin: String, username: String },
    /// A login form was submitted.
    Submit { origin: String, username: String, password: Secret },
    /// After a submission, the login form went away without a navigation
    /// (single-page apps): the sign-in most likely worked.
    Submitted { origin: String },
}

#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum Raw {
    Forms {
        origin: String,
        login: bool,
        #[serde(rename = "newPassword")]
        new_password: bool,
    },
    Focus {
        origin: String,
        field: FieldKind,
        rect: FieldRect,
        #[serde(rename = "maxLength", default)]
        max_length: Option<u32>,
        #[serde(default)]
        user: bool,
    },
    Move {
        rect: FieldRect,
    },
    Blur,
    Typing,
    Username {
        origin: String,
        username: String,
    },
    Submit {
        origin: String,
        username: String,
        password: String,
    },
    Submitted {
        origin: String,
    },
}

/// The web origin a report claims, if it is a normalized http(s) origin.
fn checked_origin(origin: String) -> Option<String> {
    (login_origin(&origin).as_deref() == Some(origin.as_str())).then_some(origin)
}

/// Decode an agent payload. The caller should wipe `payload` afterwards: it
/// may hold a password.
pub fn parse_report(payload: &str) -> Option<FormReport> {
    let raw: Raw = serde_json::from_str(payload).ok()?;
    Some(match raw {
        Raw::Forms { origin, login, new_password } => FormReport::Forms { origin: checked_origin(origin)?, login, new_password },
        Raw::Focus { origin, field, rect, max_length, user } => {
            if !rect.is_valid() {
                return None;
            }
            let max_length = max_length.filter(|m| *m > 0);
            FormReport::Focus { origin: checked_origin(origin)?, field, rect, max_length, user }
        }
        Raw::Move { rect } => {
            if !rect.is_valid() {
                return None;
            }
            FormReport::Moved { rect }
        }
        Raw::Blur => FormReport::Blur,
        Raw::Typing => FormReport::Typing,
        Raw::Username { origin, username } => {
            let username = username.trim().to_string();
            if username.is_empty() || username.chars().count() > MAX_USERNAME {
                return None;
            }
            FormReport::Username { origin: checked_origin(origin)?, username }
        }
        Raw::Submit { origin, username, password } => {
            let password = Secret::new(password);
            let username = username.trim().to_string();
            if password.is_empty() || password.expose().len() > MAX_PASSWORD || username.chars().count() > MAX_USERNAME {
                return None;
            }
            FormReport::Submit { origin: checked_origin(origin)?, username, password }
        }
        Raw::Submitted { origin } => FormReport::Submitted { origin: checked_origin(origin)? },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submissions() {
        let r = parse_report(r#"{"t":"submit","origin":"https://a.test","username":"  me@a.test ","password":"p\"w"}"#);
        assert_eq!(
            r,
            Some(FormReport::Submit {
                origin: "https://a.test".into(),
                username: "me@a.test".into(),
                password: Secret::new("p\"w".into()),
            })
        );
        // No password, an oversized one, or a non-web origin: dropped.
        assert_eq!(parse_report(r#"{"t":"submit","origin":"https://a.test","username":"me","password":""}"#), None);
        let huge = format!(r#"{{"t":"submit","origin":"https://a.test","username":"me","password":"{}"}}"#, "x".repeat(5000));
        assert_eq!(parse_report(&huge), None);
        assert_eq!(parse_report(r#"{"t":"submit","origin":"null","username":"me","password":"x"}"#), None);
        assert_eq!(parse_report(r#"{"t":"submit","origin":"file://","username":"me","password":"x"}"#), None);
        // Origins must already be normalized (as `location.origin` is).
        assert_eq!(parse_report(r#"{"t":"submit","origin":"https://a.test/x","username":"me","password":"x"}"#), None);
        assert_eq!(parse_report(r#"{"t":"submit","origin":"https://a.test:443","username":"me","password":"x"}"#), None);
        assert!(parse_report(r#"{"t":"submit","origin":"http://localhost:8080","username":"","password":"x"}"#).is_some());
    }

    #[test]
    fn forms_focus_and_steps() {
        assert_eq!(
            parse_report(r#"{"t":"forms","origin":"https://a.test","login":true,"newPassword":false}"#),
            Some(FormReport::Forms { origin: "https://a.test".into(), login: true, new_password: false })
        );
        assert_eq!(
            parse_report(r#"{"t":"focus","origin":"https://a.test","field":"new-password","rect":{"x":10,"y":20.5,"w":200,"h":30},"maxLength":null}"#),
            Some(FormReport::Focus {
                origin: "https://a.test".into(),
                field: FieldKind::NewPassword,
                rect: FieldRect { x: 10., y: 20.5, w: 200., h: 30. },
                max_length: None,
                user: false,
            })
        );
        let focus = parse_report(r#"{"t":"focus","origin":"https://a.test","field":"username","user":true,"rect":{"x":0,"y":0,"w":1,"h":1},"maxLength":16}"#);
        assert!(matches!(
            focus,
            Some(FormReport::Focus { field: FieldKind::Username, max_length: Some(16), user: true, .. })
        ));
        assert_eq!(parse_report(r#"{"t":"focus","origin":"https://a.test","field":"other","rect":{"x":0,"y":0,"w":1,"h":1}}"#), None);
        assert_eq!(parse_report(r#"{"t":"move","rect":{"x":0,"y":-5,"w":1e9,"h":1}}"#), None);
        assert!(parse_report(r#"{"t":"move","rect":{"x":0,"y":-5,"w":10,"h":1}}"#).is_some());
        assert_eq!(parse_report(r#"{"t":"blur"}"#), Some(FormReport::Blur));
        assert_eq!(parse_report(r#"{"t":"typing"}"#), Some(FormReport::Typing));
        assert_eq!(
            parse_report(r#"{"t":"username","origin":"https://a.test","username":" me "}"#),
            Some(FormReport::Username { origin: "https://a.test".into(), username: "me".into() })
        );
        assert_eq!(parse_report(r#"{"t":"username","origin":"https://a.test","username":"  "}"#), None);
        assert_eq!(
            parse_report(r#"{"t":"submitted","origin":"https://a.test"}"#),
            Some(FormReport::Submitted { origin: "https://a.test".into() })
        );
        assert_eq!(parse_report("not json"), None);
        assert_eq!(parse_report(r#"{"t":"unknown"}"#), None);
    }
}
