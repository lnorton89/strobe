// ============================================================================
// Module:       strobe_shadertoy::model
// Description:  Layer (b): the Shadertoy shader JSON as serde types, covering
//               the API wrapper and the bare export, tolerant of unknown keys.
//
// Dependencies: serde, serde_json, thiserror
// ============================================================================

//! The Shadertoy shader JSON.
//!
//! Two shapes carry the same data:
//!
//! - the REST API's `{"Shader": {"ver", "info", "renderpass"}}`, and
//! - the bare `{"ver", "info", "renderpass"}` that the browser exports
//!   and the unofficial plugin saves.
//!
//! [`Shader::from_json`] accepts either, and also recognises the API's
//! `{"Error": "..."}` reply (what a private shader, a missing shader or a
//! bad key gets) so that case becomes a message instead of a parse failure.
//!
//! ## Tolerance
//!
//! Shadertoy's JSON has grown fields for fifteen years and is not
//! versioned, so nothing here rejects an unknown key, and everything not
//! needed for conversion is optional. What *is* needed (a pass's code, an
//! input's channel) is checked by the converter, which can say which pass
//! and which channel; a `serde` message cannot.
//!
//! Some values are stringly typed in the real data: `"vflip": "true"`,
//! input ids that are a string for a texture and a number for a buffer,
//! the input kind under `ctype` in the API and under `type` in some
//! exports, the asset path under `src` or `filepath`. The types below
//! absorb all of that so the converter sees one shape.

use serde::Deserialize;
use serde_json::Value;

/// Why some JSON is not a Shadertoy shader.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// Not valid JSON at all.
    #[error("not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Valid JSON that is not a shader.
    #[error("this is not a Shadertoy shader: {0}")]
    NotAShader(String),
    /// The API answered with an error instead of a shader.
    #[error("Shadertoy answered with an error: {0}")]
    Api(String),
    /// A shader whose fields do not have the expected types.
    #[error("the shader JSON has the wrong shape: {0}")]
    Shape(String),
}

/// A shader: its description and its passes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Shader {
    /// The export format version, when given.
    #[serde(default)]
    pub ver: Option<String>,
    /// Who made it and what it is called.
    #[serde(default)]
    pub info: Info,
    /// The passes, in the order the export lists them (not necessarily the
    /// order they run in).
    pub renderpass: Vec<RenderPass>,
}

/// A shader's description.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Info {
    /// The six-character id.
    #[serde(default, deserialize_with = "flexible_string")]
    pub id: Option<String>,
    /// The title.
    #[serde(default)]
    pub name: Option<String>,
    /// The author's user name.
    #[serde(default)]
    pub username: Option<String>,
    /// The author's description.
    #[serde(default)]
    pub description: Option<String>,
    /// Tags.
    #[serde(default, deserialize_with = "null_as_default")]
    pub tags: Vec<String>,
}

/// What a pass is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassKind {
    /// The pass that draws to the screen.
    Image,
    /// An off-screen buffer (A to D).
    Buffer,
    /// Code shared by every pass; renders nothing itself.
    Common,
    /// An audio-synthesis shader (`mainSound`).
    Sound,
    /// A cube-map buffer (`mainCubemap`).
    Cubemap,
    /// Something this version does not know, as written.
    Other(String),
}

impl PassKind {
    /// The spelling Shadertoy uses.
    pub fn name(&self) -> &str {
        match self {
            Self::Image => "image",
            Self::Buffer => "buffer",
            Self::Common => "common",
            Self::Sound => "sound",
            Self::Cubemap => "cubemap",
            Self::Other(other) => other,
        }
    }
}

impl<'de> Deserialize<'de> for PassKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(match text.as_str() {
            "image" => Self::Image,
            "buffer" => Self::Buffer,
            "common" => Self::Common,
            "sound" => Self::Sound,
            "cubemap" => Self::Cubemap,
            _ => Self::Other(text),
        })
    }
}

