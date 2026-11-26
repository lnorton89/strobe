// ============================================================================
// Module:       strobe_forge::author
// Description:  The provider-neutral boundary `make` talks to a model
//               through: a conversation of text and image parts in, the
//               model's text out.
//
// Dependencies: anyhow; crate::anthropic (behind the `llm` feature)
// ============================================================================

//! The model boundary (`docs/LLM_SHADER_CREATION.md`, "Provider-neutral
//! boundary").
//!
//! An [`Author`] holds one conversation. The loop gives it the system
//! prompt once, then sends turns of [`Part`]s and gets the reply's text
//! back. The provider keeps its own transcript in its own shape (the
//! Anthropic one replays its replies' content blocks unchanged), so nothing
//! here knows a provider's wire format. No tools are offered: the reply is
//! text and [`crate::reply`] parses it.
//!
//! One real implementation exists, the Anthropic Messages API
//! ([`crate::anthropic`], behind the default `llm` feature); the tests use a
//! fake with canned replies, so no test needs a key or the network.

use anyhow::Result;

/// One piece of a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Part {
    /// Text.
    Text(String),
    /// An encoded image.
    Image {
        /// `image/jpeg` or `image/png`.
        media_type: String,
        /// The bytes, base64.
        base64: String,
    },
}

/// A model to write scenes with.
pub(crate) trait Author {
    /// Provider and model, for the log (never a key).
    fn describe(&self) -> String;
    /// Starts a conversation with `system` as its system prompt, dropping
    /// any earlier one.
    fn start(&mut self, system: &str);
    /// Sends one user turn and returns the reply's text.
    fn send(&mut self, turn: Vec<Part>) -> Result<String>;
}

/// The model `make` uses unless `--model` says otherwise.
pub(crate) const DEFAULT_MODEL: &str = "claude-opus-5-5";

/// The Anthropic author for `model`, its key from `ANTHROPIC_API_KEY`.
#[cfg(feature = "llm")]
pub(crate) fn from_env(model: &str) -> Result<Box<dyn Author>> {
    Ok(Box::new(crate::anthropic::Anthropic::from_env(model)?))
}

/// Without the `llm` feature there is no HTTP client, so no author.
#[cfg(not(feature = "llm"))]
pub(crate) fn from_env(model: &str) -> Result<Box<dyn Author>> {
    Err(anyhow::anyhow!(
        "strobe-forge was built without the `llm` feature, so `make` cannot reach {model}; \
         build it with default features"
    ))
}
