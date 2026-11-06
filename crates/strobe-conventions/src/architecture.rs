// ============================================================================
// Module:       strobe_conventions::architecture
// Description:  The crate graph's rules: which crates may not reach which,
//               read from `cargo metadata`'s resolved graph, normal edges and
//               dev edges apart.
//
// Dependencies: anyhow, serde_json, std::process (cargo metadata);
//               crate::sources (the workspace root)
// ============================================================================

//! The layers stay layers (D541).
//!
//! `docs/ARCHITECTURE.md` draws the crates as layers: the scene model and
//! the GLSL library under everything, the renderer above them, the
//! subsystems beside it, and the app on top. A layer is only a layer while
//! nothing reaches down past it or sideways into a neighbour, and a
//! `Cargo.toml` line is all it takes. Each rule here names a crate, a set of
//! crates (or third-party packages) it must never depend on, and why. The
//! check walks `cargo metadata`'s resolved graph from the crate through its
//! **normal** dependencies, transitively, so an edge through a third crate
//! counts too. Dev-dependencies are a separate graph: a renderer test
//! importing a scene converter is not the renderer depending on it, and the
//! rules say which graph they cover.

use crate::sources::workspace_root;
use anyhow::{Context as _, Result, anyhow};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

/// One rule: `krate` must not reach any of `forbidden` through `graph`.
#[derive(Debug, Clone, Copy)]
pub struct Rule {
    /// The crate the rule is about.
    pub krate: &'static str,
    /// Packages it must never reach.
    pub forbidden: &'static [&'static str],
    /// Why, for the failure message.
    pub why: &'static str,
}

/// The binaries and tooling: nothing in the workspace links them.
const TOP: [&str; 4] = ["strobe-app", "strobe-bench", "strobe-conventions", "xtask"];

/// What draws or opens a window.
const GRAPHICS: [&str; 6] = ["strobe-gfx", "glow", "glutin", "winit", "egui", "egui_glow"];

/// The rules, checked over normal (non-dev) edges.
pub const RULES: &[Rule] = &[
    Rule {
        krate: "strobe-scene",
        forbidden: &[
            "strobe-gfx",
            "strobe-audio",
            "strobe-script",
            "strobe-io",
            "strobe-remote",
            "glow",
            "glutin",
            "winit",
            "egui",
            "ureq",
            "rquickjs",
            "cpal",
        ],
        why: "the scene model is tested without a GPU, a window, a network or a script engine",
    },
    Rule {
        krate: "strobe-ssf-lib",
        forbidden: &["strobe-scene", "strobe-gfx", "glow"],
        why: "the GLSL library is text, below the scene model",
    },
    Rule {
        krate: "strobe-audio",
        forbidden: &GRAPHICS,
        why: "analysis runs on its own thread and in the browser build; it draws nothing",
    },
    Rule {
        krate: "strobe-script",
        forbidden: &GRAPHICS,
        why: "a script sets uniforms through the app; it never touches GL",
    },
    Rule {
        krate: "strobe-io",
        forbidden: &GRAPHICS,
        why: "show control is a queue of messages, drained by the app",
    },
    Rule {
        krate: "strobe-playlist",
        forbidden: &GRAPHICS,
        why: "the timeline is pure state, tested without a clock or a GPU",
    },
    Rule {
        krate: "strobe-remote",
        forbidden: &GRAPHICS,
        why: "the remote is a network service; the phone draws its UI, not this crate",
    },
    Rule {
        krate: "strobe-projector",
        forbidden: &GRAPHICS,
        why: "the projector service runs headless, as a Windows service",
    },
    Rule {
        krate: "strobe-gfx",
        forbidden: &[
            "strobe-shadertoy",
            "strobe-autocontrol",
            "strobe-script",
            "strobe-audio",
            "strobe-io",
            "strobe-remote",
        ],
        why: "the renderer plays scenes; importing and scoring them sit above it",
    },
    Rule {
        krate: "strobe-autocontrol",
        forbidden: &["strobe-shadertoy", "strobe-diag", "strobe-script", "ureq", "rquickjs"],
        why: "it runs in the browser build (D558), where the crash handler and QuickJS do not \
              build; it reads GLSL with strobe-scene's tokenizer",
    },
    Rule {
        krate: "strobe-web",
        forbidden: &["strobe-shadertoy", "strobe-diag", "strobe-script", "ureq", "rquickjs"],
        why: "the browser build takes the renderer, the DSP and the control generator alone (D524, D558)",
    },
    Rule {
        krate: "strobe-diag",
        forbidden: &[
            "strobe-gfx",
            "strobe-scene",
            "strobe-script",
            "strobe-audio",
        ],
        why: "diagnostics are the bottom layer every binary links first",
    },
];

/// The resolved graph: each package id's name, and its edges by kind.
#[derive(Debug, Default)]
pub struct Graph {
    names: BTreeMap<String, String>,
    normal: BTreeMap<String, Vec<String>>,
    dev: BTreeMap<String, Vec<String>>,
}

