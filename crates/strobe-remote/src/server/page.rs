// ============================================================================
// Module:       strobe_remote::server::page
// Description:  The controller page, embedded in the binary, the
//               Content-Security-Policy it is served with, and the web app
//               files beside it (manifest, service worker, icons).
//
// Dependencies: sha2; super::ws (base64)
// ============================================================================

//! The controller page (L-06, D253) and the installable app around it
//! (D517).
//!
//! `controller.html` is one self-contained file: markup, one `<style>` and
//! one `<script>`, with nothing loaded from anywhere else. No CDN, no fonts,
//! no analytics. The only other requests it makes are to this listener:
//! its web app manifest, its icons and its service worker, all embedded
//! here ([`asset`]).
//!
//! It is served with a Content-Security-Policy that allows exactly that
//! style and that script, by their SHA-256 hashes, worked out from the
//! embedded file once, so an edit to the page can never be served under a
//! stale policy. Everything else is `'none'` or this origin only: no other
//! script, no frames, no form posts, no plugins, and the page cannot be
//! framed by another site. `connect-src` allows the listener's own
//! WebSocket.
//!
//! The page keeps its token in `localStorage` and sends it only in a
//! WebSocket message body. It never reads the URL's query or fragment, so
//! no secret can arrive by link (a test checks the source for that).
//!
//! The service worker keeps only the page and its icons, network first, so
//! an installed app launched with Strobe off shows its own "Disconnected"
//! state. Browsers run it only in a secure context, which for a phone means
//! the operator's HTTPS proxy (D258).

use sha2::{Digest as _, Sha256};
use std::sync::OnceLock;

/// The page, as shipped.
pub(super) const HTML: &str = include_str!("controller.html");

/// The web app manifest the page links to.
const MANIFEST: &str = include_str!("manifest.webmanifest");

/// The service worker: network first, the page and icons kept for offline.
const WORKER: &str = include_str!("sw.js");

/// The policy the worker script runs under: it may fetch this origin only.
const WORKER_POLICY: &str = "Content-Security-Policy: default-src 'self'";

/// The file at `path` other than the page, with its `Content-Type` and any
/// extra header lines. The icons are the brand's own renders
/// (`cargo xtask brand`).
pub(super) fn asset(path: &str) -> Option<(&'static str, &'static [u8], &'static [&'static str])> {
    Some(match path {
        "/manifest.webmanifest" => ("application/manifest+json", MANIFEST.as_bytes(), &[]),
        "/sw.js" => (
            "text/javascript; charset=utf-8",
            WORKER.as_bytes(),
            &[WORKER_POLICY],
        ),
        "/icon.svg" => (
            "image/svg+xml",
            include_bytes!("../../../../assets/brand/icon.svg"),
            &[],
        ),
        "/icon-192.png" => (
            "image/png",
            include_bytes!("../../../../assets/brand/icon-192.png"),
            &[],
        ),
        "/icon-512.png" => (
            "image/png",
            include_bytes!("../../../../assets/brand/icon-512.png"),
            &[],
        ),
        "/icon-maskable-512.png" => (
            "image/png",
            include_bytes!("../../../../assets/brand/icon-maskable-512.png"),
            &[],
        ),
        _ => return None,
    })
}

/// The text between the first `<tag>` and `</tag>`.
fn inner<'a>(html: &'a str, tag: &str) -> &'a str {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    html.split_once(open.as_str())
        .and_then(|(_, rest)| rest.split_once(close.as_str()))
        .map_or("", |(inside, _)| inside)
}

/// `'sha256-...'` for a CSP source list.
fn hash_source(text: &str) -> String {
    format!(
        "'sha256-{}'",
        super::ws::base64(&Sha256::digest(text.as_bytes()))
    )
}

/// The policy's fixed part, with the page's hashes in it.
fn fixed_policy() -> &'static str {
    static POLICY: OnceLock<String> = OnceLock::new();
    POLICY.get_or_init(|| {
        format!(
            "default-src 'none'; script-src {}; style-src {}; img-src 'self' data:; \
             manifest-src 'self'; worker-src 'self'; \
             base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
            hash_source(inner(HTML, "script")),
            hash_source(inner(HTML, "style")),
        )
    })
}

