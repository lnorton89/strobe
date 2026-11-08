// ============================================================================
// Module:       strobe_forge::anthropic
// Description:  The Anthropic Messages API as an `Author`: raw HTTPS with
//               ureq (no Rust SDK exists), the key from the environment.
//
// Dependencies: anyhow, serde_json, ureq; crate::author
// ============================================================================

//! Claude through `POST /v1/messages`.
//!
//! - **Key**: `ANTHROPIC_API_KEY`, read once, sent only as the `x-api-key`
//!   header, never logged, never written, never in an error (BYOK,
//!   `docs/LLM_SHADER_CREATION.md`). Unset refuses before anything runs.
//! - **Request**: the model (default `claude-opus-5-5`), the skill as the
//!   system prompt, every turn so far, `output_config.effort` `high`, and
//!   adaptive thinking left at the model's default (it is always on for
//!   Opus 5.5; a thinking budget would be refused). On the models that
//!   take it, `fallbacks: "default"` with its beta header, so a safety
//!   classifier's false positive is rerun on a fallback model instead of
//!   ending the run.
//! - **History is append-only**: each reply's `content` blocks, thinking
//!   included, are replayed unchanged on the next request, as the API asks.
//! - **Reply**: the text blocks joined. A `refusal` or a `max_tokens` stop is
//!   an error naming it.
//!
//! Requests are not streamed: the client waits up to [`TIMEOUT`] for one
//! answer, and `max_tokens` leaves room for the thinking and a whole
//! shader.

use crate::author::{Author, Part};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::time::Duration;

/// The endpoint.
const URL: &str = "https://api.anthropic.com/v1/messages";
/// The API version sent.
const VERSION: &str = "2026-10-21";
/// The beta that turns on `fallbacks: "default"`.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// The variable the key is read from.
const KEY_VAR: &str = "ANTHROPIC_API_KEY";
/// Room for thinking and a whole scene.
const MAX_TOKENS: u32 = 32_000;
/// How long one answer may take.
const TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// A conversation with one Claude model.
pub(crate) struct Anthropic {
    key: String,
    model: String,
    system: String,
    messages: Vec<Value>,
    agent: ureq::Agent,
}

impl Anthropic {
    /// For `model`, with the key from `ANTHROPIC_API_KEY`.
    pub(crate) fn from_env(model: &str) -> Result<Self> {
        let key = std::env::var(KEY_VAR).unwrap_or_default();
        if key.trim().is_empty() {
            return Err(anyhow!(
                "{KEY_VAR} is not set: `make` sends the picture to Anthropic with your own key \
                 (bring your own key); set it in the environment and run again"
            ));
        }
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .user_agent(concat!("strobe-forge/", env!("CARGO_PKG_VERSION")))
            .build();
        Ok(Self {
            key: key.trim().to_owned(),
            model: model.to_owned(),
            system: String::new(),
            messages: Vec::new(),
            agent: ureq::Agent::new_with_config(config),
        })
    }
}

/// Whether `model` takes `fallbacks: "default"` on the Claude API.
fn takes_fallbacks(model: &str) -> bool {
    ["claude-opus-5", "claude-fable-5", "claude-sonnet-5-5"]
        .iter()
        .any(|prefix| model.starts_with(prefix))
}

/// One turn's parts as Messages API content blocks.
pub(crate) fn content(turn: &[Part]) -> Vec<Value> {
    turn.iter()
        .map(|part| match part {
            Part::Text(text) => json!({ "type": "text", "text": text }),
            Part::Image { media_type, base64 } => json!({
                "type": "image",
                "source": { "type": "base64", "media_type": media_type, "data": base64 },
            }),
        })
        .collect()
}

/// The request body.
pub(crate) fn body(model: &str, system: &str, messages: &[Value]) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "system": system,
        "messages": messages,
        "output_config": { "effort": "high" },
    });
    if takes_fallbacks(model)
        && let Some(object) = body.as_object_mut()
    {
        object.insert("fallbacks".to_owned(), json!("default"));
    }
    body
}

/// Reads a response: its content blocks (to replay) and its text, or an
/// error saying why there is no usable answer.
pub(crate) fn read_response(status: u16, text: &str) -> Result<(Value, String)> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| anyhow!("the API answered {status} with something that is not JSON: {e}"))?;
    if status != 200 {
        let kind = value
            .pointer("/error/type")
            .and_then(Value::as_str)
            .unwrap_or("error");
        let message = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("no message");
        return Err(anyhow!(
            "the API refused the request ({status} {kind}): {message}"
        ));
    }
    match value.get("stop_reason").and_then(Value::as_str) {
        Some("refusal") => {
            let category = value
                .pointer("/stop_details/category")
                .and_then(Value::as_str)
                .unwrap_or("unstated");
            return Err(anyhow!("the model declined (refusal, category {category})"));
        }
        Some("max_tokens") => {
            return Err(anyhow!(
                "the answer was cut off at max_tokens ({MAX_TOKENS})"
            ));
        }
        Some(_) | None => {}
    }
    let blocks = value.get("content").cloned().unwrap_or_else(|| json!([]));
    let text = blocks
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    Ok((blocks, text))
}