impl Graph {
    /// Reads `cargo metadata --format-version 1 --locked` for the workspace.
    pub fn load() -> Result<Self> {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let output = Command::new(cargo)
            .args([
                "metadata",
                "--format-version",
                "1",
                "--locked",
                "--all-features",
            ])
            .current_dir(workspace_root()?)
            .output()
            .context("failed to run cargo metadata")?;
        if !output.status.success() {
            return Err(anyhow!(
                "cargo metadata failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Self::parse(&serde_json::from_slice(&output.stdout)?)
    }

    /// Builds the graph from metadata JSON.
    pub fn parse(metadata: &Value) -> Result<Self> {
        let mut graph = Self::default();
        for package in metadata["packages"].as_array().context("no packages")? {
            if let (Some(id), Some(name)) = (package["id"].as_str(), package["name"].as_str()) {
                graph.names.insert(id.to_owned(), name.to_owned());
            }
        }
        for node in metadata["resolve"]["nodes"]
            .as_array()
            .context("no resolve graph")?
        {
            let id = node["id"]
                .as_str()
                .context("a node without an id")?
                .to_owned();
            for dep in node["deps"].as_array().into_iter().flatten() {
                let Some(target) = dep["pkg"].as_str() else {
                    continue;
                };
                for kind in dep["dep_kinds"].as_array().into_iter().flatten() {
                    // `null` is a normal dependency; build-dependencies run at
                    // compile time and are counted with them.
                    let edges = match kind["kind"].as_str() {
                        Some("dev") => &mut graph.dev,
                        Some(_) | None => &mut graph.normal,
                    };
                    edges.entry(id.clone()).or_default().push(target.to_owned());
                }
            }
        }
        Ok(graph)
    }

    /// Adds an edge, for tests.
    pub fn add(&mut self, from: &str, to: &str, dev: bool) {
        self.names.insert(from.to_owned(), from.to_owned());
        self.names.insert(to.to_owned(), to.to_owned());
        let edges = if dev { &mut self.dev } else { &mut self.normal };
        edges
            .entry(from.to_owned())
            .or_default()
            .push(to.to_owned());
    }

    fn ids_named(&self, name: &str) -> Vec<&String> {
        self.names
            .iter()
            .filter(|(_, n)| *n == name)
            .map(|(id, _)| id)
            .collect()
    }

    /// Every package name `name` reaches through normal edges, with the path
    /// to each, shortest first.
    pub fn reaches(&self, name: &str) -> BTreeMap<String, Vec<String>> {
        let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut seen: BTreeSet<&String> = BTreeSet::new();
        let mut queue: std::collections::VecDeque<(&String, Vec<String>)> = self
            .ids_named(name)
            .into_iter()
            .map(|id| (id, vec![name.to_owned()]))
            .collect();
        while let Some((id, path)) = queue.pop_front() {
            if !seen.insert(id) {
                continue;
            }
            for next in self.normal.get(id).into_iter().flatten() {
                let next_name = self
                    .names
                    .get(next)
                    .cloned()
                    .unwrap_or_else(|| next.clone());
                let mut longer = path.clone();
                longer.push(next_name.clone());
                found.entry(next_name).or_insert_with(|| longer.clone());
                queue.push_back((next, longer));
            }
        }
        found
    }

    /// Workspace crates with a normal edge to one of `names`.
    pub fn dependents_of(&self, names: &[&str]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (from, tos) in &self.normal {
            for to in tos {
                let to_name = self.names.get(to).map_or(to.as_str(), String::as_str);
                if names.contains(&to_name) {
                    let from_name = self
                        .names
                        .get(from)
                        .cloned()
                        .unwrap_or_else(|| from.clone());
                    out.push((from_name, to_name.to_owned()));
                }
            }
        }
        out.sort();
        out
    }
}

/// Every broken rule, one line each.
pub fn problems(graph: &Graph) -> Vec<String> {
    let mut out = Vec::new();
    for rule in RULES {
        let reached = graph.reaches(rule.krate);
        for forbidden in rule.forbidden {
            if let Some(path) = reached.get(*forbidden) {
                out.push(format!(
                    "{} reaches {forbidden} ({}): {}",
                    rule.krate,
                    path.join(" -> "),
                    rule.why
                ));
            }
        }
    }
    for (from, to) in graph.dependents_of(&TOP) {
        out.push(format!(
            "{from} depends on {to}, a binary or tool nothing links (D541)"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workspace_keeps_its_layers() -> Result<()> {
        let graph = Graph::load()?;
        let found = problems(&graph);
        assert!(
            found.is_empty(),
            "{} crate-graph rule(s) broken (D541); move the code, or put the shared part in a lower crate:\n{}",
            found.len(),
            found.join("\n")
        );
        assert!(
            !graph.reaches("strobe-app").is_empty(),
            "the graph was read: the app reaches its crates"
        );
        Ok(())
    }

    #[test]
    fn a_planted_forbidden_edge_is_found_through_a_third_crate() {
        let mut graph = Graph::default();
        graph.add("strobe-audio", "helper", false);
        graph.add("helper", "strobe-gfx", false);
        let found = problems(&graph);
        assert!(
            found.iter().any(|p| p.contains(
                "strobe-audio reaches strobe-gfx (strobe-audio -> helper -> strobe-gfx)"
            )),
            "{found:?}"
        );
    }

    #[test]
    fn a_dev_edge_is_not_a_dependency() {
        let mut graph = Graph::default();
        graph.add("strobe-gfx", "strobe-shadertoy", true);
        assert!(
            problems(&graph).is_empty(),
            "a test importing a converter is allowed"
        );
    }

    #[test]
    fn nothing_may_link_a_binary() {
        let mut graph = Graph::default();
        graph.add("strobe-io", "strobe-app", false);
        assert!(
            problems(&graph)
                .iter()
                .any(|p| p.contains("strobe-io depends on strobe-app")),
            "the app is a leaf"
        );
    }
}
