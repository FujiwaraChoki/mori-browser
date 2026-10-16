//! On-disk locations and small JSON persistence helpers shared by every store.
//!
//! All stores write pretty-printed JSON atomically (temp file + rename) so a
//! crash mid-write never leaves a truncated file behind. Unreadable files are
//! copied aside as `<stem>.backup-<unixsecs>.json` before falling back to
//! defaults, so user data is never silently discarded.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use serde::{Serialize, de::DeserializeOwned};

/// Environment variable that overrides the data directory (tests, portable
/// installs, multiple profiles).
pub const DATA_DIR_ENV: &str = "MORI_DATA_DIR";

/// The directory holding all of Mori's persisted state.
///
/// `$MORI_DATA_DIR` if set and non-empty, else `<XDG data dir>/mori`
/// (e.g. `~/.local/share/mori`). The directory is created if missing.
pub fn data_dir() -> PathBuf {
    let dir = std::env::var_os(DATA_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("mori")
        });
    if let Err(err) = fs::create_dir_all(&dir) {
        log::warn!("could not create data dir {}: {err}", dir.display());
    }
    dir
}

/// Current time as fractional UNIX seconds — the timestamp format used by
/// every persisted record.
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Serialize `value` as pretty JSON and atomically replace `path` with it.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(value).context("serializing JSON")?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "data.json".into());
    let tmp = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    {
        let mut file =
            fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(&json)
            .and_then(|_| file.sync_all())
            .with_context(|| format!("writing {}", tmp.display()))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

/// Read and decode a JSON file.
///
/// Returns `None` when the file is missing or unreadable. When it exists but
/// fails to parse, it is first copied to `<stem>.backup-<unixsecs>.json` next
/// to the original so the user's data can be recovered by hand.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            log::warn!("could not read {}: {err}", path.display());
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(value) => Some(value),
        Err(err) => {
            let backup = backup_path(path);
            match fs::copy(path, &backup) {
                Ok(_) => log::error!(
                    "could not parse {} ({err}); copied it to {}",
                    path.display(),
                    backup.display()
                ),
                Err(copy_err) => log::error!(
                    "could not parse {} ({err}) and backing it up failed: {copy_err}",
                    path.display()
                ),
            }
            None
        }
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "data".into());
    path.with_file_name(format!("{stem}.backup-{}.json", now() as u64))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};

    /// A unique scratch directory under the system temp dir, removed (files
    /// only, non-recursively) on drop.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("mori-core-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            if let Ok(entries) = std::fs::read_dir(&self.0) {
                for entry in entries.flatten() {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
            let _ = std::fs::remove_dir(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::TempDir;
    use super::*;

    #[test]
    fn roundtrip_and_backup() {
        let tmp = TempDir::new();
        let path = tmp.path().join("thing.json");
        assert_eq!(read_json::<Vec<u32>>(&path), None);

        write_json_atomic(&path, &vec![1u32, 2, 3]).unwrap();
        assert_eq!(read_json::<Vec<u32>>(&path), Some(vec![1, 2, 3]));

        fs::write(&path, b"{not json").unwrap();
        assert_eq!(read_json::<Vec<u32>>(&path), None);
        let backups: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("thing.backup-"))
            .collect();
        assert_eq!(backups.len(), 1);
    }
}
