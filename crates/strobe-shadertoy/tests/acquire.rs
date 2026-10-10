// ============================================================================
// Module:       acquire (integration test)
// Description:  The real HTTP client, REST client and texture downloader
//               against a local mock server (Shadertoy is unreachable from CI).
//
// Dependencies: strobe_shadertoy; std::net (the mock server)
// ============================================================================

//! Acquisition against a mock Shadertoy.
//!
//! A `TcpListener` thread on `127.0.0.1:0` plays Shadertoy: it answers the
//! routes a test registers and records every request line and header, so the
//! tests can check what the real `ureq` client sent (the API path and key,
//! the user agent) as well as what it does with the answers. Nothing here
//! touches the internet.

#![cfg(feature = "net")]

use anyhow::{Result, anyhow};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use strobe_shadertoy::ShaderId;
use strobe_shadertoy::acquire::{
    ApiKey, AssetCache, Endpoints, Fetch, FetchError, Limits, RestError, UreqFetch,
    fetch_shader_json,
};
use strobe_shadertoy::import::{Downloader, ImportError, ImportOptions, import_id, import_json};

const SINGLE: &str = include_str!("fixtures/single_image.json");
const MISSING_TEXTURE: &str = include_str!("fixtures/missing_texture.json");
const TEXTURE_PATH: &str =
    "/media/a/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.jpg";

/// One canned answer.
#[derive(Clone)]
struct Answer {
    status: u16,
    body: Vec<u8>,
    delay: Duration,
}

impl Answer {
    fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            body: b"nope".to_vec(),
            delay: Duration::ZERO,
        }
    }
}

/// A recorded request.
#[derive(Debug, Clone)]
struct Seen {
    target: String,
    headers: BTreeMap<String, String>,
}

struct Mock {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Mock {
    /// Serves `routes` (by request path, query ignored) until the process
    /// ends. Unknown paths are 404.
    fn start(routes: Vec<(&'static str, Answer)>) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let routes = routes.clone();
                let log = Arc::clone(&log);
                std::thread::spawn(move || serve(stream, &routes, &log));
            }
        });
        Ok(Self { addr, seen })
    }

    fn endpoints(&self) -> Endpoints {
        Endpoints::new(&format!("http://{}", self.addr))
    }

    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

