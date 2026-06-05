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
//! * Linux    → Secret Service / kwallet (whichever the desktop provides)
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

/// Read an API key from the OS keychain.
///
/// Returns `Some(key)` on success, `None` if the entry is missing, the
/// keychain is unavailable, or the stored value can't be retrieved. Distinct
/// "not found" vs "error" outcomes are deliberately collapsed — callers only
/// care whether a key is available for use *right now*.
pub fn load_api_key(provider_id: &str) -> Option<String> {
    let entry = entry(provider_id)?;
    match entry.get_password() {
        Ok(key) if !key.is_empty() => {
            debug!("secret_store: loaded key for '{}'", provider_id);
            Some(key)
        }
        Ok(_) => None,
        Err(keyring::Error::NoEntry) => None,
        Err(err) => {
            warn!(
                "secret_store: failed to read keyring entry for '{}': {}",
                provider_id, err
            );
            None
        }
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
            debug!("secret_store: saved key for '{}'", provider_id);
            true
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
    #[test]
    #[ignore]
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
