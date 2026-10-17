// ============================================================================
// Module:       strobe_shadertoy::import
// Description:  The import pipeline every route shares: JSON text in, scene
//               folder in the library out, with textures fetched on the way.
//
// Dependencies: crate::{model, convert, acquire, id}
// ============================================================================

//! The import pipeline.
//!
//! A dropped file, a pasted id, the browser userscript and the command line
//! all end in [`import_json`]:
//!
//! ```text
//! JSON text -> parse -> convert (no textures) -> already there? -> download
//!           -> convert again with the textures -> write atomically
//! ```
//!
//! It is plain blocking code with no GL, so a caller runs it on a worker
//! thread and shader compilation stays on the thread that owns the context.
//!
//! ## Converting twice
//!
//! The converter is pure and fast, and it is the only thing that knows which
//! textures a shader wants (`Converted::pending_assets`). So the first pass
//! learns the list, the downloader fetches it, and the second pass is the
//! same conversion with `Options::assets` filled. Nothing is written
//! between the two.
//!
//! ## Re-importing
//!
//! An existing scene folder is **not** replaced unless `replace` is set; the
//! caller gets [`ImportError::Exists`] naming the folder, **before** any
//! download, and decides (the app asks; `strobe-import --force` and the
//! userscript's "Replace?" prompt pass `replace`). The old folder is kept
//! until the new one is complete (the writer swaps them in one rename), so
//! a failed re-import leaves the old scene intact (D113).
//!
//! ## What is reported
//!
//! A converted-with-loss shader is a success with [`Imported::warnings`];
//! a texture that could not be fetched is a warning that says why. Only a
//! shader that cannot be converted at all, or a library that cannot be
//! written, is an error.

use crate::acquire::{
    ApiKey, AssetCache, Endpoints, Fetch, KeyError, RestError, download, fetch_shader_json,
};
use crate::convert::{self, ConvertError, Options, Warning, WriteError};
use crate::id::{IdError, ShaderId};
use crate::model::{ModelError, Shader};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The largest JSON file read from disk.
pub const MAX_JSON_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// How textures are obtained during an import.
pub struct Downloader<'a> {
    /// The HTTP client.
    pub fetch: &'a dyn Fetch,
    /// Shadertoy's addresses.
    pub endpoints: &'a Endpoints,
    /// Where downloaded textures are kept.
    pub cache: &'a AssetCache,
}

/// What to import and where to put it.
pub struct ImportOptions<'a> {
    /// The scene library folder.
    pub library: &'a Path,
    /// Replace a scene of the same name instead of failing.
    pub replace: bool,
    /// The scene's title (the shader's own name when `None`).
    pub name: Option<String>,
    /// Textures the caller already has, by file name.
    pub local_assets: BTreeMap<String, Vec<u8>>,
    /// Fail when a texture is still missing after downloading, instead of
    /// binding black and warning.
    pub strict_assets: bool,
    /// Fetch missing textures, or leave them black.
    pub downloader: Option<Downloader<'a>>,
}

impl<'a> ImportOptions<'a> {
    /// Options that write into `library` and fetch nothing.
    pub fn offline(library: &'a Path) -> Self {
        Self {
            library,
            replace: false,
            name: None,
            local_assets: BTreeMap::new(),
            strict_assets: false,
            downloader: None,
        }
    }
}

/// A scene that was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// The scene folder.
    pub dir: PathBuf,
    /// The folder's name (`name-ID.synScene`).
    pub folder: String,
    /// The scene's title.
    pub title: String,
    /// The Shadertoy id, when the JSON carried one.
    pub shader_id: Option<String>,
    /// Whether an existing scene was replaced.
    pub replaced: bool,
    /// Files written.
    pub files: usize,
    /// Textures fetched or found in the cache.
    pub textures_found: usize,
    /// Textures the scene still reads black (listed in
    /// `images/pending-assets.json`).
    pub textures_missing: usize,
    /// Things converted with a loss, and textures that failed, as sentences.
    pub warnings: Vec<String>,
}