/// One pass: a tab in Shadertoy's editor.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RenderPass {
    /// What kind of pass.
    #[serde(rename = "type")]
    pub kind: PassKind,
    /// The tab's name: `Image`, `Buffer A`, `Common`.
    #[serde(default)]
    pub name: String,
    /// The GLSL.
    #[serde(default)]
    pub code: String,
    /// The four input channels that are connected.
    #[serde(default, deserialize_with = "null_as_default")]
    pub inputs: Vec<Input>,
    /// What the pass renders to. A buffer's output id is how other passes'
    /// `buffer` inputs refer to it.
    #[serde(default, deserialize_with = "null_as_default")]
    pub outputs: Vec<Output>,
}

/// What an input channel is connected to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKind {
    /// A 2D image.
    Texture,
    /// A cube map.
    Cubemap,
    /// A 3D texture.
    Volume,
    /// One of the buffers.
    Buffer,
    /// A video file.
    Video,
    /// The webcam.
    Webcam,
    /// The microphone.
    Mic,
    /// A music track.
    Music,
    /// A streamed music track (SoundCloud).
    MusicStream,
    /// The keyboard state texture.
    Keyboard,
    /// Something this version does not know, as written.
    Other(String),
}

impl InputKind {
    /// Reads the spelling Shadertoy uses; unknown spellings are kept.
    pub fn from_name(text: &str) -> Self {
        match text {
            "texture" => Self::Texture,
            "cubemap" => Self::Cubemap,
            "volume" => Self::Volume,
            "buffer" => Self::Buffer,
            "video" => Self::Video,
            "webcam" => Self::Webcam,
            "mic" => Self::Mic,
            "music" => Self::Music,
            "musicstream" => Self::MusicStream,
            "keyboard" => Self::Keyboard,
            other => Self::Other(other.to_string()),
        }
    }

    /// The spelling Shadertoy uses.
    pub fn name(&self) -> &str {
        match self {
            Self::Texture => "texture",
            Self::Cubemap => "cubemap",
            Self::Volume => "volume",
            Self::Buffer => "buffer",
            Self::Video => "video",
            Self::Webcam => "webcam",
            Self::Mic => "mic",
            Self::Music => "music",
            Self::MusicStream => "musicstream",
            Self::Keyboard => "keyboard",
            Self::Other(other) => other,
        }
    }
}

/// One connected input channel.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "RawInput")]
pub struct Input {
    /// The input's id. For a `buffer` input this is the output id of the
    /// buffer it reads.
    pub id: Option<String>,
    /// Channel number, 0 to 3.
    pub channel: u32,
    /// What it is connected to.
    pub kind: InputKind,
    /// The asset path (`/media/a/<hash>.jpg`) or a preview image for
    /// buffers, from `src` or `filepath`.
    pub src: Option<String>,
    /// How it is sampled.
    pub sampler: Sampler,
}

/// An input as written, before `ctype`/`type` and `src`/`filepath` are
/// reconciled.
#[derive(Debug, Deserialize)]
struct RawInput {
    #[serde(default, deserialize_with = "flexible_string")]
    id: Option<String>,
    #[serde(default)]
    channel: u32,
    #[serde(default)]
    ctype: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    src: Option<String>,
    #[serde(default)]
    filepath: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    sampler: Sampler,
}

impl From<RawInput> for Input {
    fn from(raw: RawInput) -> Self {
        let kind = raw.ctype.or(raw.kind).map_or_else(
            || InputKind::Other(String::new()),
            |k| InputKind::from_name(&k),
        );
        Self {
            id: raw.id,
            channel: raw.channel,
            kind,
            src: raw.src.or(raw.filepath),
            sampler: raw.sampler,
        }
    }
}

/// How an input is sampled.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Sampler {
    /// `nearest`, `linear` or `mipmap`.
    #[serde(default)]
    pub filter: Option<String>,
    /// `clamp` or `repeat`.
    #[serde(default)]
    pub wrap: Option<String>,
    /// Flip vertically on load.
    #[serde(default, deserialize_with = "flexible_bool")]
    pub vflip: Option<bool>,
    /// Decode from sRGB on sampling.
    #[serde(default, deserialize_with = "flexible_bool")]
    pub srgb: Option<bool>,
    /// `byte` or `float` storage.
    #[serde(default)]
    pub internal: Option<String>,
}

