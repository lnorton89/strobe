// ============================================================================
// Module:       strobe_io::target
// Description:  What a mapping or a macro points at, and whether that
//               target can be found in the scene that is playing.
//
// Dependencies: strobe-scene (ControlBank, names, preset); crate::macros
// ============================================================================

//! Targets and their resolution (UI_DESIGN section 8, D173).
//!
//! A [`Target`] is *named*, never indexed: a scene control by its name, a
//! meta control, a transport command by its id, a cue number, a fav slot, a
//! preset, or a Performance Macro. Naming is what lets a mapping outlive
//! the scene it was made in: it is looked up again every time a scene
//! plays, and if it is not there the mapping is [`State::Unavailable`], not
//! an error and not deleted.
//!
//! ## How a control is found
//!
//! In this order, and never fuzzily (a guess that moves the wrong control
//! on a projector is worse than a control that does nothing):
//!
//! 1. the exact name, normalised the way the script API matches (case, `_`,
//!    `-` and spaces ignored);
//! 2. `group/name`, when the name has a slash, against the control's
//!    `UI_GROUP` and name, normalised the same way;
//! 3. the macro's aliases, each tried as 1 and 2, in order.
//!
//! A meta target is looked up only in the meta bank, and a command only in
//! the app's command table (the app says which ids it has).
//!
//! ## States
//!
//! [`State`] is the six states of UI_IMPLEMENTATION_PLAN section 11. This
//! module decides the first four from the target alone; `Learning` and
//! `Conflict` are the mapper's and are filled in by [`crate::engine`].

use crate::macros::MacroSet;
use serde::{Deserialize, Serialize};
use strobe_scene::control::{ControlBank, ControlKind};
use strobe_scene::names::normalize;
use strobe_scene::preset::{self, FAV_SLOTS, FavSlots};

/// What a mapping or macro acts on.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// A control of the scene that is playing.
    Control {
        /// Its name, or `group/name`.
        name: String,
        /// For an XY pad (0 = x, 1 = y) or a colour (0 = r, 1 = g, 2 = b):
        /// which component one fader drives.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        component: Option<u8>,
    },
    /// A meta control (master, brightness, hue, mirror...).
    Meta {
        /// Its name.
        name: String,
        /// As for [`Target::Control`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        component: Option<u8>,
    },
    /// A command from the app's command table, by its stable id.
    Command {
        /// The id, such as `next_scene`.
        id: String,
    },
    /// Jump to a playlist cue.
    Cue {
        /// The cue's number, from 1.
        number: u32,
    },
    /// Recall a fav slot.
    FavSlot {
        /// The slot, from 1.
        slot: u8,
    },
    /// Recall a preset of the scene that is playing.
    Preset {
        /// The preset's name.
        name: String,
    },
    /// A Performance Macro, which stands for its own target.
    Macro {
        /// The macro's id.
        id: u32,
    },
}

impl Target {
    /// A scene control with no component.
    pub fn control(name: &str) -> Self {
        Self::Control {
            name: name.to_string(),
            component: None,
        }
    }

    /// A meta control with no component.
    pub fn meta(name: &str) -> Self {
        Self::Meta {
            name: name.to_string(),
            component: None,
        }
    }

    /// A command by id.
    pub fn command(id: &str) -> Self {
        Self::Command { id: id.to_string() }
    }

    /// The source badge of UI_DESIGN section 8 for what this acts on.
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Control { .. } => "SCENE",
            Self::Meta { .. } => "META",
            Self::Cue { .. } => "PLAYLIST",
            Self::Command { .. } | Self::FavSlot { .. } | Self::Preset { .. } => "SYSTEM",
            Self::Macro { .. } => "MACRO",
        }
    }

    /// A short name for a row: `speed`, `meta: hue`, `Command next_scene`.
    pub fn label(&self) -> String {
        let with_component = |name: &str, component: &Option<u8>| match component {
            Some(c) => format!("{name}.{c}"),
            None => name.to_string(),
        };
        match self {
            Self::Control { name, component } => with_component(name, component),
            Self::Meta { name, component } => format!("meta: {}", with_component(name, component)),
            Self::Command { id } => format!("command: {id}"),
            Self::Cue { number } => format!("cue {number}"),
            Self::FavSlot { slot } => format!("fav slot {slot}"),
            Self::Preset { name } => format!("preset: {name}"),
            Self::Macro { id } => format!("macro #{id}"),
        }
    }

    /// Whether this acts on a control (so a fader makes sense for it).
    pub fn is_control(&self) -> bool {
        matches!(self, Self::Control { .. } | Self::Meta { .. })
    }
}

