//! Passwords in the system keyring, through the Secret Service D-Bus API
//! (GNOME Keyring, KWallet, KeePassXC…).
//!
//! Every call blocks on D-Bus (and, for a locked keyring, on the user
//! answering the unlock prompt), so they run on one worker thread, in
//! order. Each job opens its own encrypted session (Diffie-Hellman), so
//! secrets never cross the bus in the clear. Items are tagged with the
//! login's id, which is how Mori finds them again; the origin and username
//! are attributes too, so the keyring's own app shows what they are.

use std::{collections::HashMap, sync::mpsc, thread};

use futures::channel::oneshot;
use mori_core::passwords::{Secret, display_origin};
use secret_service::{EncryptionType, Error, blocking::SecretService};
use uuid::Uuid;

/// Attribute holding the login's id.
const ID_ATTRIBUTE: &str = "mori-id";
/// The schema libsecret-based tools group the items under.
const SCHEMA: &str = "app.mori.Password";

type Job = Box<dyn FnOnce() + Send>;

/// Whether passwords can be saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultStatus {
    /// Not checked yet.
    Unknown,
    Ready,
    /// No keyring could be reached; the reason, for the user.
    Unavailable(String),
}

pub struct Vault {
    jobs: mpsc::Sender<Job>,
}

impl Vault {
    pub fn new() -> Vault {
        let (jobs, rx) = mpsc::channel::<Job>();
        let spawned = thread::Builder::new().name("mori-keyring".into()).spawn(move || {
            for job in rx {
                job();
            }
        });
        if let Err(e) = spawned {
            log::error!("passwords: cannot start the keyring thread: {e}");
        }
        Vault { jobs }
    }

    /// Run `f` against the keyring on the worker thread.
    pub fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&SecretService) -> Result<T, String> + Send + 'static,
    ) -> oneshot::Receiver<Result<T, String>> {
        let (tx, rx) = oneshot::channel();
        let job: Job = Box::new(move || {
            let result = SecretService::connect(EncryptionType::Dh)
                .map_err(describe)
                .and_then(|ss| f(&ss));
            let _ = tx.send(result);
        });
        if let Err(mpsc::SendError(job)) = self.jobs.send(job) {
            // The worker is gone: answer on this thread instead.
            job();
        }
        rx
    }
}

/// A user-facing reason for a keyring failure.
pub fn describe(err: Error) -> String {
    const NOT_RUNNING: &str = "No keyring service (Secret Service) is running.";
    let text = err.to_string();
    if text.contains("ServiceUnknown") || text.contains("NameHasNoOwner") {
        return NOT_RUNNING.into();
    }
    match err {
        Error::Unavailable => NOT_RUNNING.into(),
        Error::Prompt | Error::PromptDisconnected => "The keyring wasn't unlocked.".into(),
        Error::Locked => "The keyring is locked.".into(),
        Error::NoResult => "The keyring has no collection to store passwords in.".into(),
        other => format!("Keyring error: {other}"),
    }
}

/// Check that a collection exists to store passwords in.
pub fn probe(ss: &SecretService) -> Result<(), String> {
    ss.get_any_collection().map(|_| ()).map_err(describe)
}

fn label(origin: &str, username: &str) -> String {
    match username {
        "" => format!("Mori: {}", display_origin(origin)),
        user => format!("Mori: {user} on {}", display_origin(origin)),
    }
}

fn attributes<'a>(id: &'a str, origin: &'a str, username: &'a str) -> HashMap<&'a str, &'a str> {
    HashMap::from([("xdg:schema", SCHEMA), (ID_ATTRIBUTE, id), ("origin", origin), ("username", username)])
}

/// Store (or replace) the password of login `id`.
pub fn store(ss: &SecretService, id: Uuid, origin: &str, username: &str, secret: &Secret) -> Result<(), String> {
    let id = id.to_string();
    let attrs = attributes(&id, origin, username);
    let found = ss.search_items(HashMap::from([(ID_ATTRIBUTE, id.as_str())])).map_err(describe)?;
    if let Some(item) = found.unlocked.first().or(found.locked.first()) {
        item.ensure_unlocked().map_err(describe)?;
        item.set_secret(secret.expose().as_bytes(), "text/plain").map_err(describe)?;
        item.set_attributes(attrs).map_err(describe)?;
        return item.set_label(&label(origin, username)).map_err(describe);
    }
    let collection = ss.get_any_collection().map_err(describe)?;
    collection.ensure_unlocked().map_err(describe)?;
    collection
        .create_item(&label(origin, username), attrs, secret.expose().as_bytes(), true, "text/plain")
        .map(|_| ())
        .map_err(describe)
}

/// The password of login `id`, or `None` if the keyring has no such item.
pub fn fetch(ss: &SecretService, id: Uuid) -> Result<Option<Secret>, String> {
    let id = id.to_string();
    let found = ss.search_items(HashMap::from([(ID_ATTRIBUTE, id.as_str())])).map_err(describe)?;
    let Some(item) = found.unlocked.first().or(found.locked.first()) else { return Ok(None) };
    item.ensure_unlocked().map_err(describe)?;
    let bytes = item.get_secret().map_err(describe)?;
    Secret::from_utf8(bytes).map(Some).ok_or_else(|| "The saved password isn't text.".into())
}

/// The passwords of several logins (for export); missing ones are left out.
pub fn fetch_many(ss: &SecretService, ids: &[Uuid]) -> Result<Vec<(Uuid, Secret)>, String> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(secret) = fetch(ss, *id)? {
            out.push((*id, secret));
        }
    }
    Ok(out)
}

/// Update what the keyring item of login `id` says it is for.
pub fn relabel(ss: &SecretService, id: Uuid, origin: &str, username: &str) -> Result<(), String> {
    let id_text = id.to_string();
    let found = ss.search_items(HashMap::from([(ID_ATTRIBUTE, id_text.as_str())])).map_err(describe)?;
    for item in found.unlocked.iter().chain(found.locked.iter()) {
        item.ensure_unlocked().map_err(describe)?;
        item.set_attributes(attributes(&id_text, origin, username)).map_err(describe)?;
        item.set_label(&label(origin, username)).map_err(describe)?;
    }
    Ok(())
}

/// Delete the keyring item of login `id` (fine if it is already gone).
pub fn delete(ss: &SecretService, id: Uuid) -> Result<(), String> {
    let id = id.to_string();
    let found = ss.search_items(HashMap::from([(ID_ATTRIBUTE, id.as_str())])).map_err(describe)?;
    for item in found.unlocked.iter().chain(found.locked.iter()) {
        item.ensure_unlocked().map_err(describe)?;
        item.delete().map_err(describe)?;
    }
    Ok(())
}
