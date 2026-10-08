// ============================================================================
// Module:       strobe_diag::context
// Description:  The session context: facts every crash report must carry,
//               updated by the app as it learns them.
//
// Dependencies: strobe_diag::{buildinfo, clock, system}
// ============================================================================

//! Session context.
//!
//! A small thread-safe registry of `key = value` facts. The app writes to
//! it as it learns things (the GL renderer string after context creation,
//! the scene name on each load, the render size on resize) and every log
//! header, crash report and `--diagnose` report reads it back. Nothing here
//! is the app's state; it is a copy kept for post-mortems.
//!
//! Two values change every frame and so are not in the map: the frame
//! counter and the current pass index are single atomics, written with
//! [`note_frame`] and [`note_pass`] for the cost of one relaxed store. The
//! map itself is for facts that change on a user action, never per frame.
//!
//! Conventional keys have constants ([`GL_RENDERER`], [`SCENE`], …) so the
//! app and the reports agree on spelling.

use crate::{buildinfo, clock, system};
use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

/// `GL_VENDOR`.
pub const GL_VENDOR: &str = "gl.vendor";
/// `GL_RENDERER`.
pub const GL_RENDERER: &str = "gl.renderer";
/// `GL_VERSION`.
pub const GL_VERSION: &str = "gl.version";
/// Which context provider is running: `native` or `angle`.
pub const GL_BACKEND: &str = "gl.backend";
/// The scene that is playing.
pub const SCENE: &str = "scene.name";
/// Output render size, such as `1280x720`.
pub const RENDER_SIZE: &str = "render.size";

/// Sentinel stored in [`PASS`] when no pass is active.
const NO_PASS: u32 = u32::MAX;

static FACTS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
static FRAME: AtomicU64 = AtomicU64::new(0);
static PASS: AtomicU32 = AtomicU32::new(NO_PASS);

/// Records a fact, replacing any earlier value for `key`.
pub fn set(key: &str, value: impl Display) {
    let mut facts = FACTS.lock().unwrap_or_else(PoisonError::into_inner);
    facts.insert(key.to_string(), value.to_string());
}

/// Forgets a fact.
pub fn remove(key: &str) {
    let mut facts = FACTS.lock().unwrap_or_else(PoisonError::into_inner);
    facts.remove(key);
}

/// A fact's current value.
pub fn get(key: &str) -> Option<String> {
    let facts = FACTS.lock().unwrap_or_else(PoisonError::into_inner);
    facts.get(key).cloned()
}

/// Records the frame counter. One relaxed atomic store; call every frame.
pub fn note_frame(count: u64) {
    FRAME.store(count, Ordering::Relaxed);
}

/// Records the pass being rendered. One relaxed atomic store.
pub fn note_pass(index: u32) {
    PASS.store(index, Ordering::Relaxed);
}

/// Records that no pass is being rendered (between frames).
pub fn clear_pass() {
    PASS.store(NO_PASS, Ordering::Relaxed);
}

/// Everything known, sorted for stable reports: build, uptime, frame and
/// pass first, then machine facts, then the app's keys alphabetically.
pub fn snapshot() -> Vec<(String, String)> {
    let facts = FACTS.lock().unwrap_or_else(PoisonError::into_inner);
    assemble(&facts)
}

/// [`snapshot`] that never waits for the lock: `None` if another thread
/// (possibly the crashed one) holds it. For crash handlers.
pub fn try_snapshot() -> Option<Vec<(String, String)>> {
    let facts = FACTS.try_lock().ok()?;
    Some(assemble(&facts))
}

fn assemble(facts: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut pairs = vec![
        ("build".to_string(), buildinfo::summary()),
        (
            "uptime".to_string(),
            format!("{:.1}s", clock::uptime().as_secs_f64()),
        ),
        (
            "frame".to_string(),
            FRAME.load(Ordering::Relaxed).to_string(),
        ),
    ];
    let pass = PASS.load(Ordering::Relaxed);
    if pass != NO_PASS {
        pairs.push(("pass".to_string(), pass.to_string()));
    }
    if let Some(machine) = system::cached() {
        pairs.extend(machine.pairs());
    }
    pairs.extend(
        facts
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    fn value(pairs: &[(String, String)], key: &str) -> Option<String> {
        pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    }

    // The registry is process-global, so every test uses its own keys.

    #[test]
    fn a_fact_can_be_set_replaced_and_removed() -> Result<()> {
        set("test.replace", 1);
        set("test.replace", "two");
        assert_eq!(get("test.replace").as_deref(), Some("two"), "replaced");
        remove("test.replace");
        assert_eq!(get("test.replace"), None, "removed");
        Ok(())
    }

    #[test]
    fn the_snapshot_leads_with_build_uptime_and_frame() -> Result<()> {
        clock::start();
        note_frame(1234);
        set("test.snapshot", "present");
        let pairs = snapshot();
        let keys: Vec<&str> = pairs.iter().take(3).map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, ["build", "uptime", "frame"], "leading keys");
        assert_eq!(
            value(&pairs, "frame").as_deref(),
            Some("1234"),
            "frame count"
        );
        assert_eq!(
            value(&pairs, "test.snapshot").as_deref(),
            Some("present"),
            "app keys are included"
        );
        remove("test.snapshot");
        Ok(())
    }

    #[test]
    fn the_pass_appears_only_while_one_is_active() -> Result<()> {
        note_pass(2);
        let during = value(&snapshot(), "pass").ok_or_else(|| anyhow!("pass missing"))?;
        assert_eq!(during, "2", "active pass");
        clear_pass();
        assert_eq!(value(&snapshot(), "pass"), None, "no pass after clear");
        Ok(())
    }

    #[test]
    fn try_snapshot_gives_up_instead_of_waiting() -> Result<()> {
        let held = FACTS.lock().unwrap_or_else(PoisonError::into_inner);
        let attempt = try_snapshot();
        drop(held);
        assert!(attempt.is_none(), "try_snapshot waited for a held lock");
        assert!(try_snapshot().is_some(), "try_snapshot works when free");
        Ok(())
    }
}
