//! Profiles: separate cookie jars, logins and site data per space.
//!
//! Every space uses the default profile unless it is assigned a named one
//! (Arc). A named profile keeps its Chromium data in its own directory
//! under the engine's profile root, so a site signed in to in one profile
//! is signed out in another. Deleting a profile forgets it at once; its
//! directory is removed on the next start, when Chromium no longer has it
//! open.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{paths, workspace::SpaceId};

pub const PROFILES_FILE: &str = "profiles.json";

pub type ProfileId = Uuid;

/// Longest profile name kept.
const MAX_NAME_CHARS: usize = 40;

const DIR_PREFIX: &str = "Profile-";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
}

/// A deleted profile and the spaces that used it, for undo.
#[derive(Clone, Debug, PartialEq)]
pub struct DeletedProfile {
    pub profile: Profile,
    pub spaces: Vec<SpaceId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Assignment {
    space: SpaceId,
    profile: ProfileId,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileStore {
    profiles: Vec<Profile>,
    /// Spaces using a named profile; every other space uses the default.
    assignments: Vec<Assignment>,
    /// Data directories of deleted profiles, removed on the next start.
    pending_removal: Vec<String>,
}

/// The data directory name of profile `id`, under the engine's profile root.
pub fn dir_name(id: ProfileId) -> String {
    format!("{DIR_PREFIX}{id}")
}

/// Whether `name` is a profile data directory name (and nothing that could
/// point elsewhere, like `..`).
pub fn is_dir_name(name: &str) -> bool {
    name.strip_prefix(DIR_PREFIX)
        .and_then(|id| Uuid::parse_str(id).ok())
        .is_some_and(|id| dir_name(id) == name)
}

fn clean_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_NAME_CHARS)
        .collect()
}

impl ProfileStore {
    pub fn load(dir: &Path) -> ProfileStore {
        let mut store: ProfileStore = paths::read_json(&dir.join(PROFILES_FILE)).unwrap_or_default();
        store.repair();
        store
    }

    /// Drop duplicate profiles, assignments to unknown profiles, and
    /// removal entries that aren't profile directories.
    fn repair(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.profiles.retain(|p| seen.insert(p.id));
        let mut spaces = std::collections::HashSet::new();
        self.assignments
            .retain(|a| seen.contains(&a.profile) && spaces.insert(a.space));
        self.pending_removal.retain(|d| is_dir_name(d));
    }

    pub fn list(&self) -> &[Profile] {
        &self.profiles
    }

    pub fn get(&self, id: ProfileId) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// The name for a new profile: `wanted` (tidied), or "Profile N" when
    /// blank; numbered when another profile already has it.
    fn unique_name(&self, wanted: &str, except: Option<ProfileId>) -> String {
        let base = match clean_name(wanted) {
            n if n.is_empty() => format!("Profile {}", self.profiles.len() + 1),
            n => n,
        };
        let taken = |name: &str| {
            self.profiles
                .iter()
                .any(|p| Some(p.id) != except && p.name.eq_ignore_ascii_case(name))
        };
        if !taken(&base) {
            return base;
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|n| !taken(n))
            .unwrap_or(base)
    }

    pub fn create(&mut self, name: &str) -> ProfileId {
        let profile = Profile {
            id: Uuid::new_v4(),
            name: self.unique_name(name, None),
        };
        let id = profile.id;
        self.profiles.push(profile);
        id
    }