/// `x`/`y` for a pad, `r`/`g`/`b` for a colour, by index.
pub fn component_name(component: u8, kind: &ControlKind) -> &'static str {
    let names: [&str; 3] = match kind {
        ControlKind::Color { .. } => ["r", "g", "b"],
        ControlKind::Slider(_)
        | ControlKind::Knob(_)
        | ControlKind::Toggle { .. }
        | ControlKind::Bang(_)
        | ControlKind::Xy(_)
        | ControlKind::Dropdown { .. } => ["x", "y", "z"],
    };
    names.get(usize::from(component)).copied().unwrap_or("?")
}

/// The state of a mapping, macro or target (UI_IMPLEMENTATION_PLAN 11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum State {
    /// The target was found and will be driven.
    Resolved,
    /// The target is not in what is playing now. Kept, muted, harmless.
    Unavailable,
    /// The target can never work as written (an unknown command, a fav slot
    /// that does not exist, an XY pad with no component).
    Broken,
    /// Switched off by the operator.
    Disabled,
    /// Waiting for the next controller message to bind to it.
    Learning,
    /// Its source is also bound to something else.
    Conflict,
}

impl State {
    /// The state's word, shown beside its icon (never colour alone).
    pub fn label(self) -> &'static str {
        match self {
            Self::Resolved => "Resolved",
            Self::Unavailable => "Unavailable",
            Self::Broken => "Broken",
            Self::Disabled => "Disabled",
            Self::Learning => "Learning",
            Self::Conflict => "Conflict",
        }
    }
}

/// A state with the reason a person would want to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The state.
    pub state: State,
    /// Why, in a sentence (empty when resolved).
    pub reason: String,
}

impl Resolution {
    fn resolved() -> Self {
        Self {
            state: State::Resolved,
            reason: String::new(),
        }
    }

    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            state: State::Unavailable,
            reason: reason.into(),
        }
    }

    fn broken(reason: impl Into<String>) -> Self {
        Self {
            state: State::Broken,
            reason: reason.into(),
        }
    }

    fn disabled(reason: impl Into<String>) -> Self {
        Self {
            state: State::Disabled,
            reason: reason.into(),
        }
    }
}

/// What targets are looked up in.
pub struct Scope<'a> {
    /// The playing scene's controls, if a scene is playing.
    pub scene: Option<&'a ControlBank>,
    /// The meta controls.
    pub meta: Option<&'a ControlBank>,
    /// The macro set, for [`Target::Macro`].
    pub macros: &'a MacroSet,
    /// The fav slots.
    pub favs: &'a FavSlots,
    /// The playing scene's preset names.
    pub presets: &'a [String],
    /// Whether the app's command table has this id.
    pub command_known: &'a dyn Fn(&str) -> bool,
}

/// Which bank a control is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BankId {
    /// The playing scene's own controls.
    Scene,
    /// The meta controls.
    Meta,
}

/// A control found in a bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlRef {
    /// The bank.
    pub bank: BankId,
    /// The control's index there.
    pub index: usize,
    /// The component a fader drives, for a multi-component control.
    pub component: Option<usize>,
}

/// What a target flattened to: the macro looked through, if it was one.
#[derive(Clone, Copy, Debug)]
pub struct Flat<'a> {
    /// The real target.
    pub target: &'a Target,
    /// Alternative names to try for a control.
    pub aliases: &'a [String],
    /// The macro's usable range, if it narrowed one.
    pub range: Option<[f32; 2]>,
}

