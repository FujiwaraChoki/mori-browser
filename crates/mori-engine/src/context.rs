//! Browsing contexts: which cookies, cache and site storage a page uses.
//!
//! Every page belongs to one Chromium request context. The default one is
//! CEF's global context (`<profile root>/Default`). A *persistent* context
//! keeps its own data in another directory under the profile root, so a
//! page in it is signed in to different accounts than the same site in the
//! default context. A *private* context lives in memory only: nothing it
//! sees is written to disk, and its data is gone once the last page using
//! it has closed and the last handle to it is dropped.

use std::{cell::RefCell, collections::HashMap, path::PathBuf, sync::OnceLock};

use cef::*;

/// The profile root (`root_cache_path`); persistent contexts live below it.
static ROOT: OnceLock<PathBuf> = OnceLock::new();

pub(crate) fn set_root(dir: PathBuf) {
    let _ = ROOT.set(dir);
}

thread_local! {
    /// Persistent contexts by directory name. Chromium keeps a profile
    /// loaded for the whole session once created, so one context per
    /// directory is reused rather than created again.
    static PERSISTENT: RefCell<HashMap<String, RequestContext>> = RefCell::new(HashMap::new());
}

#[derive(Clone)]
enum Kind {
    Default,
    Persistent(String, RequestContext),
    Private(RequestContext),
}

/// A request context pages can be created in. Cheap to clone.
#[derive(Clone)]
pub struct BrowsingContext(Kind);

impl std::fmt::Debug for BrowsingContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Kind::Default => write!(f, "BrowsingContext(default)"),
            Kind::Persistent(dir, _) => write!(f, "BrowsingContext({dir})"),
            Kind::Private(_) => write!(f, "BrowsingContext(private)"),
        }
    }
}

impl BrowsingContext {
    /// CEF's global context, where pages go unless told otherwise.
    pub fn shared() -> BrowsingContext {
        BrowsingContext(Kind::Default)
    }

    /// The context keeping its data in `<profile root>/<dir_name>`, created
    /// on first use. `None` if the engine isn't running or Chromium refused
    /// (the caller falls back to the default context).
    pub fn persistent(dir_name: &str) -> Option<BrowsingContext> {
        if let Some(ctx) = PERSISTENT.with(|p| p.borrow().get(dir_name).cloned()) {
            return Some(BrowsingContext(Kind::Persistent(dir_name.to_string(), ctx)));
        }
        let root = ROOT.get()?;
        let path = root.join(dir_name);
        std::fs::create_dir_all(&path).ok();
        let settings = RequestContextSettings {
            cache_path: CefString::from(path.to_string_lossy().as_ref()),
            persist_session_cookies: true as _,
            ..Default::default()
        };
        let ctx = request_context_create_context(Some(&settings), None)?;
        PERSISTENT.with(|p| p.borrow_mut().insert(dir_name.to_string(), ctx.clone()));
        Some(BrowsingContext(Kind::Persistent(dir_name.to_string(), ctx)))
    }

    /// A new in-memory context that shares nothing with any other.
    pub fn private() -> Option<BrowsingContext> {
        // An empty cache path makes Chromium create an off-the-record
        // profile of its own for this context.
        let settings = RequestContextSettings::default();
        request_context_create_context(Some(&settings), None).map(|c| BrowsingContext(Kind::Private(c)))
    }

    pub fn is_private(&self) -> bool {
        matches!(self.0, Kind::Private(_))
    }

    /// The directory name of a persistent context.
    pub fn dir_name(&self) -> Option<&str> {
        match &self.0 {
            Kind::Persistent(dir, _) => Some(dir),
            _ => None,
        }
    }

    /// Every persistent context created so far this session.
    pub fn loaded_persistent() -> Vec<BrowsingContext> {
        PERSISTENT.with(|p| {
            p.borrow()
                .iter()
                .map(|(dir, ctx)| BrowsingContext(Kind::Persistent(dir.clone(), ctx.clone())))
                .collect()
        })
    }

    /// Whether `self` and `other` are the same context.
    pub fn same_as(&self, other: &BrowsingContext) -> bool {
        match (&self.0, &other.0) {
            (Kind::Default, Kind::Default) => true,
            (Kind::Persistent(a, _), Kind::Persistent(b, _)) => a == b,
            (Kind::Private(a), Kind::Private(b)) => a.is_same(Some(&mut b.clone())) == 1,
            _ => false,
        }
    }

    /// The CEF request context to apply settings to: CEF's global one for
    /// the default context.
    pub(crate) fn cef_context(&self) -> Option<RequestContext> {
        match &self.0 {
            Kind::Default => request_context_get_global_context(),
            Kind::Persistent(_, ctx) | Kind::Private(ctx) => Some(ctx.clone()),
        }
    }

    pub(crate) fn request_context(&self) -> Option<RequestContext> {
        match &self.0 {
            Kind::Default => None,
            Kind::Persistent(_, ctx) | Kind::Private(ctx) => Some(ctx.clone()),
        }
    }

    /// Whether `ctx` (a browser's request context) is this one.
    pub(crate) fn matches(&self, ctx: &RequestContext) -> bool {
        match &self.0 {
            Kind::Default => ctx.is_global() == 1,
            Kind::Persistent(_, own) | Kind::Private(own) => ctx.is_same(Some(&mut own.clone())) == 1,
        }
    }
}
