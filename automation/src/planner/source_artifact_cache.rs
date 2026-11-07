use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

pub(crate) const DEFAULT_SOURCE_ARTIFACT_CACHE_MAX_ENTRIES: usize = 80;

pub(crate) struct BoundedResultCache<T> {
    max_entries: usize,
    entries: Mutex<HashMap<String, Arc<OnceLock<Result<T, String>>>>>,
    insertion_order: Mutex<VecDeque<String>>,
}

impl<T> BoundedResultCache<T>
where
    T: Clone,
{
    pub(crate) fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            entries: Mutex::new(HashMap::new()),
            insertion_order: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) fn get_or_load(
        &self,
        key: &str,
        loader: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        if self.max_entries == 0 {
            return loader();
        }

        let entry = {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| "source artifact cache was poisoned".to_string())?;
            if let Some(entry) = entries.get(key) {
                entry.clone()
            } else {
                let entry = Arc::new(OnceLock::new());
                entries.insert(key.to_string(), entry.clone());
                let mut insertion_order = self
                    .insertion_order
                    .lock()
                    .map_err(|_| "source artifact cache order was poisoned".to_string())?;
                insertion_order.push_back(key.to_string());
                while insertion_order.len() > self.max_entries {
                    if let Some(oldest_key) = insertion_order.pop_front() {
                        entries.remove(&oldest_key);
                    }
                }
                entry
            }
        };

        let result = entry.get_or_init(loader).clone();
        if result.is_err() {
            if let Ok(mut entries) = self.entries.lock() {
                if entries
                    .get(key)
                    .is_some_and(|current| Arc::ptr_eq(current, &entry))
                {
                    entries.remove(key);
                    if let Ok(mut insertion_order) = self.insertion_order.lock() {
                        insertion_order.retain(|cached_key| cached_key != key);
                    }
                }
            }
        }
        result
    }
}
