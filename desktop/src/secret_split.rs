//! Credentials larger than one native item (Windows).
//!
//! Windows Credential Manager keeps at most 2560 bytes per item, and `keyring`
//! stores UTF-16, so about 1280 characters. Some sign-in grants are longer.
//! This store keeps a value that fits in one item exactly as before. A longer
//! value goes into extra items `{key}#1`, `{key}#2`, ... and `{key}` holds a
//! small header with the total length and the part count.
//!
//! Design notes: `docs/architecture/secret-vault.md`, section 7.
// Only Windows builds use it; the logic is platform independent and its tests
// run everywhere against an in-memory backend that enforces the limit.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use std::sync::Arc;

use mailx_core::secrets::{SecretStore, SecretStoreError, SecretValue};

/// UTF-16 code units one Windows item holds (2560 bytes).
pub(crate) const WINDOWS_ITEM_LIMIT: usize = 1280;
/// Starts the header item of a split value. A control character cannot begin
/// a password or a stored grant, so a header never looks like a plain value.
const MAGIC: &str = "\u{1}mailx-split:v1:";
/// Upper bound on parts, far above any stored grant (128 x 1280 units).
const MAX_PARTS: usize = 128;

pub(crate) struct SplitSecretStore {
    inner: Arc<dyn SecretStore>,
    limit: usize,
}

struct Header {
    units: usize,
    parts: usize,
}

fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

fn part_key(key: &str, index: usize) -> String {
    format!("{key}#{index}")
}

/// `Ok(None)` is a plain value. A value that starts like a header but does not
/// parse is damaged and must not be returned as a secret.
fn classify(raw: &str) -> Result<Option<Header>, SecretStoreError> {
    let Some(rest) = raw.strip_prefix(MAGIC) else {
        return Ok(None);
    };
    let (units, parts) = rest.split_once(':').ok_or(SecretStoreError::Failed)?;
    let number = |text: &str| text.parse::<usize>().map_err(|_| SecretStoreError::Failed);
    let header = Header {
        units: number(units)?,
        parts: number(parts)?,
    };
    if header.parts == 0 || header.parts > MAX_PARTS {
        return Err(SecretStoreError::Failed);
    }
    Ok(Some(header))
}

impl SplitSecretStore {
    pub(crate) fn new(inner: Arc<dyn SecretStore>, limit: usize) -> Self {
        Self { inner, limit }
    }

    /// The header currently stored under `key`, if the value there is split.
    fn stored_header(&self, key: &str) -> Result<Option<Header>, SecretStoreError> {
        match self.inner.get(key)? {
            Some(raw) => classify(raw.expose_secret()),
            None => Ok(None),
        }
    }

    /// Parts the current header claims. A damaged header counts as none (its
    /// parts cannot be located); a locked or unavailable store is an error so
    /// stale parts are never silently left behind.
    fn stored_parts(&self, key: &str) -> Result<usize, SecretStoreError> {
        match self.stored_header(key) {
            Ok(header) => Ok(header.map_or(0, |header| header.parts)),
            Err(SecretStoreError::Failed) => Ok(0),
            Err(error) => Err(error),
        }
    }

    fn delete_parts(
        &self,
        key: &str,
        range: impl Iterator<Item = usize>,
    ) -> Result<(), SecretStoreError> {
        for index in range {
            self.inner.delete(&part_key(key, index))?;
        }
        Ok(())
    }

    /// Pieces of at most `limit` UTF-16 units that never cut a character.
    fn chunks<'a>(&self, text: &'a str) -> Vec<&'a str> {
        let mut parts = Vec::new();
        let (mut start, mut used) = (0, 0);
        for (offset, ch) in text.char_indices() {
            let width = ch.len_utf16();
            if used + width > self.limit {
                parts.push(&text[start..offset]);
                start = offset;
                used = 0;
            }
            used += width;
        }
        parts.push(&text[start..]);
        parts
    }
}