    /// Rename a profile; a blank name keeps the old one.
    pub fn rename(&mut self, id: ProfileId, name: &str) -> bool {
        if clean_name(name).is_empty() {
            return false;
        }
        let name = self.unique_name(name, Some(id));
        match self.profiles.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.name = name;
                true
            }
            None => false,
        }
    }

    /// Delete a profile. Its spaces go back to the default profile and its
    /// data directory is queued for removal (until then, [`restore`] undoes
    /// it).
    ///
    /// [`restore`]: ProfileStore::restore
    pub fn delete(&mut self, id: ProfileId) -> Option<DeletedProfile> {
        let i = self.profiles.iter().position(|p| p.id == id)?;
        let profile = self.profiles.remove(i);
        let spaces = self.spaces_of(id);
        self.assignments.retain(|a| a.profile != id);
        self.pending_removal.push(dir_name(id));
        Some(DeletedProfile { profile, spaces })
    }

    /// Bring a deleted profile back with its spaces and data. Spaces
    /// assigned another profile meanwhile keep it.
    pub fn restore(&mut self, deleted: DeletedProfile) {
        let DeletedProfile { profile, spaces } = deleted;
        let id = profile.id;
        self.pending_removal.retain(|d| *d != dir_name(id));
        if self.get(id).is_some() {
            return;
        }
        self.profiles.push(profile);
        for space in spaces {
            if self.profile_of(space).is_none() {
                self.assignments.push(Assignment { space, profile: id });
            }
        }
    }

    /// The named profile `space` uses (`None`: the default profile).
    pub fn profile_of(&self, space: SpaceId) -> Option<ProfileId> {
        self.assignments.iter().find(|a| a.space == space).map(|a| a.profile)
    }

    /// Spaces using profile `id`.
    pub fn spaces_of(&self, id: ProfileId) -> Vec<SpaceId> {
        self.assignments
            .iter()
            .filter(|a| a.profile == id)
            .map(|a| a.space)
            .collect()
    }

    /// Make `space` use `profile` (`None`: the default). Unknown profiles
    /// are ignored. Returns whether anything changed.
    pub fn assign(&mut self, space: SpaceId, profile: Option<ProfileId>) -> bool {
        if profile.is_some_and(|p| self.get(p).is_none()) || self.profile_of(space) == profile {
            return false;
        }
        self.assignments.retain(|a| a.space != space);
        if let Some(profile) = profile {
            self.assignments.push(Assignment { space, profile });
        }
        true
    }

    /// Forget assignments of spaces that no longer exist.
    pub fn retain_spaces(&mut self, exists: impl Fn(SpaceId) -> bool) {
        self.assignments.retain(|a| exists(a.space));
    }

    /// Data directories to remove before the engine starts; the queue is
    /// emptied.
    pub fn take_pending_removal(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_removal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    #[test]
    fn names_are_tidied_and_unique() {
        let mut store = ProfileStore::default();
        let work = store.create("  Work   stuff ");
        assert_eq!(store.get(work).unwrap().name, "Work stuff");
        let again = store.create("work stuff");
        assert_eq!(store.get(again).unwrap().name, "work stuff 2");
        let blank = store.create("   ");
        assert_eq!(store.get(blank).unwrap().name, "Profile 3");
        let long = store.create(&"x".repeat(100));
        assert_eq!(store.get(long).unwrap().name.chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn rename_keeps_names_unique_and_ignores_blank() {
        let mut store = ProfileStore::default();
        let a = store.create("Work");
        let b = store.create("Home");
        assert!(store.rename(b, "Work"));
        assert_eq!(store.get(b).unwrap().name, "Work 2");
        // Renaming to its own name (any case) is not a clash.
        assert!(store.rename(a, "WORK"));
        assert_eq!(store.get(a).unwrap().name, "WORK");
        assert!(!store.rename(a, "  "));
        assert_eq!(store.get(a).unwrap().name, "WORK");
        assert!(!store.rename(Uuid::new_v4(), "Ghost"));
    }

    #[test]
    fn assignment_and_deletion() {
        let mut store = ProfileStore::default();
        let (s1, s2) = (Uuid::new_v4(), Uuid::new_v4());
        let work = store.create("Work");
        assert!(store.assign(s1, Some(work)));
        assert!(!store.assign(s1, Some(work)));
        assert!(!store.assign(s2, Some(Uuid::new_v4())));
        assert_eq!(store.profile_of(s1), Some(work));
        assert_eq!(store.profile_of(s2), None);
        assert_eq!(store.spaces_of(work), vec![s1]);

        let deleted = store.delete(work).unwrap();
        assert_eq!(deleted.profile.name, "Work");
        assert_eq!(deleted.spaces, vec![s1]);
        assert_eq!(store.profile_of(s1), None);
        assert_eq!(store.take_pending_removal(), vec![dir_name(work)]);
        assert!(store.take_pending_removal().is_empty());
        assert!(store.delete(work).is_none());
    }

    #[test]
    fn undo_delete_brings_back_profile_spaces_and_data() {
        let mut store = ProfileStore::default();
        let (s1, s2) = (Uuid::new_v4(), Uuid::new_v4());
        let work = store.create("Work");
        let home = store.create("Home");
        store.assign(s1, Some(work));
        store.assign(s2, Some(work));
        let deleted = store.delete(work).unwrap();
        // Meanwhile s2 moved to another profile: it keeps that one.
        store.assign(s2, Some(home));
        store.restore(deleted.clone());
        assert_eq!(store.get(work).unwrap().name, "Work");
        assert_eq!(store.profile_of(s1), Some(work));
        assert_eq!(store.profile_of(s2), Some(home));
        // Its data is no longer queued for removal.
        assert!(store.take_pending_removal().is_empty());
        // Restoring twice changes nothing.
        store.restore(deleted);
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn back_to_default() {
        let mut store = ProfileStore::default();
        let s = Uuid::new_v4();
        let p = store.create("Work");
        store.assign(s, Some(p));
        assert!(store.assign(s, None));
        assert_eq!(store.profile_of(s), None);
        assert!(!store.assign(s, None));
    }

    #[test]
    fn dir_names_are_validated() {
        let id = Uuid::new_v4();
        assert!(is_dir_name(&dir_name(id)));
        assert!(!is_dir_name("Default"));
        assert!(!is_dir_name("Profile-../../etc"));
        assert!(!is_dir_name(&format!("Profile-{id}/..")));
        assert!(!is_dir_name(&format!("Profile-{}", id.simple())));
    }

    #[test]
    fn load_repairs_untrusted_files() {
        let tmp = TempDir::new();
        let p = Uuid::new_v4();
        let (s1, s2) = (Uuid::new_v4(), Uuid::new_v4());
        let json = serde_json::json!({
            "profiles": [{"id": p, "name": "Work"}, {"id": p, "name": "Dup"}],
            "assignments": [
                {"space": s1, "profile": p},
                {"space": s1, "profile": p},
                {"space": s2, "profile": Uuid::new_v4()}
            ],
            "pending_removal": ["../evil", dir_name(p)]
        });
        paths::write_json_atomic(&tmp.path().join(PROFILES_FILE), &json).unwrap();
        let mut store = ProfileStore::load(tmp.path());
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.profile_of(s1), Some(p));
        assert_eq!(store.profile_of(s2), None);
        assert_eq!(store.spaces_of(p).len(), 1);
        assert_eq!(store.take_pending_removal(), vec![dir_name(p)]);

        store.retain_spaces(|s| s != s1);
        assert_eq!(store.profile_of(s1), None);
    }
}
