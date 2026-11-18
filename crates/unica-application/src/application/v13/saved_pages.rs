use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Mutex;
use uuid::Uuid;

/// Saved results whose later pages are read by cursor. One entry serves
/// every page of its walk: a token names the entry and an offset this store
/// issued for it. When the store is full, saving a new entry evicts the
/// oldest, and its tokens then read nothing.
#[derive(Debug)]
pub(crate) struct SavedPages<T> {
    prefix: &'static str,
    capacity: usize,
    saved: Mutex<Saved<T>>,
}

#[derive(Debug)]
struct Saved<T> {
    entries: HashMap<String, (T, BTreeSet<usize>)>,
    order: VecDeque<String>,
}

impl<T: Clone> SavedPages<T> {
    /// `prefix` ends with a dot and tells the cursors of one store apart.
    pub(crate) fn new(prefix: &'static str, capacity: usize) -> Self {
        Self {
            prefix,
            capacity,
            saved: Mutex::new(Saved {
                entries: HashMap::new(),
                order: VecDeque::new(),
            }),
        }
    }

    /// A fresh entry name nobody can guess.
    pub(crate) fn fresh_id() -> String {
        Uuid::new_v4().simple().to_string()
    }

    /// Issues a token for `offset` of entry `id`, saving `value` first when
    /// the entry is not held yet. `id` is 32 lowercase hex digits.
    pub(crate) fn issue(&self, id: &str, value: impl FnOnce() -> T, offset: usize) -> String {
        let mut saved = self.saved.lock().expect("saved pages poisoned");
        if let Some((_, offsets)) = saved.entries.get_mut(id) {
            offsets.insert(offset);
        } else {
            while saved.entries.len() >= self.capacity {
                let Some(oldest) = saved.order.pop_front() else {
                    break;
                };
                saved.entries.remove(&oldest);
            }
            saved
                .entries
                .insert(id.to_owned(), (value(), BTreeSet::from([offset])));
            saved.order.push_back(id.to_owned());
        }
        format!("{}{id}.{offset}", self.prefix)
    }

    /// The entry and offset a token names, or nothing for a token this store
    /// did not issue or no longer holds.
    pub(crate) fn read(&self, token: &str) -> Option<(String, T, usize)> {
        let (id, offset) = token.strip_prefix(self.prefix)?.split_once('.')?;
        if id.len() != 32
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || offset.is_empty()
            || !offset.bytes().all(|byte| byte.is_ascii_digit())
            || (offset.len() > 1 && offset.starts_with('0'))
        {
            return None;
        }
        let offset = offset.parse::<usize>().ok()?;
        let saved = self.saved.lock().expect("saved pages poisoned");
        let (value, offsets) = saved.entries.get(id)?;
        offsets
            .contains(&offset)
            .then(|| (id.to_owned(), value.clone(), offset))
    }

    /// A token of the longest shape this store issues, for sizing a page.
    pub(crate) fn token_probe(&self) -> String {
        format!("{}{}.{}", self.prefix, "f".repeat(32), usize::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::SavedPages;

    #[test]
    fn a_full_store_evicts_the_oldest_entry_and_its_tokens_read_nothing() {
        let pages = SavedPages::<u8>::new("t1.", 2);
        let ids = (0..3)
            .map(|_| SavedPages::<u8>::fresh_id())
            .collect::<Vec<_>>();
        let first = pages.issue(&ids[0], || 0, 5);
        let second = pages.issue(&ids[1], || 1, 5);
        assert_eq!(pages.read(&first), Some((ids[0].clone(), 0, 5)));
        let third = pages.issue(&ids[2], || 2, 5);
        assert_eq!(pages.read(&first), None);
        assert_eq!(pages.read(&second).map(|(_, value, _)| value), Some(1));
        assert_eq!(pages.read(&third).map(|(_, value, _)| value), Some(2));
        // An offset the store did not issue, a changed prefix or a padded
        // offset read nothing.
        for forged in [
            format!("t1.{}.6", ids[1]),
            format!("t2.{}.5", ids[1]),
            format!("t1.{}.05", ids[1]),
        ] {
            assert_eq!(pages.read(&forged), None, "{forged}");
        }
        assert!(pages.token_probe().len() >= third.len());
    }
}
