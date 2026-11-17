//! Kōrero 1.49: why a dictation clean-up did not land.
//!
//! Until 1.49 a failed clean-up was only `None`: Today inferred "clean-up
//! failed, pasted as spoken" from a missing cleaned text, and the reason
//! reached the log at best. The doctor ("Check local AI") and Today need the
//! reason, so every clean-up now ends in text or in one of these reasons, and
//! the reason is stored with the History entry.
//!
//! Privacy: a reason is a fixed word. It never holds the transcript, the
//! model's answer, an error body or a URL, so it is safe to store, show and
//! put in a support report.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CleanupFailReason {
    /// Clean-up is on but has no provider, model or prompt to use.
    NotSetUp,
    /// The AI address did not answer (Ollama stopped, wrong port, a proxy
    /// that refused the connection).
    Unreachable,
    /// Something answered at the address, but a proxy stood in the way
    /// (HTTP 407, or a proxy error page).
    ProxyBlocked,
    /// Ollama answered, but does not have the chosen model.
    ModelMissing,
    /// Not enough free memory to load or run the model.
    OutOfMemory,
    /// The text is longer than the model can read in one pass.
    TooLong,
    /// The model did not answer in time (usually still loading).
    TimedOut,
    /// A cloud provider has no API key, or refused the key.
    KeyRejected,
    /// The provider answered with an error not covered above.
    HttpError,
    /// The provider answered with nothing.
    Empty,
    /// The cursor wait ran out; the clean-up carries on in the background.
    /// Becomes `CameLate` when it then finishes, or the real reason if it fails.
    StillRunning,
    /// The clean-up finished after the words were pasted as spoken, and was
    /// filed in History.
    CameLate,
    /// Anything else.
    Other,
}

impl CleanupFailReason {
    /// The word stored in History's `post_process_failure` column.
    pub fn as_db(self) -> &'static str {
        match self {
            Self::NotSetUp => "not_set_up",
            Self::Unreachable => "unreachable",
            Self::ProxyBlocked => "proxy_blocked",
            Self::ModelMissing => "model_missing",
            Self::OutOfMemory => "out_of_memory",
            Self::TooLong => "too_long",
            Self::TimedOut => "timed_out",
            Self::KeyRejected => "key_rejected",
            Self::HttpError => "http_error",
            Self::Empty => "empty",
            Self::StillRunning => "still_running",
            Self::CameLate => "came_late",
            Self::Other => "other",
        }
    }

    /// Read a stored word back. An unknown word (a newer version wrote it) is
    /// `Other`, never an error, so History always loads.
    pub fn from_db(s: &str) -> Self {
        Self::ALL
            .iter()
            .copied()
            .find(|r| r.as_db() == s)
            .unwrap_or(Self::Other)
    }

    pub const ALL: [CleanupFailReason; 13] = [
        Self::NotSetUp,
        Self::Unreachable,
        Self::ProxyBlocked,
        Self::ModelMissing,
        Self::OutOfMemory,
        Self::TooLong,
        Self::TimedOut,
        Self::KeyRejected,
        Self::HttpError,
        Self::Empty,
        Self::StillRunning,
        Self::CameLate,
        Self::Other,
    ];

    /// Sort an error message from `llm_client` / `ollama_chat` into a reason.
    ///
    /// These messages are Kōrero's own (`format!` in those two files), plus
    /// reqwest's error text inside them. The patterns are pinned by tests
    /// that build the exact messages, so rewording one of them fails a test
    /// instead of silently turning a reason into `Other`.
    pub fn classify(msg: &str) -> Self {
        let m = msg.to_ascii_lowercase();
        // A proxy shows up as 407, or as a proxy's own error page.
        if m.contains(" 407") || m.contains("proxy") {
            return Self::ProxyBlocked;
        }
        if m.contains("too long for the model to read") {
            return Self::TooLong;
        }
        if m.contains("needs more memory than is free") || m.contains("out of memory") {
            return Self::OutOfMemory;
        }
        if m.contains("no api key for") {
            return Self::KeyRejected;
        }
        if m.contains("didn't start answering within")
            || m.contains("didn't finish its answer within")
            || m.contains("timed out")
        {
            return Self::TimedOut;
        }
        if m.contains("couldn't reach ollama")
            || m.contains("error sending request")
            || m.contains("connection refused")
            || m.contains("error trying to connect")
        {
            return Self::Unreachable;
        }
        if m.contains("404") && m.contains("not found") && m.contains("model") {
            return Self::ModelMissing;
        }
        if m.contains("status 401") || m.contains("status 403") {
            return Self::KeyRejected;
        }
        if m.contains("produced no output") || m.contains("answered with nothing") {
            return Self::Empty;
        }
        // The server answered with an error status: `ollama_chat` says
        // "Ollama returned HTTP <n>", `llm_client` "failed with status <n>".
        if m.contains("returned http ") || m.contains("failed with status ") {
            return Self::HttpError;
        }
        Self::Other
    }
}

#[cfg(test)]
mod tests {
    use super::CleanupFailReason as R;

    #[test]
    fn every_reason_round_trips_through_the_database_word() {
        for r in R::ALL {
            assert_eq!(R::from_db(r.as_db()), r, "{r:?}");
        }
        assert_eq!(R::from_db("something_a_later_version_wrote"), R::Other);
    }

    /// The exact messages `ollama_chat` and `llm_client` produce, so a
    /// rewording there shows up here.
    #[test]
    fn korero_messages_sort_into_their_reasons() {
        let cases = [
            (crate::ollama_chat::TOO_LONG.to_string(), R::TooLong),
            (
                "This model needs more memory than is free right now. Close other apps that use \
                 the graphics card, or choose a smaller model."
                    .to_string(),
                R::OutOfMemory,
            ),
            ("The local model didn't start answering within 300 s.".into(), R::TimedOut),
            ("The local model didn't finish its answer within 900 s.".into(), R::TimedOut),
            (
                "Couldn't reach Ollama: error sending request for url (http://localhost:11434/api/chat)"
                    .into(),
                R::Unreachable,
            ),
            (
                "HTTP request failed: error sending request for url (https://api.example.com/v1/chat/completions)"
                    .into(),
                R::Unreachable,
            ),
            (
                "Ollama returned HTTP 404: model \"gemma4:12b\" not found, try pulling it first"
                    .into(),
                R::ModelMissing,
            ),
            ("Ollama returned HTTP 407: Proxy Authentication Required".into(), R::ProxyBlocked),
            (
                "API request failed with status 407 Proxy Authentication Required: <html>".into(),
                R::ProxyBlocked,
            ),
            ("No API key for OpenAI: nothing was sent.".into(), R::KeyRejected),
            (
                "API request failed with status 401 Unauthorized: {\"error\":\"bad key\"}".into(),
                R::KeyRejected,
            ),
            ("The provider produced no output — the stream ended.".into(), R::Empty),
            ("gemma4:12b answered with nothing. Try another model.".into(), R::Empty),
            ("Ollama returned HTTP 500: llama runner process has terminated".into(), R::HttpError),
            ("Failed to build HTTP client: tls".into(), R::Other),
        ];
        for (msg, want) in cases {
            assert_eq!(R::classify(&msg), want, "{msg}");
        }
    }
}
