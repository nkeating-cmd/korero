//! Kōrero — OS keychain wrapper for post-process LLM API keys.
//!
//! Upstream Handy serialised API keys as plaintext inside settings_store.json
//! alongside the rest of the AppSettings struct. Whisper-style transcription
//! works fine without keys, but the moment a user configures DeepSeek / OpenAI
//! / Anthropic / OpenRouter / Groq, their key sat on disk in cleartext.
//!
//! This module routes every key read/write through the platform-native secret
//! store:
//!
//! * Windows  → Credential Manager (DPAPI-protected, per-user)
//! * macOS    → Keychain (per-user, optionally per-app via signing identity)
//! * Linux    → no store is compiled in (Kōrero ships for Windows); keys there
//!   last for the session only, and saving one reports that it was not kept
//!
//! On startup, [`settings::load_or_create_app_settings`] runs a one-shot
//! migration: any plaintext key still in `settings_store.json` is copied into
//! the keychain and then blanked on disk. After that, the JSON only ever
//! contains empty strings for known provider IDs; the real values live in the
//! keychain and are hydrated into the in-memory [`SecretMap`] on every load.
//!
//! All errors are non-fatal — if the keychain is unavailable (locked, sandbox,
//! headless CI), Kōrero logs and falls back to the in-memory map for the
//! current session. This means a misbehaving keychain never bricks the app;
//! at worst the user re-enters their key.

use keyring::Entry;
use log::{debug, warn};

/// Service name registered with the OS credential store.
///
/// Kept stable across versions so an upgrade doesn't orphan existing entries.
/// Macron is fine on every supported platform (UTF-8 service strings).
const SERVICE: &str = "Kōrero — post-process LLM";

/// Construct a keyring entry handle for a given provider id.
///
/// Returns `None` if the keyring backend rejects the (service, account) tuple
/// — typically because the platform's credential store is unavailable. Callers
/// must treat `None` as "cannot persist, log and continue".
fn entry(provider_id: &str) -> Option<Entry> {
    match Entry::new(SERVICE, provider_id) {
        Ok(entry) => Some(entry),
        Err(err) => {
            warn!(
                "secret_store: failed to bind keyring entry for '{}': {}",
                provider_id, err
            );
            None
        }
    }
}

/// What the OS keychain holds for a provider.
///
/// Kōrero 1.45.1 (review of F15): "nothing stored" and "could not read" are
/// different answers. Until a real store was compiled in, nothing could tell
/// them apart and nothing needed to. With one, treating a failed read as "no
/// key" would blank the key in memory, and the next settings save would then
/// DELETE a perfectly good stored key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    Key(String),
    Absent,
    Unreadable,
}

/// Read a provider's key from the OS keychain, keeping "absent" and
/// "unreadable" apart (see [`Stored`]).
pub fn read_api_key(provider_id: &str) -> Stored {
    let Some(entry) = entry(provider_id) else {
        return Stored::Unreadable;
    };
    match entry.get_password() {
        Ok(key) if !key.is_empty() => {
            debug!("secret_store: loaded key for '{}'", provider_id);
            Stored::Key(key)
        }
        Ok(_) => Stored::Absent,
        Err(keyring::Error::NoEntry) => Stored::Absent,
        Err(err) => {
            warn!(
                "secret_store: failed to read keyring entry for '{}': {}",
                provider_id, err
            );
            Stored::Unreadable
        }
    }
}

/// Read an API key from the OS keychain.
///
/// Returns `Some(key)` on success, `None` if the entry is missing, the
/// keychain is unavailable, or the stored value can't be retrieved. Use
/// [`read_api_key`] wherever the difference decides what happens to the key.
pub fn load_api_key(provider_id: &str) -> Option<String> {
    match read_api_key(provider_id) {
        Stored::Key(key) => Some(key),
        Stored::Absent | Stored::Unreadable => None,
    }
}

