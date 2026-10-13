// ============================================================================
// Module:       strobe_shadertoy::acquire::assets
// Description:  The asset downloader and its SHA-256-keyed on-disk cache:
//               fetches the textures a converted scene still lacks.
//
// Dependencies: super::fetch; crate::convert::PendingAsset; sha2
// ============================================================================

//! Texture downloads.
//!
//! The converter lists the textures a scene wants and was not given
//! (`Converted::pending_assets`, written to `images/pending-assets.json`).
//! [`download`] fetches them so a second conversion can include the files.
//!
//! ## The cache
//!
//! [`AssetCache`] keeps what was downloaded under a data-directory folder,
//! one file per URL named by the **SHA-256 of the URL** in lowercase hex. A
//! texture shared by a hundred shaders (`/media/a/<hash>.jpg`) is downloaded
//! once. The name is derived from the URL, not the bytes, so a cache hit
//! needs no network round trip and no hashing of the content; and since the
//! name is hex, a URL can never name a path outside the cache. A file is
//! written to a temporary name and renamed into place, so an interrupted
//! download leaves nothing the cache would mistake for a texture.
//!
//! ## Limits
//!
//! Each texture has [`ASSET_TIMEOUT`] (10 s) and [`MAX_ASSET_BYTES`] (16 MiB).
//! If the host cannot be reached at all, the remaining textures are not
//! attempted: four 10-second timeouts in a row help nobody. A texture that
//! fails is a [`DownloadFailure`] and the scene it belongs to reads black
//! there, with the converter's usual warning; it never fails the import.
//!
//! A response that is HTML (a challenge page served with status 200) or
//! empty is refused, so the cache never holds a page named `x.jpg`.

use super::fetch::{ASSET_TIMEOUT, Endpoints, Fetch, Limits, MAX_ASSET_BYTES};
use crate::convert::PendingAsset;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The cache folder's name inside the data directory.
pub const CACHE_DIR: &str = "shadertoy-cache";

/// Lowercase hex SHA-256 of `text`.
pub fn sha256_hex(text: &[u8]) -> String {
    let digest = Sha256::digest(text);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Writing to a String cannot fail.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// A folder of downloaded textures, named by the hash of their URL.
#[derive(Debug, Clone)]
pub struct AssetCache {
    dir: PathBuf,
}

impl AssetCache {
    /// A cache in `dir` (created on the first write).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The cache inside the data directory `data_dir`.
    pub fn in_data_dir(data_dir: &Path) -> Self {
        Self::new(data_dir.join(CACHE_DIR))
    }

    /// The file `url` is cached in.
    pub fn path_for(&self, url: &str) -> PathBuf {
        self.dir.join(sha256_hex(url.as_bytes()))
    }

    /// The cached bytes for `url`, if present and non-empty.
    pub fn get(&self, url: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path_for(url))
            .ok()
            .filter(|bytes| !bytes.is_empty())
    }

    /// Stores `bytes` for `url`, atomically.
    pub fn put(&self, url: &str, bytes: &[u8]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path_for(url);
        let temporary = path.with_extension(format!("part-{}", std::process::id()));
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(&temporary, &path).inspect_err(|_| {
            // Best effort: the error being returned is the one that matters.
            let _ = std::fs::remove_file(&temporary);
        })
    }
}

/// A texture that could not be fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadFailure {
    /// The asset.
    pub asset: PendingAsset,
    /// Why, in words for the console.
    pub reason: String,
}

/// What [`download`] got.
#[derive(Debug, Default)]
pub struct Downloaded {
    /// Texture bytes by file name (`Options::assets`' shape).
    pub assets: BTreeMap<String, Vec<u8>>,
    /// Textures served from the cache without a request.
    pub from_cache: usize,
    /// Textures that could not be had.
    pub failures: Vec<DownloadFailure>,
}