/// Looks through a macro to the target it stands for.
pub fn flatten<'a>(target: &'a Target, macros: &'a MacroSet) -> Result<Flat<'a>, Resolution> {
    let Target::Macro { id } = target else {
        return Ok(Flat {
            target,
            aliases: &[],
            range: None,
        });
    };
    let Some(found) = macros.get(*id) else {
        return Err(Resolution::broken(format!("there is no macro #{id}")));
    };
    if !found.enabled {
        return Err(Resolution::disabled(format!(
            "macro \"{}\" is off",
            found.name
        )));
    }
    if matches!(found.target, Target::Macro { .. }) {
        return Err(Resolution::broken(format!(
            "macro \"{}\" points at another macro",
            found.name
        )));
    }
    Ok(Flat {
        target: &found.target,
        aliases: &found.aliases,
        range: found.range,
    })
}

/// The index of the control `name` (or an alias) names in `bank`.
pub fn find_control(bank: &ControlBank, name: &str, aliases: &[String]) -> Option<usize> {
    std::iter::once(name)
        .chain(aliases.iter().map(String::as_str))
        .find_map(|candidate| find_one(bank, candidate))
}

fn find_one(bank: &ControlBank, name: &str) -> Option<usize> {
    if let Some(index) = bank.find(name) {
        return Some(index);
    }
    let (group, control) = name.split_once('/')?;
    let (group, control) = (normalize(group), normalize(control));
    bank.defs().iter().position(|def| {
        def.group.as_deref().map(normalize).as_deref() == Some(group.as_str())
            && normalize(&def.name) == control
    })
}

/// Finds the control a flat target names, or says why not.
pub fn locate_control(
    flat: &Flat<'_>,
    scene: Option<&ControlBank>,
    meta: Option<&ControlBank>,
) -> Result<ControlRef, Resolution> {
    let (bank_id, bank, name, component) = match flat.target {
        Target::Control { name, component } => {
            let Some(bank) = scene else {
                return Err(Resolution::unavailable("no scene is playing"));
            };
            (BankId::Scene, bank, name, component)
        }
        Target::Meta { name, component } => {
            let Some(bank) = meta else {
                return Err(Resolution::unavailable(
                    "the meta controls are not available",
                ));
            };
            (BankId::Meta, bank, name, component)
        }
        Target::Command { .. }
        | Target::Cue { .. }
        | Target::FavSlot { .. }
        | Target::Preset { .. }
        | Target::Macro { .. } => {
            return Err(Resolution::broken("this target is not a control"));
        }
    };
    let Some(index) = find_control(bank, name, flat.aliases) else {
        return Err(match bank_id {
            BankId::Scene => Resolution::unavailable(format!(
                "\"{name}\" is not a control of the scene that is playing"
            )),
            BankId::Meta => Resolution::broken(format!("there is no meta control \"{name}\"")),
        });
    };
    let Some(def) = bank.defs().get(index) else {
        return Err(Resolution::unavailable("the control went away"));
    };
    let dims = def.dims();
    let component = match (component, &def.kind) {
        (Some(c), ControlKind::Xy(_) | ControlKind::Color { .. }) if usize::from(*c) < dims => {
            Some(usize::from(*c))
        }
        (Some(c), ControlKind::Xy(_) | ControlKind::Color { .. }) => {
            return Err(Resolution::broken(format!(
                "\"{name}\" has {dims} components, not {}",
                usize::from(*c) + 1
            )));
        }
        (None, ControlKind::Xy(_) | ControlKind::Color { .. }) => {
            return Err(Resolution::broken(format!(
                "\"{name}\" has {dims} components: choose which one this drives"
            )));
        }
        (
            Some(_) | None,
            ControlKind::Slider(_)
            | ControlKind::Knob(_)
            | ControlKind::Toggle { .. }
            | ControlKind::Bang(_)
            | ControlKind::Dropdown { .. },
        ) => None,
    };
    Ok(ControlRef {
        bank: bank_id,
        index,
        component,
    })
}