/// Why nothing was written.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The text is not a Shadertoy shader.
    #[error("{0}")]
    NotAShader(#[from] ModelError),
    /// The shader cannot be converted.
    #[error("{0}")]
    Convert(#[from] ConvertError),
    /// The scene is already in the library and `replace` was not set.
    #[error("\"{title}\" is already in the library ({folder})")]
    Exists {
        /// The existing folder.
        dir: PathBuf,
        /// Its name.
        folder: String,
        /// The scene's title.
        title: String,
    },
    /// The folder could not be written.
    #[error("{0}")]
    Write(#[from] WriteError),
    /// The file could not be read.
    #[error("cannot read {path}: {reason}")]
    Read {
        /// The file.
        path: String,
        /// Why.
        reason: String,
    },
    /// The id could not be fetched (no key, no network, refused).
    #[error("{0}")]
    Fetch(#[from] RestError),
    /// There is no API key.
    #[error("{0}")]
    Key(#[from] KeyError),
    /// The input is neither a file nor a shader id.
    #[error("{0}")]
    Input(String),
}

/// Imports the shader in `text`. See the module docs for the steps.
pub fn import_json(text: &str, options: &ImportOptions<'_>) -> Result<Imported, ImportError> {
    let shader = Shader::from_json(text)?;
    let convert_options = |assets: BTreeMap<String, Vec<u8>>, strict: bool| Options {
        name: options.name.clone(),
        assets,
        strict_assets: strict,
        original_json: Some(text.to_string()),
    };
    let mut converted = convert::convert(
        &shader,
        &convert_options(options.local_assets.clone(), false),
    )?;

    let target = options.library.join(&converted.folder_name);
    let existed = target.exists();
    if existed && !options.replace {
        return Err(ImportError::Exists {
            dir: target,
            folder: converted.folder_name,
            title: converted.title,
        });
    }

    let mut failures = Vec::new();
    let mut assets = options.local_assets.clone();
    if let (false, Some(downloader)) = (converted.pending_assets.is_empty(), &options.downloader) {
        let got = download(
            downloader.fetch,
            downloader.endpoints,
            downloader.cache,
            &converted.pending_assets,
        );
        failures = got.failures;
        assets.extend(got.assets);
    }
    // Convert again when there is something new to bind, or to enforce
    // `strict_assets` against what is still missing.
    if assets.len() > options.local_assets.len() || options.strict_assets {
        converted = convert::convert(&shader, &convert_options(assets, options.strict_assets))?;
    }

    let dir = convert::write(&converted, options.library, options.replace)?;

    let mut warnings: Vec<String> = converted
        .warnings
        .iter()
        .filter(|warning| match warning {
            // A texture that failed to download is described by its
            // failure, which says why.
            Warning::MissingAsset(asset) => !failures.iter().any(|f| f.asset.file == asset.file),
            Warning::InputBoundToBlack { .. }
            | Warning::UnboundChannelRead { .. }
            | Warning::SamplerConflict { .. }
            | Warning::UnknownSamplerValue { .. }
            | Warning::SrgbNotDecoded { .. } => true,
        })
        .map(ToString::to_string)
        .collect();
    for failure in &failures {
        warnings.push(format!(
            "{} channel {}: texture {} was not downloaded ({}); it reads black. \
             images/pending-assets.json lists it",
            failure.asset.pass, failure.asset.channel, failure.asset.file, failure.reason
        ));
    }

    // Texture files in the scene: everything under images/ that is not the
    // placeholder or the pending list. A texture is never named like either
    // (D417), so the names are compared whole: a texture named after one
    // is `tex-st_black.png`, which still counts.
    let textures_found = converted
        .files
        .keys()
        .filter(|path| {
            path.starts_with("images/")
                && path.as_str() != "images/st_black.png"
                && path.as_str() != "images/pending-assets.json"
        })
        .count();

    Ok(Imported {
        dir,
        folder: converted.folder_name,
        title: converted.title,
        shader_id: shader.info.id.clone(),
        replaced: existed,
        files: converted.files.len(),
        textures_found,
        textures_missing: converted.pending_assets.len(),
        warnings,
    })
}

/// Reads `path` (a Shadertoy export) and imports it.
pub fn import_file(path: &Path, options: &ImportOptions<'_>) -> Result<Imported, ImportError> {
    let read = |reason: String| ImportError::Read {
        path: path.display().to_string(),
        reason,
    };
    let meta = std::fs::metadata(path).map_err(|e| read(e.to_string()))?;
    if meta.len() > MAX_JSON_FILE_BYTES {
        return Err(read(format!(
            "the file is {} bytes; a shader export is far smaller than {MAX_JSON_FILE_BYTES}",
            meta.len()
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| read(e.to_string()))?;
    let text = decode_text(bytes).map_err(read)?;
    import_json(&text, options)
}

/// A JSON file's text: UTF-8 (a BOM is left for [`Shader::from_json`] to
/// skip), or UTF-16 when it starts with a little- or big-endian byte-order
/// mark, as PowerShell 5's `>` redirect and `Out-File` write it (D416).
fn decode_text(bytes: Vec<u8>) -> Result<String, String> {
    let big_endian = match bytes.get(..2) {
        Some([0xff, 0xfe]) => false,
        Some([0xfe, 0xff]) => true,
        Some(_) | None => {
            return String::from_utf8(bytes).map_err(|_| {
                "the file is not UTF-8 text (save it as UTF-8, or as UTF-16 with a byte-order \
                 mark)"
                    .to_string()
            });
        }
    };
    let body = bytes.get(2..).unwrap_or_default();
    let (units, rest) = body.as_chunks::<2>();
    if !rest.is_empty() {
        return Err("the file starts as UTF-16 but has an odd number of bytes".to_string());
    }
    let units = units.iter().map(|pair| {
        if big_endian {
            u16::from_be_bytes(*pair)
        } else {
            u16::from_le_bytes(*pair)
        }
    });
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|error| format!("the file starts as UTF-16 but is not valid UTF-16: {error}"))
}

/// Fetches shader `id` from the REST API and imports it.
pub fn import_id(
    id: &ShaderId,
    key: &ApiKey,
    fetch: &dyn Fetch,
    endpoints: &Endpoints,
    options: &ImportOptions<'_>,
) -> Result<Imported, ImportError> {
    let text = fetch_shader_json(fetch, endpoints, id, key)?;
    import_json(&text, options)
}

/// What a typed or dropped string refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A JSON file on disk.
    File(PathBuf),
    /// A shader on Shadertoy.
    Id(ShaderId),
}

impl Source {
    /// Classifies `text`: an existing file wins (so a file called `XsXXDn`
    /// is a file), then a Shadertoy id or URL. Surrounding quotes are
    /// dropped, as Windows' "Copy as path" adds them.
    pub fn classify(text: &str) -> Result<Self, ImportError> {
        let text = text.trim().trim_matches(['"', '\'']).trim();
        if text.is_empty() {
            return Err(ImportError::Input(
                "type a Shadertoy id or URL, or the path of an exported .json file".to_string(),
            ));
        }
        let path = Path::new(text);
        if path.is_file() {
            return Ok(Self::File(path.to_path_buf()));
        }
        match ShaderId::parse(text) {
            Ok(id) => Ok(Self::Id(id)),
            Err(IdError::BadId(_) | IdError::WrongHost(_) | IdError::NotAShaderPage(_))
                if looks_like_path(text) =>
            {
                Err(ImportError::Input(format!("no file at {text}")))
            }
            Err(error) => Err(ImportError::Input(format!(
                "`{text}` is not a file, and not a Shadertoy id or URL: {error}"
            ))),
        }
    }
}

/// Whether `text` reads as a file path rather than an id or a link.
fn looks_like_path(text: &str) -> bool {
    !text.contains("://")
        && (text.ends_with(".json")
            || text.contains(std::path::MAIN_SEPARATOR)
            || text.contains('\\'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::cell::RefCell;

    use crate::acquire::{FetchError, Limits};

    const SINGLE: &str = include_str!("../tests/fixtures/single_image.json");
    const MISSING_TEXTURE: &str = include_str!("../tests/fixtures/missing_texture.json");

    fn scratch(name: &str) -> Result<PathBuf> {
        let dir = std::env::temp_dir().join(format!("strobe-import-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    struct Fake {
        answer: Result<Vec<u8>, FetchError>,
        hits: RefCell<usize>,
    }

    impl Fetch for Fake {
        fn get(&self, _url: &str, _limits: Limits) -> Result<Vec<u8>, FetchError> {
            *self.hits.borrow_mut() += 1;
            self.answer.clone()
        }
    }

    #[test]
    fn a_json_import_writes_the_scene_and_reports_it() -> Result<()> {
        let library = scratch("write")?;
        let done = import_json(SINGLE, &ImportOptions::offline(&library))?;
        assert!(done.dir.join("scene.json").is_file(), "scene.json written");
        assert!(done.dir.join("main.glsl").is_file(), "main.glsl written");
        assert!(done.folder.ends_with(".synScene"), "folder {}", done.folder);
        assert!(!done.replaced, "a new scene");
        assert!(done.files >= 3, "files {}", done.files);
        std::fs::remove_dir_all(&library)?;
        Ok(())
    }

    #[test]
    fn a_second_import_asks_and_replace_swaps_it() -> Result<()> {
        let library = scratch("again")?;
        import_json(SINGLE, &ImportOptions::offline(&library))?;
        let second = import_json(SINGLE, &ImportOptions::offline(&library));
        assert!(
            matches!(&second, Err(ImportError::Exists { title, .. }) if !title.is_empty()),
            "must ask first: {second:?}"
        );
        let replaced = import_json(
            SINGLE,
            &ImportOptions {
                replace: true,
                ..ImportOptions::offline(&library)
            },
        )?;
        assert!(replaced.replaced, "replaced flag");
        std::fs::remove_dir_all(&library)?;
        Ok(())
    }

    #[test]
    fn not_json_and_not_a_shader_are_distinct_errors() -> Result<()> {
        let library = scratch("bad")?;
        let options = ImportOptions::offline(&library);
        for (text, needle) in [
            ("this is not json", "not valid JSON"),
            (r#"{"hello": 1}"#, "not a Shadertoy shader"),
            (r#"{"Error": "Shader not found"}"#, "Shader not found"),
        ] {
            let error = import_json(text, &options)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert!(error.contains(needle), "`{text}` -> `{error}`");
        }
        assert_eq!(
            std::fs::read_dir(&library)?.count(),
            0,
            "a failed import leaves the library untouched"
        );
        std::fs::remove_dir_all(&library)?;
        Ok(())
    }

    #[test]
    fn a_missing_texture_that_cannot_be_downloaded_is_a_visible_warning() -> Result<()> {
        let library = scratch("tex")?;
        let cache = scratch("tex-cache")?;
        let fetch = Fake {
            answer: Err(FetchError::Status { status: 403 }),
            hits: RefCell::new(0),
        };
        let endpoints = Endpoints::new("http://mock.test");
        let cache = AssetCache::new(&cache);
        let done = import_json(
            MISSING_TEXTURE,
            &ImportOptions {
                downloader: Some(Downloader {
                    fetch: &fetch,
                    endpoints: &endpoints,
                    cache: &cache,
                }),
                ..ImportOptions::offline(&library)
            },
        )?;
        assert!(*fetch.hits.borrow() >= 1, "it tried");
        assert_eq!(done.textures_missing, 1, "one texture still missing");
        assert!(
            done.warnings
                .iter()
                .any(|w| w.contains("not downloaded") && w.contains("403")),
            "{:?}",
            done.warnings
        );
        assert!(
            done.dir.join("images/pending-assets.json").is_file(),
            "the pending list is written"
        );
        std::fs::remove_dir_all(&library)?;
        Ok(())
    }

    #[test]
    fn a_texture_that_downloads_is_bound_and_no_longer_pending() -> Result<()> {
        let library = scratch("tex-ok")?;
        let cache_dir = scratch("tex-ok-cache")?;
        // A valid 1x1 PNG so the converter and the player accept it.
        let png: Vec<u8> = vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78,
            0xda, 0x63, 0x60, 0x60, 0x60, 0xf8, 0x0f, 0x00, 0x01, 0x04, 0x01, 0x00, 0x80, 0xbb,
            0xd1, 0x5b, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        let fetch = Fake {
            answer: Ok(png),
            hits: RefCell::new(0),
        };
        let endpoints = Endpoints::new("http://mock.test");
        let cache = AssetCache::new(&cache_dir);
        let done = import_json(
            MISSING_TEXTURE,
            &ImportOptions {
                downloader: Some(Downloader {
                    fetch: &fetch,
                    endpoints: &endpoints,
                    cache: &cache,
                }),
                ..ImportOptions::offline(&library)
            },
        )?;
        assert_eq!(done.textures_missing, 0, "nothing pending");
        assert_eq!(done.textures_found, 1, "one texture found");
        assert!(
            !done.dir.join("images/pending-assets.json").exists(),
            "no pending list"
        );
        std::fs::remove_dir_all(&library)?;
        std::fs::remove_dir_all(&cache_dir)?;
        Ok(())
    }

    #[test]
    fn re_importing_asks_before_it_downloads() -> Result<()> {
        let library = scratch("ask-first")?;
        import_json(MISSING_TEXTURE, &ImportOptions::offline(&library))?;
        let cache_dir = scratch("ask-first-cache")?;
        let fetch = Fake {
            answer: Ok(vec![1]),
            hits: RefCell::new(0),
        };
        let endpoints = Endpoints::new("http://mock.test");
        let cache = AssetCache::new(&cache_dir);
        let again = import_json(
            MISSING_TEXTURE,
            &ImportOptions {
                downloader: Some(Downloader {
                    fetch: &fetch,
                    endpoints: &endpoints,
                    cache: &cache,
                }),
                ..ImportOptions::offline(&library)
            },
        );
        assert!(
            matches!(again, Err(ImportError::Exists { .. })),
            "{again:?}"
        );
        assert_eq!(*fetch.hits.borrow(), 0, "no request before the answer");
        std::fs::remove_dir_all(&library)?;
        std::fs::remove_dir_all(&cache_dir)?;
        Ok(())
    }

    #[test]
    fn a_utf16_export_with_a_byte_order_mark_imports() -> Result<()> {
        // PowerShell 5's `>` and `Out-File` write UTF-16 LE with a BOM; some
        // Windows tools write the big-endian form.
        let dir = scratch("utf16")?;
        for (name, big_endian) in [("le.json", false), ("be.json", true)] {
            let mut bytes: Vec<u8> = if big_endian {
                vec![0xfe, 0xff]
            } else {
                vec![0xff, 0xfe]
            };
            for unit in SINGLE.encode_utf16() {
                let pair = if big_endian {
                    unit.to_be_bytes()
                } else {
                    unit.to_le_bytes()
                };
                bytes.extend_from_slice(&pair);
            }
            let file = dir.join(name);
            std::fs::write(&file, &bytes)?;
            let library = dir.join(format!("library-{name}"));
            let done = import_file(&file, &ImportOptions::offline(&library))?;
            assert!(done.dir.join("main.glsl").is_file(), "{name}: imported");
        }
        // Half a code unit, and a lone surrogate, are not text.
        for (name, bytes) in [
            ("odd.json", vec![0xff, 0xfe, 0x7b]),
            ("lone.json", vec![0xff, 0xfe, 0x00, 0xd8]),
        ] {
            let file = dir.join(name);
            std::fs::write(&file, bytes)?;
            let error = import_file(&file, &ImportOptions::offline(&dir.join("unused")))
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert!(error.contains("UTF-16"), "{name}: `{error}`");
        }
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn input_is_classified_as_a_file_an_id_or_an_error() -> Result<()> {
        let dir = scratch("classify")?;
        let file = dir.join("shader.json");
        std::fs::write(&file, SINGLE)?;
        assert_eq!(
            Source::classify(&format!("\"{}\"", file.display()))?,
            Source::File(file.clone()),
            "quoted path"
        );
        assert!(
            matches!(
                Source::classify("https://www.shadertoy.com/view/XsXXDn")?,
                Source::Id(_)
            ),
            "url"
        );
        assert!(
            matches!(Source::classify("XsXXDn")?, Source::Id(_)),
            "bare id"
        );
        for bad in [
            "",
            "   ",
            "nonsense words",
            "missing/file.json",
            "C:\\none.json",
        ] {
            assert!(Source::classify(bad).is_err(), "`{bad}` must be refused");
        }
        let message = Source::classify("missing/file.json")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(message.contains("no file at"), "path-like input: {message}");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
