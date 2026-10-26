// ============================================================================
// Module:       strobe_app::ui::fonts
// Description:  The control window's typefaces: Segoe UI (regular and
//               semibold) and Consolas from the system where Windows has
//               them, egui's bundled faces everywhere else.
//
// Dependencies: egui (FontDefinitions); std::fs at start-up only
// ============================================================================

//! Typefaces (D465).
//!
//! egui ships one proportional face, Ubuntu Light: a single thin weight, so
//! a heading could only ever be bigger than the text under it, never
//! heavier, and the whole window read light and unfinished. Every Windows
//! install has Segoe UI in regular and semibold and Consolas, and Windows is
//! where the show runs, so [`install`] reads them from the system's font
//! folder once at start-up. Nothing is bundled or downloaded: the files are
//! the operating system's.
//!
//! Where a file is missing or unreadable (macOS, Linux, a stripped-down
//! Windows image), egui's own faces stay, and [`strong`] falls back to the
//! proportional family. A caller therefore never names a family the
//! context does not have, and the window looks as it did before.
//!
//! The choice is per thread, as the theme's is (`theme::palette`): egui is
//! single-threaded per context, and the control window's is the only one
//! that installs anything. Tests never install, so their text metrics are
//! egui's on every machine.

use egui::{FontData, FontDefinitions, FontFamily, FontId};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

/// The family name semibold text is drawn in once [`install`] has run.
const STRONG: &str = "strong";

thread_local! {
    /// The family [`strong`] returns: [`STRONG`] after a successful
    /// [`install`], the proportional family before. Kept built so a frame
    /// clones an `Arc` rather than allocating a name.
    static STRONG_FAMILY: RefCell<FontFamily> = const { RefCell::new(FontFamily::Proportional) };
}

/// Reads the system's faces and installs them on `ctx`. Call once, at
/// start-up: it reads about a megabyte from disk. Does nothing, and leaves
/// egui's faces, where the files are not there.
pub fn install(ctx: &egui::Context) {
    let Some(dir) = system_fonts() else {
        return;
    };
    let read = |name: &str| std::fs::read(dir.join(name)).ok();
    let (Some(regular), Some(semibold)) = (read("segoeui.ttf"), read("seguisb.ttf")) else {
        log::info!(
            "UI fonts: Segoe UI not found in {}; using egui's",
            dir.display()
        );
        return;
    };
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "segoe-ui".to_owned(),
        Arc::new(FontData::from_owned(regular)),
    );
    fonts.font_data.insert(
        "segoe-ui-semibold".to_owned(),
        Arc::new(FontData::from_owned(semibold)),
    );
    // egui's faces stay behind Segoe in each list, for the glyphs it lacks.
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "segoe-ui".to_owned());
    let mut strong = vec!["segoe-ui-semibold".to_owned()];
    strong.extend(proportional.iter().cloned());
    fonts
        .families
        .insert(FontFamily::Name(STRONG.into()), strong);
    if let Some(mono) = read("consola.ttf") {
        fonts
            .font_data
            .insert("consolas".to_owned(), Arc::new(FontData::from_owned(mono)));
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "consolas".to_owned());
    }
    ctx.set_fonts(fonts);
    STRONG_FAMILY.with(|family| family.replace(FontFamily::Name(STRONG.into())));
    log::info!("UI fonts: Segoe UI from {}", dir.display());
}

/// The system's font folder on Windows; `None` elsewhere.
fn system_fonts() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let root = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot"))?;
    Some(PathBuf::from(root).join("Fonts"))
}

/// A semibold font of `size`: a heading, a title. The regular proportional
/// face where no semibold one was installed.
#[must_use]
pub fn strong(size: f32) -> FontId {
    FontId::new(size, STRONG_FAMILY.with(|family| family.borrow().clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_an_install_strong_text_uses_a_family_every_context_has() {
        let font = strong(15.0);
        assert_eq!(
            font.family,
            FontFamily::Proportional,
            "a context with egui's faces only has no `strong` family"
        );
        assert!(
            (font.size - 15.0).abs() < f32::EPSILON,
            "the size passes through: {font:?}"
        );
    }
}