/// One output of a pass.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Output {
    /// The output's id, referenced by `buffer` inputs.
    #[serde(default, deserialize_with = "flexible_string")]
    pub id: Option<String>,
    /// Channel number.
    #[serde(default)]
    pub channel: u32,
}

impl Shader {
    /// Parses a shader from JSON text, in either the API shape or the bare
    /// export shape.
    pub fn from_json(text: &str) -> Result<Self, ModelError> {
        let value: Value = serde_json::from_str(text)?;
        Self::from_value(value)
    }

    /// [`Self::from_json`] for an already parsed value.
    pub fn from_value(value: Value) -> Result<Self, ModelError> {
        let value = match value {
            Value::Object(mut map) => {
                if let Some(error) = map.get("Error") {
                    let message = error
                        .as_str()
                        .map_or_else(|| error.to_string(), str::to_string);
                    return Err(ModelError::Api(message));
                }
                match map.remove("Shader") {
                    Some(inner) => inner,
                    None => Value::Object(map),
                }
            }
            Value::Array(mut items) if items.len() == 1 => {
                // A one-element list of shaders (some exporters wrap this
                // way); `Shader` inside it is unwrapped like the API's.
                let only = items.remove(0);
                return Self::from_value(only);
            }
            Value::Array(items) => {
                return Err(ModelError::NotAShader(format!(
                    "a list of {} items; import one shader at a time",
                    items.len()
                )));
            }
            other => {
                return Err(ModelError::NotAShader(format!(
                    "expected an object, found {}",
                    kind_of(&other)
                )));
            }
        };
        if value.get("renderpass").is_none() {
            return Err(ModelError::NotAShader(
                "no `renderpass` list (neither `{\"Shader\": ...}` nor a bare export)".to_string(),
            ));
        }
        serde_json::from_value(value).map_err(|e| ModelError::Shape(e.to_string()))
    }
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// A string, or a number written as one: Shadertoy ids are `"4dXGRr"` for
/// textures and `257` for buffers.
fn flexible_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Text {
        Text(String),
        Number(serde_json::Number),
    }
    Ok(
        Option::<Text>::deserialize(deserializer)?.map(|text| match text {
            Text::Text(text) => text,
            Text::Number(number) => number.to_string(),
        }),
    )
}

/// A boolean, or `"true"` / `"false"` (also `"1"` / `"0"`), which is how
/// sampler flags are written.
fn flexible_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Flag {
        Bool(bool),
        Text(String),
        Number(i64),
    }
    Option::<Flag>::deserialize(deserializer)?
        .map(|flag| match flag {
            Flag::Bool(value) => Ok(value),
            Flag::Number(0) => Ok(false),
            Flag::Number(_) => Ok(true),
            Flag::Text(text) => match text.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => Ok(true),
                "false" | "0" | "no" | "" => Ok(false),
                _ => Err(serde::de::Error::custom(format!(
                    "`{text}` is not a boolean"
                ))),
            },
        })
        .transpose()
}