/// What a settings save does with one provider's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Skip,
    Save,
    Delete,
}

/// Kōrero 1.45.1 (review of F15): decide what a settings save does with one
/// provider's key. `previous` is the value the app held before this save
/// (`None` during start-up, before there is one); `in_store` is whether this
/// process has seen the key in the OS keychain (read it, or written it).
///
///   * An empty value deletes the stored key ONLY when the user just cleared a
///     key that was there. An empty value that was already empty says nothing:
///     it may be a key that could not be read, and must never delete one.
///   * A key is written when it changed, or when it has not been seen in the
///     keychain yet (a plaintext key waiting to be moved there).
///   * An unchanged key already in the keychain is left alone, so the 60-odd
///     settings saves that have nothing to do with keys never touch it.
pub fn key_action(previous: Option<&str>, new: &str, in_store: bool) -> KeyAction {
    if new.is_empty() {
        return match previous {
            Some(p) if !p.is_empty() => KeyAction::Delete,
            _ => KeyAction::Skip,
        };
    }
    if in_store && previous == Some(new) {
        KeyAction::Skip
    } else {
        KeyAction::Save
    }
}

/// The value the app holds for a provider after reading the keychain: the
/// stored key when there is one, otherwise whatever it already held (a
/// plaintext key not yet moved, or nothing). Never blanks on a failed read.
pub fn hydrated(current: &str, stored: Stored) -> String {
    match stored {
        Stored::Key(key) => key,
        Stored::Absent | Stored::Unreadable => current.to_string(),
    }
}

/// Persist an API key to the OS keychain.
///
/// An empty `value` is treated as a delete request — this matches the UX of
/// the settings form, where clearing the field should fully revoke the stored
/// secret rather than leaving an empty-string ghost in the credential store.
///
/// Returns `true` if the operation succeeded, `false` on any error. Errors are
/// logged but never propagated; failing to write a key should not crash the
/// settings save flow.
pub fn save_api_key(provider_id: &str, value: &str) -> bool {
    let Some(entry) = entry(provider_id) else {
        return false;
    };

    if value.is_empty() {
        return delete_inner(&entry, provider_id);
    }

    match entry.set_password(value) {
        Ok(()) => {
            // Kōrero 1.45.1 (F15, L-007): a write only counts once a FRESH handle
            // can read it back. A store that accepts the write and forgets it
            // (keyring's mock store, when no platform backend is compiled in)
            // used to report success here, and the migration then blanked the
            // only copy of the key.
            if load_api_key(provider_id).as_deref() == Some(value) {
                debug!("secret_store: saved key for '{}'", provider_id);
                true
            } else {
                warn!(
                    "secret_store: key for '{}' was written but could not be read back; treating as not saved",
                    provider_id
                );
                false
            }
        }
        Err(err) => {
            warn!(
                "secret_store: failed to write keyring entry for '{}': {}",
                provider_id, err
            );
            false
        }
    }
}

/// Remove an API key from the OS keychain.
///
/// Returns `true` if the entry was deleted or already absent; `false` only on
/// a hard backend error. Treating "already gone" as success keeps the caller's
/// idempotency contract clean.
// Korero (v1.2.0): kept as public API for future use (key rotation, provider
// removal). Not called in the current build -- suppress the dead_code lint.
#[allow(dead_code)]
pub fn delete_api_key(provider_id: &str) -> bool {
    let Some(entry) = entry(provider_id) else {
        return false;
    };
    delete_inner(&entry, provider_id)
}

