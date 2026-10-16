//! Saving stores without blocking the UI.
//!
//! [`Saver::stage`] serializes a store on the caller's thread (cheap) and,
//! only if the bytes differ from what was last written, queues them for a
//! single background writer thread that replaces the file atomically
//! (temp file, fsync, rename). Writes happen in the order they were staged,
//! so a later save of a file can never be overwritten by an earlier one.
//! [`Saver::flush`] blocks until everything queued is on disk, for quit.

use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
};

use anyhow::Context as _;
use serde::Serialize;

enum Job {
    Write { name: &'static str, path: PathBuf, bytes: Vec<u8> },
    Flush(mpsc::Sender<()>),
}

pub struct Saver {
    dir: PathBuf,
    /// Hash of the content last queued per file.
    queued: HashMap<&'static str, u64>,
    /// Files whose last write failed; they are written again next time.
    failed: Arc<Mutex<HashSet<&'static str>>>,
    tx: Option<mpsc::Sender<Job>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Saver {
    /// A saver writing files into `dir`, with its own writer thread.
    pub fn new(dir: impl Into<PathBuf>) -> Saver {
        let (tx, rx) = mpsc::channel::<Job>();
        let failed: Arc<Mutex<HashSet<&'static str>>> = Arc::default();
        let failures = failed.clone();
        let worker = thread::Builder::new()
            .name("mori-saver".into())
            .spawn(move || {
                for job in rx {
                    match job {
                        Job::Write { name, path, bytes } => {
                            if let Err(err) = crate::paths::write_bytes_atomic(&path, &bytes) {
                                log::error!("saving {name}: {err:#}");
                                if let Ok(mut f) = failures.lock() {
                                    f.insert(name);
                                }
                            }
                        }
                        Job::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })
            .ok();
        Saver {
            dir: dir.into(),
            queued: HashMap::new(),
            failed,
            tx: worker.is_some().then_some(tx),
            worker,
        }
    }

    /// Serialize `value` and queue it for `dir/file` if it changed since
    /// the last call. Returns whether a write was queued.
    pub fn stage<T: Serialize>(&mut self, file: &'static str, value: &T) -> anyhow::Result<bool> {
        let bytes = serde_json::to_vec_pretty(value).context("serializing JSON")?;
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let hash = hasher.finish();
        let retry = self.failed.lock().map(|mut f| f.remove(file)).unwrap_or(false);
        if !retry && self.queued.get(file) == Some(&hash) {
            return Ok(false);
        }
        self.queued.insert(file, hash);
        let path = self.dir.join(file);
        match &self.tx {
            Some(tx) => {
                if tx.send(Job::Write { name: file, path: path.clone(), bytes: bytes.clone() }).is_err() {
                    crate::paths::write_bytes_atomic(&path, &bytes)?;
                }
            }
            // No writer thread could be started: write synchronously.
            None => crate::paths::write_bytes_atomic(&path, &bytes)?,
        }
        Ok(true)
    }

    /// Block until every queued write has finished.
    pub fn flush(&self) {
        let Some(tx) = &self.tx else { return };
        let (done_tx, done_rx) = mpsc::channel();
        if tx.send(Job::Flush(done_tx)).is_ok() {
            let _ = done_rx.recv();
        }
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        // Closing the channel lets the writer finish its queue and exit.
        self.tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    #[test]
    fn writes_only_changes() {
        let tmp = TempDir::new();
        let mut saver = Saver::new(tmp.path());
        assert!(saver.stage("a.json", &vec![1, 2]).unwrap());
        assert!(!saver.stage("a.json", &vec![1, 2]).unwrap());
        assert!(saver.stage("b.json", &vec![1, 2]).unwrap());
        assert!(saver.stage("a.json", &vec![3]).unwrap());
        saver.flush();
        assert_eq!(crate::paths::read_json::<Vec<i32>>(&tmp.path().join("a.json")), Some(vec![3]));
        assert_eq!(crate::paths::read_json::<Vec<i32>>(&tmp.path().join("b.json")), Some(vec![1, 2]));
    }

    #[test]
    fn later_writes_win() {
        let tmp = TempDir::new();
        let mut saver = Saver::new(tmp.path());
        for i in 0..50 {
            saver.stage("n.json", &i).unwrap();
        }
        saver.flush();
        assert_eq!(crate::paths::read_json::<i32>(&tmp.path().join("n.json")), Some(49));
    }

    #[test]
    fn drop_finishes_queued_writes() {
        let tmp = TempDir::new();
        {
            let mut saver = Saver::new(tmp.path());
            saver.stage("x.json", &"kept").unwrap();
        }
        assert_eq!(
            crate::paths::read_json::<String>(&tmp.path().join("x.json")).as_deref(),
            Some("kept")
        );
    }

    #[test]
    fn failed_writes_are_retried() {
        let tmp = TempDir::new();
        // A directory where the file should be makes the rename fail.
        std::fs::create_dir_all(tmp.path().join("busy.json").join("inner")).unwrap();
        let mut saver = Saver::new(tmp.path());
        assert!(saver.stage("busy.json", &1).unwrap());
        saver.flush();
        // Same content, but the last write failed: it is queued again.
        assert!(saver.stage("busy.json", &1).unwrap());
        saver.flush();
        std::fs::remove_dir(tmp.path().join("busy.json").join("inner")).unwrap();
        std::fs::remove_dir(tmp.path().join("busy.json")).unwrap();
    }
}