/// The response headers for the page, served to a request whose `Host`
/// (already validated) was `host`, through the HTTPS proxy when `proxied`.
pub(super) fn headers(host: &str, proxied: bool) -> Vec<String> {
    // An IPv6 literal is not a CSP host-source; 'self' covers it in
    // browsers that map it to ws:, and IPv4 or a proxy's name gets the
    // explicit form.
    let connect = if host.starts_with('[') {
        "'self'".to_string()
    } else if proxied {
        format!("'self' wss://{host}")
    } else {
        format!("'self' ws://{host}")
    };
    vec![
        format!(
            "Content-Security-Policy: {}; connect-src {connect}",
            fixed_policy()
        ),
        "X-Frame-Options: DENY".to_string(),
        "Cross-Origin-Opener-Policy: same-origin".to_string(),
        "Cross-Origin-Resource-Policy: same-origin".to_string(),
        "Permissions-Policy: camera=(), microphone=(), geolocation=()".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RemoteCommand;

    #[test]
    fn the_page_loads_nothing_from_anywhere_else() {
        // `url(#id)` names a gradient in the page's own SVG; it loads
        // nothing, so it is the one `url(` allowed.
        let lower = HTML.to_ascii_lowercase().replace("url(#", "#");
        for needle in [
            "http://",
            "https://",
            "//cdn",
            "@import",
            "<iframe",
            "<object",
            "<embed",
            "src=",
            "url(",
            "fetch(",
            "xmlhttprequest",
            "sendbeacon",
            "eval(",
            "new function",
        ] {
            assert!(!lower.contains(needle), "the page contains {needle:?}");
        }
        assert_eq!(lower.matches("<script").count(), 1, "one script");
        assert_eq!(lower.matches("<style").count(), 1, "one style");
        assert!(
            !lower.contains(" style=\"") && !lower.contains(" onclick="),
            "no inline attributes a hash cannot cover"
        );
    }

    #[test]
    fn every_file_the_web_app_names_is_served() {
        let mut named: Vec<&str> = HTML
            .split("href=\"")
            .skip(1)
            .filter_map(|piece| piece.split('"').next())
            .collect();
        named.extend(
            MANIFEST
                .split("\"src\": \"")
                .skip(1)
                .filter_map(|piece| piece.split('"').next()),
        );
        named.extend(
            WORKER
                .split_once("SHELL = [")
                .and_then(|(_, rest)| rest.split_once(']'))
                .map_or("", |(list, _)| list)
                .split(',')
                .map(|item| item.trim().trim_matches('"')),
        );
        assert!(named.len() >= 12, "page, manifest and worker: {named:?}");
        for path in named {
            assert!(
                path == "/" || asset(path).is_some(),
                "{path} is named but not served"
            );
        }
        assert!(
            HTML.contains("register(\"/sw.js\")") && asset("/sw.js").is_some(),
            "the page registers the worker it is served"
        );
        assert!(asset("/admin").is_none(), "nothing else is served");
    }

    #[test]
    fn the_manifest_makes_an_installable_app() -> anyhow::Result<()> {
        let manifest: serde_json::Value = serde_json::from_str(MANIFEST)?;
        assert_eq!(manifest["start_url"], "/", "starts at the page");
        assert_eq!(manifest["display"], "standalone", "its own window");
        assert!(
            manifest["name"].is_string() && manifest["short_name"].is_string(),
            "named"
        );
        let icons = manifest["icons"].as_array().map_or(&[][..], Vec::as_slice);
        for (size, purpose) in [
            ("192x192", "any"),
            ("512x512", "any"),
            ("512x512", "maskable"),
        ] {
            assert!(
                icons
                    .iter()
                    .any(|icon| icon["sizes"] == size && icon["purpose"] == purpose),
                "a {size} {purpose} icon"
            );
        }
        let policy = headers("192.168.1.20:47812", false).join("\n");
        assert!(
            policy.contains("manifest-src 'self'") && policy.contains("worker-src 'self'"),
            "the policy lets the page reach both: {policy}"
        );
        Ok(())
    }

    #[test]
    fn no_secret_can_come_from_or_go_into_a_url() {
        for needle in [
            "location.search",
            "location.hash",
            "URLSearchParams",
            "?token",
            "document.cookie",
        ] {
            assert!(!HTML.contains(needle), "the page uses {needle}");
        }
    }

    #[test]
    fn every_button_sends_a_command_the_protocol_allows() {
        let ids: Vec<&str> = RemoteCommand::ALL.iter().map(|c| c.id()).collect();
        let mut seen = 0;
        for piece in HTML.split("data-command=\"").skip(1) {
            let name = piece.split('"').next().unwrap_or_default();
            assert!(ids.contains(&name), "{name} is not a remote command");
            seen += 1;
        }
        assert!(seen >= 8, "the controls are all there: {seen}");
        for command in [
            "next_scene",
            "prev_scene",
            "toggle_hold",
            "restart_cue",
            "extend_cue",
            "skip_section",
            "tap_tempo",
            "blackout_on",
            "blackout_off",
        ] {
            assert!(HTML.contains(command), "{command} is reachable");
        }
    }

    #[test]
    fn the_page_meets_the_accessibility_and_layout_brief() {
        for (needle, what) in [
            ("viewport-fit=cover", "safe areas"),
            ("env(safe-area-inset-", "safe areas"),
            ("prefers-reduced-motion", "reduced motion"),
            ("prefers-contrast", "high contrast"),
            ("forced-colors", "Windows high contrast"),
            ("orientation: landscape", "landscape layout"),
            ("orientation: portrait", "portrait layout"),
            (":focus-visible", "visible keyboard focus"),
            ("aria-live", "announced status"),
            ("aria-pressed", "toggle state not by colour alone"),
            ("<label for=\"name\"", "a labelled name field"),
            ("lang=\"en\"", "a document language"),
            ("\"stale\"", "a stale state"),
            ("Reconnecting", "a reconnect state"),
            ("Disconnected", "a disconnected state"),
            ("visibilitychange", "reconnect after sleep"),
        ] {
            assert!(HTML.contains(needle), "{what}: {needle}");
        }
    }

    #[test]
    fn the_page_asks_for_a_name_and_never_for_a_code() {
        assert!(
            HTML.contains("type: \"request_pair\""),
            "it sends request_pair (D469)"
        );
        for needle in [
            "type: \"pair\"",
            "one-time-code",
            "six-digit",
            "Pairing code",
        ] {
            assert!(!HTML.contains(needle), "the page still has {needle:?}");
        }
        assert!(
            HTML.contains("maxlength=\"32\"") && crate::MAX_DEVICE_NAME == 32,
            "the field allows what the protocol does"
        );
    }

    #[test]
    fn the_policy_hashes_the_page_as_shipped() {
        let headers = headers("192.168.1.20:47812", false);
        let policy = headers
            .iter()
            .find(|h| h.starts_with("Content-Security-Policy: "))
            .map_or("", String::as_str);
        assert!(policy.contains("default-src 'none'"), "{policy}");
        assert!(
            policy.contains(&hash_source(inner(HTML, "script"))),
            "the script's hash: {policy}"
        );
        assert!(
            policy.contains(&hash_source(inner(HTML, "style"))),
            "the style's hash: {policy}"
        );
        assert!(!policy.contains("unsafe-inline"), "no unsafe-inline");
        assert!(
            policy.contains("connect-src 'self' ws://192.168.1.20:47812"),
            "its own socket: {policy}"
        );
        assert!(policy.contains("frame-ancestors 'none'"), "not framable");
        assert!(
            inner(HTML, "script").contains("use strict"),
            "the script was found"
        );
        assert!(
            inner(HTML, "style").contains(":root"),
            "the style was found"
        );
    }
}