fn delete_inner(entry: &Entry, provider_id: &str) -> bool {
    match entry.delete_credential() {
        Ok(()) => {
            debug!("secret_store: deleted key for '{}'", provider_id);
            true
        }
        Err(keyring::Error::NoEntry) => true, // idempotent
        Err(err) => {
            warn!(
                "secret_store: failed to delete keyring entry for '{}': {}",
                provider_id, err
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn an_empty_value_only_deletes_a_key_the_user_just_cleared() {
        assert_eq!(key_action(Some("sk-live"), "", true), KeyAction::Delete);
        assert_eq!(key_action(Some("sk-live"), "", false), KeyAction::Delete);
        // Already empty: it may be a key that could not be read. Never delete.
        assert_eq!(key_action(Some(""), "", true), KeyAction::Skip);
        assert_eq!(key_action(Some(""), "", false), KeyAction::Skip);
        // Start-up, before the app holds any settings: never delete.
        assert_eq!(key_action(None, "", false), KeyAction::Skip);
        assert_eq!(key_action(None, "", true), KeyAction::Skip);
    }

    #[test]
    fn a_key_is_written_when_it_changed_or_is_not_in_the_store_yet() {
        assert_eq!(key_action(Some("old"), "new", true), KeyAction::Save);
        assert_eq!(key_action(Some(""), "new", false), KeyAction::Save);
        assert_eq!(key_action(None, "plain", false), KeyAction::Save);
        // A plaintext key the app loaded from disk and has not moved yet.
        assert_eq!(key_action(Some("plain"), "plain", false), KeyAction::Save);
        // Unchanged and already stored: an unrelated settings save leaves it alone.
        assert_eq!(key_action(Some("same"), "same", true), KeyAction::Skip);
    }

    #[test]
    fn a_failed_or_empty_read_never_blanks_what_the_app_holds() {
        assert_eq!(hydrated("", Stored::Key("k".into())), "k");
        assert_eq!(hydrated("plain", Stored::Key("k".into())), "k");
        assert_eq!(hydrated("plain", Stored::Absent), "plain");
        assert_eq!(hydrated("plain", Stored::Unreadable), "plain");
        assert_eq!(hydrated("", Stored::Unreadable), "");
    }

    /// Build a provider id that won't collide with any real Kōrero entry
    /// even if the test crashes mid-run and leaves crud behind.
    fn unique_provider_id() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("korero_test_{nanos}")
    }

    /// RAII cleanup so a panicking assert still removes the test entry.
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = delete_api_key(&self.0);
        }
    }

    /// Keychain round-trip: save → load returns the same value.
    ///
    /// `#[ignore]` keeps this off the default test run because keyring writes
    /// to the real OS credential store, which:
    ///   * needs an interactive desktop session on Linux (Secret Service)
    ///   * mutates user state on every run
    ///   * may prompt for unlock on macOS depending on the user's keychain
    ///     access controls.
    ///
    /// Run on demand with `cargo test -- --ignored keychain_roundtrip`.
    ///
    /// Kōrero 1.45.1 (F15, L-007): on Windows this runs by default. Credential
    /// Manager needs no desktop prompt, and leaving the test ignored is how a
    /// build with no platform store shipped from v1.18.1 to v1.45.0: every
    /// save and load here uses its own handle, so it fails on a mock store.
    #[test]
    #[cfg_attr(not(windows), ignore)]
    fn keychain_roundtrip() {
        let provider = unique_provider_id();
        let _guard = Cleanup(provider.clone());

        // Empty load on a fresh provider id returns None.
        assert!(
            load_api_key(&provider).is_none(),
            "fresh provider id should not have a stored key"
        );

        // Save a key, then load it back.
        assert!(
            save_api_key(&provider, "korero-roundtrip-secret"),
            "save_api_key should succeed against the OS keychain"
        );
        assert_eq!(
            load_api_key(&provider).as_deref(),
            Some("korero-roundtrip-secret"),
            "load_api_key should return the value we just saved"
        );

        // Empty save deletes — load returns None.
        assert!(save_api_key(&provider, ""), "empty save should delete");
        assert!(
            load_api_key(&provider).is_none(),
            "deleted entry should not return a value"
        );

        // Delete is idempotent.
        assert!(
            delete_api_key(&provider),
            "delete of already-empty entry should succeed (idempotent)"
        );
    }
}
