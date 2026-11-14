use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: u64,
    pub starred: bool,
    pub pinned: bool,
    pub trashed: bool,
    pub sync_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplicationObject {
    pub id: String,
    pub kind: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ChunkRef {
    pub id: String,
    pub length: u32,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Manifest {
    pub file_id: String,
    pub version_id: String,
    pub size: u64,
    pub key: [u8; 32],
    pub chunks: Vec<ChunkRef>,
}

impl Drop for Manifest {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.key);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Commit {
    pub id: String,
    pub parents: Vec<String>,
    pub device_id: String,
    pub modified: u64,
    pub operation: Operation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Operation {
    Create {
        id: String,
        parent: Option<String>,
        name: String,
        kind: EntryKind,
        manifest: Option<String>,
        size: u64,
    },
    Write {
        id: String,
        manifest: String,
        size: u64,
    },
    Rename {
        id: String,
        name: String,
    },
    Move {
        id: String,
        parent: Option<String>,
    },
    Relocate {
        id: String,
        parent: Option<String>,
        name: String,
        displaced: Option<String>,
    },
    Trash {
        id: String,
        trashed: bool,
    },
}

impl Operation {
    pub(crate) fn entry_id(&self) -> &str {
        match self {
            Self::Create { id, .. }
            | Self::Write { id, .. }
            | Self::Rename { id, .. }
            | Self::Move { id, .. }
            | Self::Relocate { id, .. }
            | Self::Trash { id, .. } => id,
        }
    }
    pub(crate) fn manifest_id(&self) -> Option<&str> {
        match self {
            Self::Create { manifest, .. } => manifest.as_deref(),
            Self::Write { manifest, .. } => Some(manifest),
            _ => None,
        }
    }
    pub(crate) fn affects_entry(&self, id: &str) -> bool {
        self.entry_id() == id
            || matches!(self, Self::Relocate { displaced: Some(target), .. } if target == id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LocalState {
    pub device_id: String,
    pub pending: BTreeSet<String>,
    pub favorites: BTreeSet<String>,
    pub pinned: BTreeSet<String>,
    pub cache_limit_bytes: u64,
    pub known_commits: BTreeSet<String>,
}

impl Default for LocalState {
    fn default() -> Self {
        Self {
            device_id: crate::crypto::new_id(),
            pending: BTreeSet::new(),
            favorites: BTreeSet::new(),
            pinned: BTreeSet::new(),
            cache_limit_bytes: 10 * 1024 * 1024 * 1024,
            known_commits: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedEntry {
    pub entry: Entry,
    pub manifest: Option<String>,
    pub original: bool,
}

pub(crate) type Resolved = BTreeMap<String, ResolvedEntry>;