fn serve(mut stream: TcpStream, routes: &[(&'static str, Answer)], log: &Mutex<Vec<Seen>>) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&buffer).into_owned();
    let mut lines = text.split("\r\n");
    let target = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .unwrap_or_default()
        .to_string();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    if let Ok(mut log) = log.lock() {
        log.push(Seen {
            target: target.clone(),
            headers,
        });
    }
    let path = target.split('?').next().unwrap_or_default();
    let answer = routes
        .iter()
        .find(|(p, _)| *p == path)
        .map_or_else(|| Answer::status(404), |(_, a)| a.clone());
    std::thread::sleep(answer.delay);
    let head = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        answer.status,
        answer.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&answer.body);
}

fn scratch(name: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("strobe-acq-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn limits(max_bytes: usize, timeout_ms: u64) -> Limits {
    Limits {
        max_bytes,
        timeout: Duration::from_millis(timeout_ms),
    }
}

fn key() -> Result<ApiKey> {
    ApiKey::parse("Bd8tWD").ok_or_else(|| anyhow!("key"))
}

#[test]
fn the_client_returns_a_200_body_and_identifies_itself() -> Result<()> {
    let mock = Mock::start(vec![("/hello", Answer::ok("world"))])?;
    let url = format!("http://{}/hello", mock.addr);
    let body = UreqFetch::direct().get(&url, limits(1024, 5000))?;
    assert_eq!(body, b"world", "body");
    let seen = mock.requests();
    let agent = seen
        .first()
        .and_then(|s| s.headers.get("user-agent"))
        .cloned()
        .unwrap_or_default();
    assert!(agent.starts_with("strobe/"), "user agent: {agent}");
    Ok(())
}

#[test]
fn statuses_timeouts_size_limits_and_dead_ports_are_distinct_errors() -> Result<()> {
    let slow = Answer {
        delay: Duration::from_millis(1500),
        ..Answer::ok("late")
    };
    let mock = Mock::start(vec![
        ("/forbidden", Answer::status(403)),
        ("/slow", slow),
        ("/big", Answer::ok(vec![b'x'; 5000])),
    ])?;
    let client = UreqFetch::direct();
    let url = |p: &str| format!("http://{}{p}", mock.addr);

    assert_eq!(
        client.get(&url("/forbidden"), limits(1024, 5000)),
        Err(FetchError::Status { status: 403 }),
        "403"
    );
    assert_eq!(
        client.get(&url("/missing"), limits(1024, 5000)),
        Err(FetchError::Status { status: 404 }),
        "404"
    );
    let started = Instant::now();
    let timed_out = client.get(&url("/slow"), limits(1024, 300));
    assert!(
        matches!(timed_out, Err(FetchError::TimedOut(_))),
        "timeout: {timed_out:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(1400),
        "gave up at the limit, not when the server answered: {:?}",
        started.elapsed()
    );
    let big = client.get(&url("/big"), limits(1000, 5000));
    assert!(
        matches!(big, Err(FetchError::TooLarge(_))),
        "size limit: {big:?}"
    );

    // A port nothing listens on: bind, note the port, drop the listener.
    let dead = {
        let socket = TcpListener::bind("127.0.0.1:0")?;
        socket.local_addr()?
    };
    let refused = client.get(&format!("http://{dead}/x"), limits(1024, 3000));
    assert!(
        refused
            .as_ref()
            .err()
            .is_some_and(FetchError::host_unreachable),
        "refused: {refused:?}"
    );
    Ok(())
}

#[test]
fn the_rest_client_sends_the_key_to_the_api_path_and_returns_the_json() -> Result<()> {
    let mock = Mock::start(vec![("/api/v1/shaders/Tst001", Answer::ok(SINGLE))])?;
    let text = fetch_shader_json(
        &UreqFetch::direct(),
        &mock.endpoints(),
        &ShaderId::parse("Tst001")?,
        &key()?,
    )?;
    assert_eq!(text, SINGLE, "the body is returned verbatim");
    let seen = mock.requests();
    assert_eq!(
        seen.first().map(|s| s.target.as_str()),
        Some("/api/v1/shaders/Tst001?key=Bd8tWD"),
        "path and key"
    );
    Ok(())
}

#[test]
fn a_refused_or_challenged_rest_call_is_a_message_not_a_hang() -> Result<()> {
    let mock = Mock::start(vec![
        ("/api/v1/shaders/Tst403", Answer::status(403)),
        (
            "/api/v1/shaders/Tst200",
            Answer::ok("<html><title>Just a moment...</title></html>"),
        ),
    ])?;
    let client = UreqFetch::direct();
    let refused = fetch_shader_json(
        &client,
        &mock.endpoints(),
        &ShaderId::parse("Tst403")?,
        &key()?,
    );
    let text = refused
        .as_ref()
        .err()
        .map(ToString::to_string)
        .unwrap_or_default();
    assert!(
        text.contains("403") && text.contains("userscript"),
        "{text}"
    );
    assert!(!text.contains("Bd8tWD"), "the key leaked into: {text}");
    let challenged = fetch_shader_json(
        &client,
        &mock.endpoints(),
        &ShaderId::parse("Tst200")?,
        &key()?,
    );
    assert!(
        matches!(challenged, Err(RestError::WebPage(_))),
        "{challenged:?}"
    );
    Ok(())
}

#[test]
fn importing_by_id_downloads_the_texture_once_then_serves_it_from_the_cache() -> Result<()> {
    let png = image_bytes();
    let mock = Mock::start(vec![
        ("/api/v1/shaders/Tst008", Answer::ok(MISSING_TEXTURE)),
        (TEXTURE_PATH, Answer::ok(png.clone())),
    ])?;
    let library = scratch("by-id-lib")?;
    let cache_dir = scratch("by-id-cache")?;
    let client = UreqFetch::direct();
    let endpoints = mock.endpoints();
    let cache = AssetCache::new(&cache_dir);
    let options = |replace: bool| ImportOptions {
        replace,
        downloader: Some(Downloader {
            fetch: &client,
            endpoints: &endpoints,
            cache: &cache,
        }),
        ..ImportOptions::offline(&library)
    };

    let done = import_id(
        &ShaderId::parse("Tst008")?,
        &key()?,
        &client,
        &endpoints,
        &options(false),
    )?;
    assert_eq!(
        done.textures_missing, 0,
        "texture bound: {:?}",
        done.warnings
    );
    assert_eq!(done.textures_found, 1, "one texture");
    let images: Vec<_> = std::fs::read_dir(done.dir.join("images"))?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        images.iter().any(|n| n.ends_with(".jpg")),
        "the texture is in the scene: {images:?}"
    );
    let texture_requests = |mock: &Mock| {
        mock.requests()
            .iter()
            .filter(|s| s.target == TEXTURE_PATH)
            .count()
    };
    assert_eq!(texture_requests(&mock), 1, "downloaded once");

    // Again, replacing: the texture comes from the cache.
    let again = import_id(
        &ShaderId::parse("Tst008")?,
        &key()?,
        &client,
        &endpoints,
        &options(true),
    )?;
    assert!(again.replaced, "replaced");
    assert_eq!(texture_requests(&mock), 1, "no second texture request");
    std::fs::remove_dir_all(&library)?;
    std::fs::remove_dir_all(&cache_dir)?;
    Ok(())
}

#[test]
fn a_texture_that_404s_is_a_visible_warning_and_the_scene_still_imports() -> Result<()> {
    let mock = Mock::start(vec![])?;
    let library = scratch("404-lib")?;
    let cache_dir = scratch("404-cache")?;
    let client = UreqFetch::direct();
    let endpoints = mock.endpoints();
    let cache = AssetCache::new(&cache_dir);
    let done = import_json(
        MISSING_TEXTURE,
        &ImportOptions {
            downloader: Some(Downloader {
                fetch: &client,
                endpoints: &endpoints,
                cache: &cache,
            }),
            ..ImportOptions::offline(&library)
        },
    )?;
    assert_eq!(done.textures_missing, 1, "still missing");
    assert!(
        done.warnings
            .iter()
            .any(|w| w.contains("was not downloaded") && w.contains("404")),
        "{:?}",
        done.warnings
    );
    assert!(
        done.dir.join("images/pending-assets.json").is_file(),
        "pending list written"
    );
    std::fs::remove_dir_all(&library)?;
    std::fs::remove_dir_all(&cache_dir)?;
    Ok(())
}

#[test]
fn a_shader_that_names_an_off_site_texture_never_makes_a_request() -> Result<()> {
    let mock = Mock::start(vec![])?;
    let evil = MISSING_TEXTURE.replace(
        "/media/a/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.jpg",
        "http://127.0.0.1:9/internal/secret.jpg",
    );
    let library = scratch("evil-lib")?;
    let cache_dir = scratch("evil-cache")?;
    let client = UreqFetch::direct();
    let endpoints = mock.endpoints();
    let cache = AssetCache::new(&cache_dir);
    let done = import_json(
        &evil,
        &ImportOptions {
            downloader: Some(Downloader {
                fetch: &client,
                endpoints: &endpoints,
                cache: &cache,
            }),
            ..ImportOptions::offline(&library)
        },
    )?;
    assert!(
        done.warnings
            .iter()
            .any(|w| w.contains("not on shadertoy.com")),
        "refused by name: {:?}",
        done.warnings
    );
    assert!(mock.requests().is_empty(), "no request reached the mock");
    std::fs::remove_dir_all(&library)?;
    std::fs::remove_dir_all(&cache_dir)?;
    Ok(())
}

#[test]
fn an_api_error_reply_becomes_the_error_message() -> Result<()> {
    let mock = Mock::start(vec![(
        "/api/v1/shaders/TstErr",
        Answer::ok(r#"{"Error": "Shader not found"}"#),
    )])?;
    let library = scratch("api-error")?;
    let result = import_id(
        &ShaderId::parse("TstErr")?,
        &key()?,
        &UreqFetch::direct(),
        &mock.endpoints(),
        &ImportOptions::offline(&library),
    );
    assert!(
        matches!(&result, Err(ImportError::NotAShader(e)) if e.to_string().contains("Shader not found")),
        "{result:?}"
    );
    std::fs::remove_dir_all(&library)?;
    Ok(())
}

/// A valid 1x1 PNG.
fn image_bytes() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x60,
        0x60, 0x60, 0xf8, 0x0f, 0x00, 0x01, 0x04, 0x01, 0x00, 0x80, 0xbb, 0xd1, 0x5b, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ]
}
