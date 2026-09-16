//! Experimental admission control for pending chunks versus new sheets.
use std::collections::BTreeMap;
use std::sync::Mutex;
static PENDING: Mutex<BTreeMap<usize, usize>> = Mutex::new(BTreeMap::new());

pub struct PendingChunk(usize);
impl PendingChunk {
    pub fn new(bytes: usize) -> Self {
        *PENDING.lock().unwrap().entry(bytes).or_default() += 1;
        Self(bytes)
    }
}
impl Drop for PendingChunk {
    fn drop(&mut self) {
        let mut pending = PENDING.lock().unwrap();
        let count = pending.get_mut(&self.0).unwrap();
        *count -= 1;
        if *count == 0 {
            pending.remove(&self.0);
        }
    }
}
/// Atomically compare ready chunk sizes and claim the next sheet.
pub fn admit_sheet<T>(bytes: usize, claim: impl FnOnce() -> T) -> Option<T> {
    let pending = PENDING.lock().unwrap();
    if pending
        .last_key_value()
        .is_some_and(|(largest, _)| *largest > bytes)
    {
        None
    } else {
        Some(claim())
    }
}