impl Author for Anthropic {
    fn describe(&self) -> String {
        format!("Anthropic {}", self.model)
    }

    fn start(&mut self, system: &str) {
        system.clone_into(&mut self.system);
        self.messages.clear();
    }

    fn send(&mut self, turn: Vec<Part>) -> Result<String> {
        self.messages
            .push(json!({ "role": "user", "content": content(&turn) }));
        let body = body(&self.model, &self.system, &self.messages);
        let mut request = self
            .agent
            .post(URL)
            .header("x-api-key", &self.key)
            .header("anthropic-version", VERSION)
            .header("content-type", "application/json");
        if takes_fallbacks(&self.model) {
            request = request.header("anthropic-beta", FALLBACK_BETA);
        }
        let outcome = request
            .send(body.to_string())
            .map_err(|e| anyhow!("could not reach the Anthropic API: {e}"))
            .and_then(|mut response| {
                let status = response.status().as_u16();
                let text = response
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| anyhow!("reading the answer: {e}"))?;
                read_response(status, &text)
            });
        match outcome {
            Ok((blocks, text)) => {
                self.messages
                    .push(json!({ "role": "assistant", "content": blocks }));
                Ok(text)
            }
            Err(error) => {
                // The turn got no answer: take it back, so a retry does not
                // send two user turns in a row.
                self.messages.pop();
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::ensure;

    #[test]
    fn the_body_carries_the_image_the_effort_and_fallbacks() -> Result<()> {
        let turn = vec![
            Part::Image {
                media_type: "image/jpeg".to_owned(),
                base64: "AAAA".to_owned(),
            },
            Part::Text("Make a scene.".to_owned()),
        ];
        let messages = vec![json!({ "role": "user", "content": content(&turn) })];
        let body = body("claude-opus-5-5", "the skill", &messages);
        ensure!(body["system"] == "the skill", "system prompt: {body}");
        ensure!(body["output_config"]["effort"] == "high", "effort: {body}");
        ensure!(
            body["fallbacks"] == "default",
            "fallbacks on Opus 5.5: {body}"
        );
        ensure!(
            body.get("thinking").is_none(),
            "no thinking config on Opus 5.5: {body}"
        );
        ensure!(
            body["messages"][0]["content"][0]["source"]["data"] == "AAAA"
                && body["messages"][0]["content"][0]["source"]["media_type"] == "image/jpeg",
            "the image block: {body}"
        );
        let other = super::body("claude-haiku-5-5", "s", &messages);
        ensure!(
            other.get("fallbacks").is_none(),
            "no fallbacks where not taken: {other}"
        );
        Ok(())
    }

    #[test]
    fn a_response_gives_its_text_and_blocks_to_replay() -> Result<()> {
        let text = r#"{"content":[{"type":"thinking","thinking":"","signature":"s"},{"type":"text","text":"Hello "},{"type":"text","text":"there"}],"stop_reason":"end_turn"}"#;
        let (blocks, reply) = read_response(200, text)?;
        ensure!(reply == "Hello there", "text joined: {reply}");
        ensure!(
            blocks.as_array().is_some_and(|b| b.len() == 3),
            "every block kept for replay, thinking included: {blocks}"
        );
        Ok(())
    }

    #[test]
    fn errors_refusals_and_truncation_are_errors() -> Result<()> {
        let api = read_response(
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
        );
        ensure!(
            api.as_ref()
                .err()
                .is_some_and(|e| e.to_string().contains("authentication_error")),
            "an API error names its type: {api:?}"
        );
        let refusal = read_response(
            200,
            r#"{"content":[],"stop_reason":"refusal","stop_details":{"type":"refusal","category":"cyber"}}"#,
        );
        ensure!(
            refusal
                .as_ref()
                .err()
                .is_some_and(|e| e.to_string().contains("cyber")),
            "a refusal names its category: {refusal:?}"
        );
        let cut = read_response(
            200,
            r#"{"content":[{"type":"text","text":"<file"}],"stop_reason":"max_tokens"}"#,
        );
        ensure!(cut.is_err(), "a cut-off answer is not used");
        Ok(())
    }
}