impl SecretStore for SplitSecretStore {
    fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
        let Some(raw) = self.inner.get(key)? else {
            return Ok(None);
        };
        let Some(header) = classify(raw.expose_secret())? else {
            return Ok(Some(raw));
        };
        let mut whole = String::new();
        for index in 1..=header.parts {
            // A missing or short part is damage, never "no secret".
            let part = self
                .inner
                .get(&part_key(key, index))?
                .ok_or(SecretStoreError::Failed)?;
            whole.push_str(part.expose_secret());
        }
        let whole = SecretValue::new(whole);
        if units(whole.expose_secret()) != header.units {
            return Err(SecretStoreError::Failed);
        }
        Ok(Some(whole))
    }

    fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
        let text = value.expose_secret();
        let old_parts = self.stored_parts(key)?;
        if units(text) <= self.limit && !text.starts_with(MAGIC) {
            self.inner.set(key, value)?;
            return self.delete_parts(key, 1..=old_parts);
        }
        let chunks = self.chunks(text);
        if chunks.len() > MAX_PARTS {
            return Err(SecretStoreError::Failed);
        }
        // Parts first, header last: a reader never sees a header that points
        // at parts not written yet.
        for (index, chunk) in chunks.iter().enumerate() {
            self.inner.set(
                &part_key(key, index + 1),
                &SecretValue::new((*chunk).to_owned()),
            )?;
        }
        let header = format!("{MAGIC}{}:{}", units(text), chunks.len());
        self.inner.set(key, &SecretValue::new(header))?;
        self.delete_parts(key, chunks.len() + 1..=old_parts)
    }

    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        // A damaged header still gets removed; its parts cannot be located.
        let parts = self.stored_parts(key)?;
        // Parts first: if this stops halfway, a retry still finds the header.
        self.delete_parts(key, 1..=parts)?;
        self.inner.delete(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory store with the Windows item limit: writing a longer value fails.
    #[derive(Default)]
    struct Limited(Mutex<HashMap<String, String>>);
    impl SecretStore for Limited {
        fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .map(SecretValue::new))
        }
        fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
            if units(value.expose_secret()) > WINDOWS_ITEM_LIMIT {
                return Err(SecretStoreError::Failed);
            }
            self.0
                .lock()
                .unwrap()
                .insert(key.into(), value.expose_secret().into());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }
    fn fixture() -> (Arc<Limited>, SplitSecretStore) {
        let raw = Arc::new(Limited::default());
        (raw.clone(), SplitSecretStore::new(raw, WINDOWS_ITEM_LIMIT))
    }
    fn keys(raw: &Limited) -> Vec<String> {
        let mut keys: Vec<_> = raw.0.lock().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }
    fn value(len: usize) -> SecretValue {
        SecretValue::new(
            (0..len)
                .map(|i| char::from(b'a' + (i % 26) as u8))
                .collect(),
        )
    }
    fn read(store: &SplitSecretStore, key: &str) -> Option<String> {
        store
            .get(key)
            .unwrap()
            .map(|v| v.expose_secret().to_owned())
    }

    #[test]
    fn short_values_stay_in_one_plain_item() {
        let (raw, store) = fixture();
        for len in [0, 1, 40, WINDOWS_ITEM_LIMIT] {
            store.set("k", &value(len)).unwrap();
            assert_eq!(keys(&raw), ["k"]);
            assert_eq!(
                raw.get("k").unwrap().unwrap().expose_secret(),
                value(len).expose_secret()
            );
            assert_eq!(read(&store, "k").unwrap(), value(len).expose_secret());
        }
        // Items written before this wrapper existed read back unchanged.
        raw.set("old", &SecretValue::new("legacy password".into()))
            .unwrap();
        assert_eq!(read(&store, "old").unwrap(), "legacy password");
        assert_eq!(read(&store, "absent"), None);
    }

    #[test]
    fn long_values_split_and_round_trip() {
        let (raw, store) = fixture();
        for len in [WINDOWS_ITEM_LIMIT + 1, 2 * WINDOWS_ITEM_LIMIT, 5000] {
            store.set("k", &value(len)).unwrap();
            assert_eq!(read(&store, "k").unwrap(), value(len).expose_secret());
        }
        let parts = 5000_usize.div_ceil(WINDOWS_ITEM_LIMIT);
        assert_eq!(keys(&raw).len(), parts + 1);
        assert!(keys(&raw).contains(&"k#1".to_owned()));
    }

    #[test]
    fn splitting_never_cuts_a_character_and_counts_utf16_units() {
        let (_, store) = fixture();
        // Each emoji is two UTF-16 units; 700 of them is 1400 units.
        let text = "\u{1F600}".repeat(700) + "é中";
        store.set("k", &SecretValue::new(text.clone())).unwrap();
        assert_eq!(read(&store, "k").unwrap(), text);
    }

    #[test]
    fn shrinking_removes_stale_parts_and_delete_removes_everything() {
        let (raw, store) = fixture();
        store.set("k", &value(4000)).unwrap();
        store.set("k", &value(2000)).unwrap();
        assert_eq!(keys(&raw), ["k", "k#1", "k#2"]);
        store.set("k", &value(10)).unwrap();
        assert_eq!(keys(&raw), ["k"]);
        store.set("k", &value(3000)).unwrap();
        store.delete("k").unwrap();
        assert!(keys(&raw).is_empty());
        store.delete("k").unwrap();
        // Neighbouring keys are untouched.
        store.set("k", &value(3000)).unwrap();
        store.set("k2", &value(5)).unwrap();
        store.delete("k").unwrap();
        assert_eq!(keys(&raw), ["k2"]);
    }

    #[test]
    fn a_value_that_looks_like_a_header_is_stored_and_read_back_verbatim() {
        let (_, store) = fixture();
        let text = format!("{MAGIC}5:1");
        store.set("k", &SecretValue::new(text.clone())).unwrap();
        assert_eq!(read(&store, "k").unwrap(), text);
    }

    #[test]
    fn damaged_split_values_are_errors_not_missing_secrets() {
        let (raw, store) = fixture();
        store.set("k", &value(3000)).unwrap();
        raw.delete("k#2").unwrap();
        assert_eq!(store.get("k").err(), Some(SecretStoreError::Failed));
        raw.set("k#2", &SecretValue::new("short".into())).unwrap();
        assert_eq!(store.get("k").err(), Some(SecretStoreError::Failed));
        for bad in [
            format!("{MAGIC}x:1"),
            format!("{MAGIC}5:0"),
            format!("{MAGIC}5"),
        ] {
            raw.set("bad", &SecretValue::new(bad)).unwrap();
            assert_eq!(store.get("bad").err(), Some(SecretStoreError::Failed));
            // A damaged header can still be replaced and removed.
            store.set("bad", &value(3)).unwrap();
            assert_eq!(read(&store, "bad").unwrap(), value(3).expose_secret());
        }
    }

    #[test]
    fn a_failed_part_write_never_publishes_a_header() {
        struct FailsAt(Limited, &'static str);
        impl SecretStore for FailsAt {
            fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
                self.0.get(key)
            }
            fn set(&self, key: &str, v: &SecretValue) -> Result<(), SecretStoreError> {
                if key == self.1 {
                    return Err(SecretStoreError::Unavailable);
                }
                self.0.set(key, v)
            }
            fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
                self.0.delete(key)
            }
        }
        let inner = Arc::new(FailsAt(Limited::default(), "k#2"));
        let store = SplitSecretStore::new(inner.clone(), WINDOWS_ITEM_LIMIT);
        let error = store.set("k", &value(3000)).err();
        assert_eq!(error, Some(SecretStoreError::Unavailable));
        assert!(inner.get("k").unwrap().is_none());
    }
}