/// Whether `target` can be driven right now, and if not, why.
pub fn resolve(target: &Target, scope: &Scope<'_>) -> Resolution {
    let flat = match flatten(target, scope.macros) {
        Ok(flat) => flat,
        Err(resolution) => return resolution,
    };
    match flat.target {
        Target::Control { .. } | Target::Meta { .. } => {
            match locate_control(&flat, scope.scene, scope.meta) {
                Ok(_) => Resolution::resolved(),
                Err(resolution) => resolution,
            }
        }
        Target::Command { id } => {
            if (scope.command_known)(id) {
                Resolution::resolved()
            } else {
                Resolution::broken(format!("there is no command \"{id}\""))
            }
        }
        Target::Cue { number } => {
            if *number >= 1 {
                Resolution::resolved()
            } else {
                Resolution::broken("cue numbers start at 1")
            }
        }
        Target::FavSlot { slot } => {
            let index = usize::from(*slot).wrapping_sub(1);
            if index >= FAV_SLOTS {
                Resolution::broken(format!("fav slots are 1 to {FAV_SLOTS}"))
            } else if scope.favs.get(index).is_none() {
                Resolution::unavailable(format!("fav slot {slot} is empty"))
            } else {
                Resolution::resolved()
            }
        }
        Target::Preset { name } => {
            if !preset::valid_name(name) {
                Resolution::broken(format!("\"{name}\" is not a usable preset name"))
            } else if scope.scene.is_none() {
                Resolution::unavailable("no scene is playing")
            } else if preset::find_name(scope.presets, name).is_some() {
                Resolution::resolved()
            } else {
                Resolution::unavailable(format!("the scene has no preset \"{name}\""))
            }
        }
        Target::Macro { .. } => Resolution::broken("a macro cannot point at a macro"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macros::Macro;
    use anyhow::{Result, anyhow};
    use strobe_scene::control::ControlDef;
    use strobe_scene::json::RawControl;

    fn bank(controls: &[&str]) -> Result<ControlBank> {
        let mut defs = Vec::new();
        for json in controls {
            let raw: RawControl = serde_json::from_str(json)?;
            defs.push(ControlDef::from_raw(&raw)?);
        }
        Ok(ControlBank::new(defs)?)
    }

    fn scene() -> Result<ControlBank> {
        bank(&[
            r#"{ "NAME": "Speed", "TYPE": "slider", "UI_GROUP": "Motion" }"#,
            r#"{ "NAME": "zoom_level", "TYPE": "knob" }"#,
            r#"{ "NAME": "tint", "TYPE": "color" }"#,
            r#"{ "NAME": "pad", "TYPE": "xy" }"#,
        ])
    }

    struct Fixture {
        scene: ControlBank,
        meta: ControlBank,
        macros: MacroSet,
        favs: FavSlots,
        presets: Vec<String>,
    }

    impl Fixture {
        fn new() -> Result<Self> {
            Ok(Self {
                scene: scene()?,
                meta: strobe_scene::meta::bank()?,
                macros: MacroSet::default(),
                favs: FavSlots::default(),
                presets: vec!["Slow".into()],
            })
        }

        fn resolve(&self, target: &Target, playing: bool) -> Resolution {
            let known = |id: &str| id == "next_scene";
            let scope = Scope {
                scene: playing.then_some(&self.scene),
                meta: Some(&self.meta),
                macros: &self.macros,
                favs: &self.favs,
                presets: &self.presets,
                command_known: &known,
            };
            resolve(target, &scope)
        }
    }

    #[test]
    fn a_control_resolves_by_the_scripts_name_rule() -> Result<()> {
        let f = Fixture::new()?;
        for name in ["Speed", "speed", "SPEED", "zoom level", "Zoom-Level"] {
            assert_eq!(
                f.resolve(&Target::control(name), true).state,
                State::Resolved,
                "{name}"
            );
        }
        Ok(())
    }

    #[test]
    fn group_slash_name_is_the_second_rule() -> Result<()> {
        let f = Fixture::new()?;
        assert_eq!(
            f.resolve(&Target::control("motion/speed"), true).state,
            State::Resolved,
            "group/name"
        );
        assert_eq!(
            f.resolve(&Target::control("other/speed"), true).state,
            State::Unavailable,
            "wrong group is not a guess"
        );
        Ok(())
    }

    #[test]
    fn a_missing_control_is_unavailable_not_broken() -> Result<()> {
        let f = Fixture::new()?;
        let r = f.resolve(&Target::control("glow"), true);
        assert_eq!(r.state, State::Unavailable, "state");
        assert!(
            r.reason.contains("glow"),
            "the reason names it: {}",
            r.reason
        );
        assert_eq!(
            f.resolve(&Target::control("speed"), false).state,
            State::Unavailable,
            "no scene"
        );
        Ok(())
    }

    #[test]
    fn near_misses_are_never_resolved() -> Result<()> {
        let f = Fixture::new()?;
        for name in ["spee", "speeds", "zoom", "sp"] {
            assert_eq!(
                f.resolve(&Target::control(name), true).state,
                State::Unavailable,
                "{name}"
            );
        }
        Ok(())
    }

    #[test]
    fn aliases_let_one_macro_follow_a_control_across_scenes() -> Result<()> {
        let mut f = Fixture::new()?;
        let id = f.macros.add(Macro::new("Rate", Target::control("rate")));
        assert_eq!(
            f.resolve(&Target::Macro { id }, true).state,
            State::Unavailable,
            "no such control yet"
        );
        f.macros.set_aliases(id, vec!["speed".into()]);
        assert_eq!(
            f.resolve(&Target::Macro { id }, true).state,
            State::Resolved,
            "alias found"
        );
        Ok(())
    }

    #[test]
    fn multi_component_controls_need_a_component() -> Result<()> {
        let f = Fixture::new()?;
        let with = |name: &str, component| Target::Control {
            name: name.into(),
            component,
        };
        assert_eq!(
            f.resolve(&with("tint", None), true).state,
            State::Broken,
            "colour needs one"
        );
        assert_eq!(
            f.resolve(&with("tint", Some(2)), true).state,
            State::Resolved,
            "blue"
        );
        assert_eq!(
            f.resolve(&with("tint", Some(3)), true).state,
            State::Broken,
            "no fourth"
        );
        assert_eq!(
            f.resolve(&with("pad", Some(1)), true).state,
            State::Resolved,
            "y"
        );
        assert_eq!(
            f.resolve(&with("speed", Some(1)), true).state,
            State::Resolved,
            "ignored on a slider"
        );
        Ok(())
    }

    #[test]
    fn meta_resolves_without_a_scene_and_a_typo_is_broken() -> Result<()> {
        let f = Fixture::new()?;
        assert_eq!(
            f.resolve(&Target::meta("Hue"), false).state,
            State::Resolved,
            "meta"
        );
        assert_eq!(
            f.resolve(&Target::meta("hew"), true).state,
            State::Broken,
            "typo"
        );
        Ok(())
    }

    #[test]
    fn commands_cues_and_fav_slots_are_checked_for_sense() -> Result<()> {
        let mut f = Fixture::new()?;
        assert_eq!(
            f.resolve(&Target::command("next_scene"), false).state,
            State::Resolved,
            "known"
        );
        assert_eq!(
            f.resolve(&Target::command("nope"), false).state,
            State::Broken,
            "unknown"
        );
        assert_eq!(
            f.resolve(&Target::Cue { number: 3 }, false).state,
            State::Resolved,
            "cue 3"
        );
        assert_eq!(
            f.resolve(&Target::Cue { number: 0 }, false).state,
            State::Broken,
            "cue 0"
        );
        assert_eq!(
            f.resolve(&Target::FavSlot { slot: 0 }, false).state,
            State::Broken,
            "slot 0"
        );
        assert_eq!(
            f.resolve(&Target::FavSlot { slot: 9 }, false).state,
            State::Broken,
            "slot 9"
        );
        assert_eq!(
            f.resolve(&Target::FavSlot { slot: 2 }, false).state,
            State::Unavailable,
            "empty slot"
        );
        f.favs.set(
            1,
            preset::Fav {
                scene: "a".into(),
                preset: None,
            },
        );
        assert_eq!(
            f.resolve(&Target::FavSlot { slot: 2 }, false).state,
            State::Resolved,
            "filled slot"
        );
        Ok(())
    }

    #[test]
    fn presets_resolve_against_the_playing_scenes_list() -> Result<()> {
        let f = Fixture::new()?;
        let preset = |name: &str| Target::Preset { name: name.into() };
        assert_eq!(
            f.resolve(&preset("slow"), true).state,
            State::Resolved,
            "case-insensitive"
        );
        assert_eq!(
            f.resolve(&preset("fast"), true).state,
            State::Unavailable,
            "not in this scene"
        );
        assert_eq!(
            f.resolve(&preset("slow"), false).state,
            State::Unavailable,
            "no scene"
        );
        assert_eq!(
            f.resolve(&preset("../x"), true).state,
            State::Broken,
            "bad name"
        );
        Ok(())
    }

    #[test]
    fn macros_look_through_to_their_target_and_report_their_own_state() -> Result<()> {
        let mut f = Fixture::new()?;
        let id = f.macros.add(Macro::new("Speed", Target::control("speed")));
        assert_eq!(
            f.resolve(&Target::Macro { id }, true).state,
            State::Resolved,
            "resolved"
        );
        f.macros.set_enabled(id, false);
        assert_eq!(
            f.resolve(&Target::Macro { id }, true).state,
            State::Disabled,
            "disabled"
        );
        assert_eq!(
            f.resolve(&Target::Macro { id: 999 }, true).state,
            State::Broken,
            "missing macro"
        );
        let inner = f.macros.add(Macro::new("Inner", Target::control("speed")));
        let outer = f
            .macros
            .add(Macro::new("Outer", Target::Macro { id: inner }));
        assert_eq!(
            f.resolve(&Target::Macro { id: outer }, true).state,
            State::Broken,
            "macro of a macro"
        );
        Ok(())
    }

    #[test]
    fn locate_control_returns_the_bank_index_and_component() -> Result<()> {
        let f = Fixture::new()?;
        let target = Target::Control {
            name: "tint".into(),
            component: Some(1),
        };
        let flat = flatten(&target, &f.macros).map_err(|r| anyhow!(r.reason))?;
        let found =
            locate_control(&flat, Some(&f.scene), Some(&f.meta)).map_err(|r| anyhow!(r.reason))?;
        assert_eq!(found.bank, BankId::Scene, "scene bank");
        assert_eq!(found.index, 2, "third control");
        assert_eq!(found.component, Some(1), "green");
        Ok(())
    }

    #[test]
    fn targets_round_trip_through_json_and_label_themselves() -> Result<()> {
        let targets = [
            Target::control("speed"),
            Target::Control {
                name: "tint".into(),
                component: Some(0),
            },
            Target::meta("hue"),
            Target::command("next_scene"),
            Target::Cue { number: 4 },
            Target::FavSlot { slot: 3 },
            Target::Preset {
                name: "slow".into(),
            },
            Target::Macro { id: 7 },
        ];
        for target in targets {
            let text = serde_json::to_string(&target)?;
            assert_eq!(serde_json::from_str::<Target>(&text)?, target, "{text}");
            assert!(!target.label().is_empty(), "label");
            assert!(!target.badge().is_empty(), "badge");
        }
        assert_eq!(Target::control("speed").badge(), "SCENE", "scene badge");
        assert_eq!(Target::meta("hue").badge(), "META", "meta badge");
        Ok(())
    }
}