/// `null` where a list or object is expected reads as empty.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    const BARE: &str = r#"{
        "ver": "0.1",
        "info": { "id": "XsXXDn", "name": "Tunnel", "username": "someone",
                  "description": null, "tags": ["tunnel", "demo"], "viewed": 12 },
        "renderpass": [
            { "type": "image", "name": "Image", "code": "void mainImage(out vec4 c, in vec2 p) { c = vec4(1.0); }",
              "inputs": [
                { "id": "4dXGRr", "channel": 0, "ctype": "texture",
                  "src": "/media/a/abc.jpg",
                  "sampler": { "filter": "mipmap", "wrap": "repeat", "vflip": "true",
                               "srgb": "false", "internal": "byte" },
                  "published": 1 },
                { "id": 257, "channel": 1, "type": "buffer", "filepath": "/media/previz/buffer00.png",
                  "sampler": { "filter": "linear", "wrap": "clamp", "vflip": false, "srgb": "false" } }
              ],
              "outputs": [ { "id": "4dfGRr", "channel": 0 } ],
              "somethingNew": [1, 2, 3] }
        ],
        "futureKey": {}
    }"#;

    #[test]
    fn the_bare_export_shape_parses() -> Result<()> {
        let shader = Shader::from_json(BARE)?;
        assert_eq!(shader.info.id.as_deref(), Some("XsXXDn"), "id");
        assert_eq!(shader.info.tags, vec!["tunnel", "demo"], "tags");
        assert_eq!(shader.info.description, None, "null description");
        let pass = &shader.renderpass[0];
        assert_eq!(pass.kind, PassKind::Image, "pass type");
        assert_eq!(pass.inputs.len(), 2, "both inputs");
        Ok(())
    }

    #[test]
    fn the_api_wrapper_gives_the_same_shader() -> Result<()> {
        let wrapped = format!(r#"{{ "Shader": {BARE} }}"#);
        assert_eq!(
            Shader::from_json(&wrapped)?,
            Shader::from_json(BARE)?,
            "the wrapper must not change the data"
        );
        Ok(())
    }

    #[test]
    fn stringly_typed_fields_are_normalised() -> Result<()> {
        let shader = Shader::from_json(BARE)?;
        let [texture, buffer] = [
            &shader.renderpass[0].inputs[0],
            &shader.renderpass[0].inputs[1],
        ];
        assert_eq!(texture.kind, InputKind::Texture, "ctype");
        assert_eq!(texture.sampler.vflip, Some(true), "\"true\" is a flag");
        assert_eq!(texture.sampler.srgb, Some(false), "\"false\" is a flag");
        assert_eq!(
            buffer.kind,
            InputKind::Buffer,
            "`type` is accepted for ctype"
        );
        assert_eq!(buffer.id.as_deref(), Some("257"), "numeric ids become text");
        assert_eq!(
            buffer.src.as_deref(),
            Some("/media/previz/buffer00.png"),
            "`filepath` is accepted for src"
        );
        assert_eq!(buffer.sampler.vflip, Some(false), "a real boolean too");
        Ok(())
    }

    #[test]
    fn unknown_kinds_are_kept_not_rejected() -> Result<()> {
        let shader = Shader::from_json(
            r#"{ "renderpass": [ { "type": "hologram", "code": "",
                 "inputs": [ { "channel": 2, "ctype": "smell" } ] } ] }"#,
        )?;
        assert_eq!(
            shader.renderpass[0].kind,
            PassKind::Other("hologram".to_string()),
            "unknown pass type"
        );
        assert_eq!(
            shader.renderpass[0].inputs[0].kind,
            InputKind::Other("smell".to_string()),
            "unknown input type"
        );
        Ok(())
    }

    #[test]
    fn an_api_error_reply_becomes_its_message() {
        let error = Shader::from_json(r#"{ "Error": "Shader not found" }"#)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(error.contains("Shader not found"), "got: {error}");
    }

    #[test]
    fn malformed_json_is_a_typed_error_with_a_position() {
        let result = Shader::from_json("{ \"renderpass\": [ ");
        assert!(
            matches!(&result, Err(ModelError::Json(inner)) if inner.is_eof()),
            "truncated input is a JSON EOF error, got {result:?}"
        );
    }

    #[test]
    fn things_that_are_not_shaders_say_so() {
        for (text, needle) in [
            ("[1, 2]", "list of 2"),
            ("42", "found a number"),
            (r#"{ "info": {} }"#, "renderpass"),
            (r#"{ "renderpass": 5 }"#, "wrong shape"),
        ] {
            let message = Shader::from_json(text)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert!(message.contains(needle), "`{text}` -> {message}");
        }
    }

    #[test]
    fn a_one_element_list_is_unwrapped() -> Result<()> {
        let wrapped = format!("[{BARE}]");
        assert_eq!(
            Shader::from_json(&wrapped)?,
            Shader::from_json(BARE)?,
            "list of one"
        );
        Ok(())
    }
}
