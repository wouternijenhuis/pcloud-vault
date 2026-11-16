//! In-memory list of recent uploads and downloads, shown on the Upload page and in the
//! status bar. Nothing here is persisted: it only describes what this session did.

use serde::Serialize;
use std::{collections::VecDeque, sync::Mutex};

const KEEP: usize = 200;

#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Upload,
    Download,
}

#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TransferState {
    Active,
    Done,
    Failed,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String,
    pub name: String,
    pub direction: Direction,
    /// "vault" (encrypted) or "cloud" (not encrypted).
    pub location: &'static str,
    pub done: u64,
    pub total: u64,
    pub state: TransferState,
    pub error: Option<String>,
    pub time: u64,
}

#[derive(Default)]
pub struct Transfers {
    items: Mutex<VecDeque<Transfer>>,
    serial: std::sync::atomic::AtomicU64,
}

impl Transfers {
    pub fn start(&self, name: &str, direction: Direction, vault: bool, total: u64) -> String {
        let serial = self
            .serial
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("t{serial}");
        if let Ok(mut items) = self.items.lock() {
            items.push_front(Transfer {
                id: id.clone(),
                name: name.into(),
                direction,
                location: if vault { "vault" } else { "cloud" },
                done: 0,
                total,
                state: TransferState::Active,
                error: None,
                time: crate::app::now(),
            });
            // Drop the oldest finished entries first; never hide active work.
            while items.len() > KEEP {
                match items
                    .iter()
                    .rposition(|t| matches!(t.state, TransferState::Done | TransferState::Failed))
                {
                    Some(index) => {
                        items.remove(index);
                    }
                    None => break,
                }
            }
        }
        id
    }

    pub fn progress(&self, id: &str, done: u64) {
        self.update(id, |t| t.done = done.min(t.total.max(done)));
    }

    pub fn finish(&self, id: &str, result: &Result<(), String>) {
        self.update(id, |t| match result {
            Ok(()) => {
                t.state = TransferState::Done;
                t.done = t.total.max(t.done);
            }
            Err(error) => {
                t.state = TransferState::Failed;
                t.error = Some(error.clone());
            }
        });
    }

    /// Runs `work` as one tracked transfer.
    pub fn track<T>(
        &self,
        name: &str,
        direction: Direction,
        vault: bool,
        total: u64,
        work: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let id = self.start(name, direction, vault, total);
        let result = work();
        self.finish(&id, &result.as_ref().map(|_| ()).map_err(Clone::clone));
        result
    }

    pub fn list(&self) -> Vec<Transfer> {
        self.items
            .lock()
            .map(|items| items.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Removes finished entries; active ones stay visible.
    pub fn clear_finished(&self) {
        if let Ok(mut items) = self.items.lock() {
            items.retain(|t| t.state == TransferState::Active);
        }
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut Transfer)) {
        if let Ok(mut items) = self.items.lock() {
            if let Some(item) = items.iter_mut().find(|t| t.id == id) {
                change(item);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_success_failure_and_clearing() {
        let transfers = Transfers::default();
        transfers
            .track("a.txt", Direction::Upload, true, 10, || Ok(()))
            .unwrap();
        let failed: Result<(), String> =
            transfers.track("b.txt", Direction::Download, false, 5, || {
                Err("offline".into())
            });
        assert!(failed.is_err());
        let active = transfers.start("c.txt", Direction::Upload, false, 7);
        transfers.progress(&active, 3);
        let list = transfers.list();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].state, TransferState::Active);
        assert_eq!(list[0].done, 3);
        assert_eq!(list[1].error.as_deref(), Some("offline"));
        assert_eq!(list[2].done, 10);
        transfers.clear_finished();
        assert_eq!(transfers.list().len(), 1);
    }
}