/// Fetches every asset in `pending` (once per file name), consulting `cache`
/// first and filling it after.
pub fn download(
    fetch: &dyn Fetch,
    endpoints: &Endpoints,
    cache: &AssetCache,
    pending: &[PendingAsset],
) -> Downloaded {
    let mut out = Downloaded::default();
    let mut unreachable: Option<String> = None;
    for asset in pending {
        if out.assets.contains_key(&asset.file)
            || out.failures.iter().any(|f| f.asset.file == asset.file)
        {
            continue;
        }
        let fail = |reason: String, out: &mut Downloaded| {
            out.failures.push(DownloadFailure {
                asset: asset.clone(),
                reason,
            });
        };
        let target = match endpoints.asset_url(&asset.url) {
            Ok(target) => target,
            Err(reason) => {
                fail(reason, &mut out);
                continue;
            }
        };
        if let Some(bytes) = cache.get(&asset.url) {
            out.from_cache += 1;
            out.assets.insert(asset.file.clone(), bytes);
            continue;
        }
        if let Some(why) = &unreachable {
            fail(format!("not attempted: {why}"), &mut out);
            continue;
        }
        match fetch.get(
            &target,
            Limits {
                max_bytes: MAX_ASSET_BYTES,
                timeout: ASSET_TIMEOUT,
            },
        ) {
            Ok(bytes) => match check(&bytes) {
                Ok(()) => {
                    if let Err(error) = cache.put(&asset.url, &bytes) {
                        log::warn!("texture cache: could not store {}: {error}", asset.file);
                    }
                    out.assets.insert(asset.file.clone(), bytes);
                }
                Err(reason) => fail(reason, &mut out),
            },
            Err(error) => {
                if error.host_unreachable() {
                    unreachable = Some(error.to_string());
                }
                fail(error.to_string(), &mut out);
            }
        }
    }
    out
}

