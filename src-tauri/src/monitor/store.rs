use super::model::CallRecord;
use std::collections::{BTreeMap, HashMap};

/// Keep greatest (UTC milliseconds, stable ID) keys regardless of traversal order.
/// The ID index contains retained records only, so it cannot grow without bound.
pub(super) struct Store {
    pub limit: usize,
    pub truncated: bool,
    pub calls: BTreeMap<(i64, String), CallRecord>,
    ids: HashMap<String, (i64, String)>,
}
impl Store {
    pub fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            truncated: false,
            calls: BTreeMap::new(),
            ids: HashMap::new(),
        }
    }
    pub fn insert(&mut self, call: CallRecord) -> (bool, Option<String>) {
        if self.ids.contains_key(&call.id) {
            return (false, None);
        }
        let key = (call.timestamp_ms, call.id.clone());
        let mut removed = None;
        if self.calls.len() == self.limit {
            self.truncated = true;
            if self
                .calls
                .first_key_value()
                .is_some_and(|(oldest, _)| &key <= oldest)
            {
                return (false, None);
            }
            if let Some((_, old)) = self.calls.pop_first() {
                self.ids.remove(&old.id);
                removed = Some(old.id);
            }
        }
        self.ids.insert(call.id.clone(), key.clone());
        self.calls.insert(key, call);
        (true, removed)
    }
    #[cfg(test)]
    pub fn indexed_len(&self) -> usize {
        self.ids.len()
    }
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains_key(id)
    }
    pub fn snapshot(&self) -> Vec<CallRecord> {
        self.calls.values().rev().cloned().collect()
    }
}
