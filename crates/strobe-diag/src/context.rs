// ============================================================================
// Module:       strobe_diag::context
// Description:  The session context: facts every crash report must carry,
//               updated by the app as it learns them.
//
// Dependencies: strobe_diag::{buildinfo, clock}
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

use crate::{buildinfo, clock};
use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError, TryLockError};

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
///
/// The value is formatted before the lock is taken. `Display` is code this
/// module does not own: if it panics while the lock is held, the panic hook
/// (which reads [`snapshot`] for the crash report) runs on the same thread
/// before unwinding and waits for a lock that thread will never release
/// (D399).
pub fn set(key: &str, value: impl Display) {
    let text = value.to_string();
    let mut facts = FACTS.lock().unwrap_or_else(PoisonError::into_inner);
    facts.insert(key.to_string(), text);
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
/// pass first, then the app's keys alphabetically. Machine facts (OS, CPU,
/// RAM) are not here; reports get them from [`crate::system`].
pub fn snapshot() -> Vec<(String, String)> {
    let facts = FACTS.lock().unwrap_or_else(PoisonError::into_inner);
    assemble(&facts)
}

/// [`snapshot`] that never waits for the lock: `None` if another thread
/// (possibly the crashed one) holds it. For crash handlers.
pub fn try_snapshot() -> Option<Vec<(String, String)>> {
    let facts = match FACTS.try_lock() {
        Ok(facts) => facts,
        // A lock poisoned by an earlier panic still guards usable facts.
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return None,
    };
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
    use std::sync::Arc;

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

    /// Panics when asked to display itself.
    struct PanicsOnDisplay;

    impl Display for PanicsOnDisplay {
        fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            // The point of the type: a `Display` that panics, raised
            // without the panic macros the workspace denies.
            let values = std::hint::black_box(vec![1_u8]);
            std::hint::black_box(values[std::hint::black_box(7)]);
            Ok(())
        }
    }

    type Hook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static>;

    /// A `Display` that panics while `set` formats it must not do so with
    /// the facts lock held: the panic hook (ours reads `snapshot()`) runs on
    /// the panicking thread before any unwinding, and a lock held by that
    /// same thread would never be released (D399). The hook stands in for
    /// ours with a non-blocking read, so a held lock shows as a failure
    /// instead of a hung test.
    #[test]
    fn a_display_that_panics_does_not_hold_the_facts_lock() -> Result<()> {
        use std::sync::atomic::AtomicBool;
        const THREAD: &str = "context-display-panic";
        static HOOK_SAW_FREE_LOCK: AtomicBool = AtomicBool::new(false);
        static HOOK_RAN: AtomicBool = AtomicBool::new(false);

        let previous: Arc<Mutex<Option<Hook>>> =
            Arc::new(Mutex::new(Some(std::panic::take_hook())));
        let chained = Arc::clone(&previous);
        std::panic::set_hook(Box::new(move |info| {
            if std::thread::current().name() == Some(THREAD) {
                HOOK_RAN.store(true, Ordering::SeqCst);
                // Other tests take the lock briefly, so a free lock may
                // take a few tries; a lock held by this thread never frees.
                let free = (0..10_000).any(|_| {
                    std::thread::yield_now();
                    try_snapshot().is_some()
                });
                HOOK_SAW_FREE_LOCK.store(free, Ordering::SeqCst);
            } else if let Some(hook) = chained
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
            {
                hook(info);
            }
        }));

        let outcome = std::thread::Builder::new()
            .name(THREAD.to_string())
            .spawn(|| std::panic::catch_unwind(|| set("test.display-panic", PanicsOnDisplay)))
            .map_err(|error| anyhow!("cannot spawn the test thread: {error}"))?
            .join();

        // Put the earlier hook back before asserting anything.
        let restored = previous
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(std::panic::take_hook());
        if let Some(hook) = restored {
            std::panic::set_hook(hook);
        }

        let caught = outcome.map_err(|_| anyhow!("the panic escaped catch_unwind"))?;
        assert!(caught.is_err(), "the Display must have panicked");
        assert!(HOOK_RAN.load(Ordering::SeqCst), "the hook never ran");
        assert!(
            HOOK_SAW_FREE_LOCK.load(Ordering::SeqCst),
            "the facts lock was held while the value was formatted"
        );
        assert_eq!(get("test.display-panic"), None, "nothing half-written");
        // A later set still works: the lock is not poisoned into uselessness.
        set("test.display-panic", "fine");
        assert_eq!(get("test.display-panic").as_deref(), Some("fine"), "set");
        remove("test.display-panic");
        Ok(())
    }

    #[test]
    fn try_snapshot_gives_up_instead_of_waiting() -> Result<()> {
        let held = FACTS.lock().unwrap_or_else(PoisonError::into_inner);
        let attempt = try_snapshot();
        drop(held);
        assert!(attempt.is_none(), "try_snapshot waited for a held lock");
        // Other tests in this binary share FACTS and run in parallel, so the
        // lock can be briefly held by one of them here; "works when free"
        // means it succeeds once nobody holds it, not on the first try.
        let succeeded = (0..10_000).any(|_| {
            std::thread::yield_now();
            try_snapshot().is_some()
        });
        assert!(succeeded, "try_snapshot works when free");
        Ok(())
    }
}