/// Whether `bytes` could be a texture: not empty, not a web page.
fn check(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("the response was empty".to_string());
    }
    let head = bytes.get(..bytes.len().min(64)).unwrap_or_default();
    let lowered: Vec<u8> = head.iter().map(u8::to_ascii_lowercase).collect();
    let text = String::from_utf8_lossy(&lowered);
    let text = text.trim_start();
    if text.starts_with("<!doctype") || text.starts_with("<html") || text.starts_with('<') {
        return Err("the response is a web page, not an image (a bot check?)".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::fetch::FetchError;
    use super::*;
    use anyhow::Result;
    use std::cell::RefCell;

    /// Answers by URL suffix and counts requests.
    struct Server {
        routes: Vec<(&'static str, Result<Vec<u8>, FetchError>)>,
        hits: RefCell<Vec<String>>,
    }

    impl Fetch for Server {
        fn get(&self, url: &str, _limits: Limits) -> Result<Vec<u8>, FetchError> {
            self.hits.borrow_mut().push(url.to_string());
            self.routes
                .iter()
                .find(|(suffix, _)| url.ends_with(suffix))
                .map_or(Err(FetchError::Status { status: 404 }), |(_, r)| r.clone())
        }
    }

    fn asset(file: &str, path: &str) -> PendingAsset {
        PendingAsset {
            pass: "Image".to_string(),
            channel: 0,
            file: file.to_string(),
            url: format!("https://www.shadertoy.com{path}"),
        }
    }

    fn scratch(name: &str) -> Result<PathBuf> {
        let dir = std::env::temp_dir().join(format!("strobe-assets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[test]
    fn the_known_sha256_vectors_hold() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "empty"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "abc"
        );
    }

    #[test]
    fn a_download_fills_the_cache_and_the_second_run_makes_no_request() -> Result<()> {
        let dir = scratch("cache")?;
        let cache = AssetCache::new(&dir);
        let endpoints = Endpoints::new("http://mock.test");
        let server = Server {
            routes: vec![("/media/a/x.jpg", Ok(vec![1, 2, 3]))],
            hits: RefCell::new(Vec::new()),
        };
        let pending = [asset("x.jpg", "/media/a/x.jpg")];

        let first = download(&server, &endpoints, &cache, &pending);
        assert_eq!(first.assets.get("x.jpg"), Some(&vec![1, 2, 3]), "bytes");
        assert_eq!(server.hits.borrow().len(), 1, "one request");
        assert_eq!(
            server.hits.borrow().first().map(String::as_str),
            Some("http://mock.test/media/a/x.jpg"),
            "rebased onto the endpoint"
        );

        let again = download(&server, &endpoints, &cache, &pending);
        assert_eq!(again.from_cache, 1, "served from the cache");
        assert_eq!(server.hits.borrow().len(), 1, "no second request");
        assert_eq!(
            again.assets.get("x.jpg"),
            Some(&vec![1, 2, 3]),
            "same bytes"
        );

        // The cache file is named by the hash of the URL.
        let name = sha256_hex(b"https://www.shadertoy.com/media/a/x.jpg");
        assert!(dir.join(&name).is_file(), "cache file {name}");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_failed_texture_is_reported_and_the_others_still_arrive() -> Result<()> {
        let dir = scratch("partial")?;
        let server = Server {
            routes: vec![
                ("/media/a/ok.png", Ok(vec![9])),
                ("/media/a/bad.png", Err(FetchError::Status { status: 404 })),
            ],
            hits: RefCell::new(Vec::new()),
        };
        let pending = [
            asset("bad.png", "/media/a/bad.png"),
            asset("ok.png", "/media/a/ok.png"),
        ];
        let got = download(
            &server,
            &Endpoints::new("http://mock.test"),
            &AssetCache::new(&dir),
            &pending,
        );
        assert!(got.assets.contains_key("ok.png"), "the good one arrived");
        assert_eq!(got.failures.len(), 1, "one failure");
        assert!(
            got.failures
                .first()
                .is_some_and(|f| f.asset.file == "bad.png" && f.reason.contains("404")),
            "names the file and the status: {:?}",
            got.failures
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn an_unreachable_host_stops_the_remaining_requests() -> Result<()> {
        let dir = scratch("down")?;
        let server = Server {
            routes: vec![("", Err(FetchError::TimedOut(10)))],
            hits: RefCell::new(Vec::new()),
        };
        let pending = [
            asset("a.jpg", "/media/a/a.jpg"),
            asset("b.jpg", "/media/a/b.jpg"),
            asset("c.jpg", "/media/a/c.jpg"),
        ];
        let got = download(
            &server,
            &Endpoints::new("http://mock.test"),
            &AssetCache::new(&dir),
            &pending,
        );
        assert_eq!(server.hits.borrow().len(), 1, "only the first is attempted");
        assert_eq!(got.failures.len(), 3, "all three are reported");
        assert!(
            got.failures
                .iter()
                .skip(1)
                .all(|f| f.reason.contains("not attempted")),
            "{:?}",
            got.failures
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn off_site_urls_web_pages_and_empty_bodies_are_refused() -> Result<()> {
        let dir = scratch("refuse")?;
        let server = Server {
            routes: vec![
                ("/media/a/page.jpg", Ok(b"<!DOCTYPE html><html>".to_vec())),
                ("/media/a/empty.jpg", Ok(Vec::new())),
            ],
            hits: RefCell::new(Vec::new()),
        };
        let off_site = PendingAsset {
            url: "http://192.168.0.1/admin".to_string(),
            ..asset("admin.jpg", "")
        };
        let pending = [
            off_site,
            asset("page.jpg", "/media/a/page.jpg"),
            asset("empty.jpg", "/media/a/empty.jpg"),
        ];
        let cache = AssetCache::new(&dir);
        let got = download(
            &server,
            &Endpoints::new("http://mock.test"),
            &cache,
            &pending,
        );
        assert!(got.assets.is_empty(), "nothing accepted: {:?}", got.assets);
        assert_eq!(got.failures.len(), 3, "three refusals");
        assert_eq!(
            server.hits.borrow().len(),
            2,
            "the off-site URL was never requested"
        );
        assert!(
            cache
                .get("https://www.shadertoy.com/media/a/page.jpg")
                .is_none(),
            "no web page in the cache"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
