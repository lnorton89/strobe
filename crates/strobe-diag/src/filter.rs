// ============================================================================
// Module:       strobe_diag::filter
// Description:  Parses STROBE_LOG (`debug`, `strobe_gfx=trace,warn`) into a
//               per-target level filter.
//
// Dependencies: log (LevelFilter)
// ============================================================================

//! The `STROBE_LOG` filter syntax.
//!
//! A comma-separated list. A bare level (`debug`) sets the default for
//! every target. `target=level` sets one target and everything below it:
//! `strobe_gfx=trace` covers `strobe_gfx::player` too. When several
//! directives match, the most specific (longest) wins. Hyphens in crate
//! names are accepted (`strobe-gfx=trace`). Levels are `off`, `error`,
//! `warn`, `info`, `debug`, `trace`, case-insensitive.
//!
//! A directive that does not parse is skipped and reported in
//! [`Filter::problems`], which the logger writes into the log's first
//! lines; a typo in an environment variable must never stop a show.

use log::LevelFilter;
use std::str::FromStr;

/// The default level when `STROBE_LOG` is unset: `debug` in debug builds,
/// `info` in release builds.
pub fn default_level() -> LevelFilter {
    if cfg!(debug_assertions) {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    }
}

/// A parsed filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    default: LevelFilter,
    /// `(target prefix, level)`, longest prefix first.
    directives: Vec<(String, LevelFilter)>,
    problems: Vec<String>,
}

impl Filter {
    /// Parses `spec`; directives that do not parse are recorded, not fatal.
    pub fn parse(spec: &str, default: LevelFilter) -> Self {
        let mut filter = Self {
            default,
            directives: Vec::new(),
            problems: Vec::new(),
        };
        for part in spec
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            match part.split_once('=') {
                None => match LevelFilter::from_str(part) {
                    Ok(level) => filter.default = level,
                    Err(_) => filter.problems.push(format!("unknown level `{part}`")),
                },
                Some((target, level)) => match LevelFilter::from_str(level.trim()) {
                    Ok(level) if !target.trim().is_empty() => {
                        filter
                            .directives
                            .push((target.trim().replace('-', "_"), level));
                    }
                    Ok(_) | Err(_) => filter.problems.push(format!("bad directive `{part}`")),
                },
            }
        }
        filter
            .directives
            .sort_by_key(|(target, _)| std::cmp::Reverse(target.len()));
        filter
    }

    /// Reads `STROBE_LOG`, falling back to [`default_level`].
    pub fn from_env() -> Self {
        let spec = std::env::var("STROBE_LOG").unwrap_or_default();
        Self::parse(&spec, default_level())
    }

    /// The highest level any directive enables; the value for
    /// [`log::set_max_level`], so the `log` macros skip formatting the
    /// records nothing wants.
    pub fn max_level(&self) -> LevelFilter {
        self.directives
            .iter()
            .map(|(_, level)| *level)
            .fold(self.default, std::cmp::max)
    }

    /// Whether a record at `level` from `target` passes.
    pub fn enabled(&self, target: &str, level: log::Level) -> bool {
        let limit = self
            .directives
            .iter()
            .find(|(prefix, _)| matches_prefix(target, prefix))
            .map_or(self.default, |(_, limit)| *limit);
        level <= limit
    }

    /// Directives that could not be parsed.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// The effective filter as text, for the log header.
    pub fn describe(&self) -> String {
        let mut text = self.default.to_string().to_lowercase();
        for (target, level) in &self.directives {
            text.push_str(&format!(",{target}={}", level.to_string().to_lowercase()));
        }
        text
    }
}

/// `target` equals `prefix` or sits below it (`prefix::…`).
fn matches_prefix(target: &str, prefix: &str) -> bool {
    target
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use log::Level;

    #[test]
    fn a_bare_level_sets_the_default() -> Result<()> {
        let filter = Filter::parse("warn", LevelFilter::Info);
        assert!(filter.enabled("anything", Level::Warn), "warn passes");
        assert!(!filter.enabled("anything", Level::Info), "info is filtered");
        Ok(())
    }

    #[test]
    fn a_target_directive_covers_its_children_only() -> Result<()> {
        let filter = Filter::parse("strobe_gfx=trace", LevelFilter::Warn);
        assert!(
            filter.enabled("strobe_gfx::player", Level::Trace),
            "child target is covered"
        );
        assert!(
            filter.enabled("strobe_gfx", Level::Trace),
            "exact target is covered"
        );
        assert!(
            !filter.enabled("strobe_gfx_extra", Level::Info),
            "a longer crate name is not a child"
        );
        assert!(
            !filter.enabled("winit", Level::Info),
            "other targets keep the default"
        );
        Ok(())
    }

    #[test]
    fn the_most_specific_directive_wins() -> Result<()> {
        let filter = Filter::parse("strobe_gfx=warn,strobe_gfx::player=trace", LevelFilter::Off);
        assert!(
            filter.enabled("strobe_gfx::player", Level::Trace),
            "specific directive wins"
        );
        assert!(
            !filter.enabled("strobe_gfx::caps", Level::Info),
            "general directive applies elsewhere"
        );
        Ok(())
    }

    #[test]
    fn hyphens_and_case_are_accepted() -> Result<()> {
        let filter = Filter::parse("Strobe-Gfx=DEBUG", LevelFilter::Off);
        // Target names are case-sensitive Rust paths; only the level is not.
        assert!(
            !filter.enabled("strobe_gfx", Level::Debug),
            "target case is preserved"
        );
        let filter = Filter::parse("strobe-gfx=DEBUG", LevelFilter::Off);
        assert!(
            filter.enabled("strobe_gfx", Level::Debug),
            "hyphen maps to underscore, level is case-insensitive"
        );
        Ok(())
    }

    #[test]
    fn bad_directives_are_reported_not_fatal() -> Result<()> {
        let filter = Filter::parse("loud,strobe_gfx=verbose,=info,debug", LevelFilter::Warn);
        assert_eq!(
            filter.problems().len(),
            3,
            "problems: {:?}",
            filter.problems()
        );
        assert!(filter.enabled("x", Level::Debug), "valid parts still apply");
        Ok(())
    }

    #[test]
    fn max_level_is_the_highest_enabled() -> Result<()> {
        let filter = Filter::parse("warn,strobe_gfx=trace", LevelFilter::Info);
        assert_eq!(
            filter.max_level(),
            LevelFilter::Trace,
            "directive raises it"
        );
        let quiet = Filter::parse("off", LevelFilter::Info);
        assert_eq!(quiet.max_level(), LevelFilter::Off, "off disables all");
        Ok(())
    }

    #[test]
    fn the_description_round_trips() -> Result<()> {
        let filter = Filter::parse("warn,strobe_gfx::player=debug", LevelFilter::Info);
        assert_eq!(
            Filter::parse(&filter.describe(), LevelFilter::Info),
            filter,
            "describe() parses back to the same filter"
        );
        Ok(())
    }
}
