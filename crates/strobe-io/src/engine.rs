// ============================================================================
// Module:       strobe_io::engine
// Description:  The mapper: takes a source and a signal, finds the mappings
//               (and the built-in OSC addresses) that answer to it, and
//               drives the control banks or hands back commands. Also learn.
//
// Dependencies: strobe-scene (ControlBank); crate::{event, mapping, macros,
//               target, osc}
// ============================================================================

//! The mapper (D173, D174).
//!
//! [`Engine::handle`] is the whole of show control as seen from the frame
//! loop: the app drains its queue once per scene frame and passes each
//! message here with the two [`Banks`]. Controls are written through
//! [`ControlBank`] (`set_normalized`, `press`), never around it, so a
//! toggle snaps, a bang counts presses, a dropdown rounds, a smoothed
//! control eases and a speed control integrates exactly as when the
//! inspector or a script writes them. Everything that is not a control
//! (a command, a cue, a fav slot, a preset) comes back as an [`Output`] for
//! the app to carry out through the same actions its buttons use.
//!
//! ## What a message does
//!
//! 1. If a target is armed for **learn**, the first usable message binds to
//!    it ([`Engine::arm_new`], [`Engine::arm_relearn`]) instead of driving
//!    anything, unless its source is already mapped to something else, which
//!    is a [`Outcome::Conflict`] the operator answers.
//! 2. Otherwise every enabled mapping whose source matches is driven. A
//!    target that is not in the playing scene is skipped and reported as
//!    unavailable: not an error, not a log line per message.
//! 3. An OSC address no mapping claims falls back to the built-in
//!    `/strobe/...` space ([`crate::osc::builtin_target`]) when that is on.
//!
//! What a signal does to what kind of control:
//!
//! | Control | Analog (fader, knob) | Button (pad, key) |
//! |---|---|---|
//! | slider, knob, XY, colour component | position through range, curve, invert; soft takeover | toggle: flips between range ends; momentary: follows |
//! | toggle | on above the threshold | toggle: flips; momentary: follows |
//! | bang | presses when it crosses the threshold | presses |
//! | dropdown | position picks the option | steps to the next option |
//! | command, cue, fav slot, preset | fires when it crosses the threshold | fires on press |

use crate::event::{Signal, Source};
use crate::macros::MacroSet;
use crate::mapping::{ButtonMode, Cfg, Mapping, Runtime, find_conflict};
use crate::osc;
use crate::takeover::Takeover;
use crate::target::{BankId, ControlRef, Flat, Target, flatten, locate_control};
use strobe_scene::control::{ControlBank, ControlKind};

/// The two banks a message can write to.
pub struct Banks<'a> {
    /// The playing scene's controls, if a scene is playing.
    pub scene: Option<&'a mut ControlBank>,
    /// The meta controls.
    pub meta: Option<&'a mut ControlBank>,
}

/// Something for the app to do, that is not a control write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Run the command with this id.
    Command(String),
    /// Jump to this cue, numbered from 1.
    Cue(u32),
    /// Recall this fav slot, numbered from 1.
    FavSlot(u8),
    /// Recall this preset of the playing scene.
    Preset(String),
}

/// What one message amounted to, for the activity monitor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing is mapped to it.
    Unmapped,
    /// It drove this many targets.
    Applied(usize),
    /// A fader is waiting to pick its control up.
    Waiting,
    /// It was mapped, but the target is not in what is playing.
    Unavailable(String),
    /// Learn bound it, as the mapping at this index.
    Learned(usize),
    /// Learn found its source already in use; the operator must choose.
    Conflict,
    /// Learn is armed and this message cannot be learned (a note off).
    Ignored,
}

impl Outcome {
    /// A short phrase for the monitor's row.
    pub fn label(&self) -> String {
        match self {
            Self::Unmapped => "unmapped".to_string(),
            Self::Applied(1) => "applied".to_string(),
            Self::Applied(n) => format!("applied to {n}"),
            Self::Waiting => "waiting for pickup".to_string(),
            Self::Unavailable(why) => format!("unavailable: {why}"),
            Self::Learned(_) => "learned".to_string(),
            Self::Conflict => "already mapped".to_string(),
            Self::Ignored => "ignored while learning".to_string(),
        }
    }
}

/// What learn is waiting to bind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Armed {
    /// A new mapping to this target.
    New(Target),
    /// A new source for the mapping at this index.
    Relearn(usize),
}

/// A learned source that is already mapped, waiting for an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The source that was heard.
    pub source: Source,
    /// The mapping it is already bound to.
    pub existing: usize,
    /// What it was going to be bound to.
    pub armed: Armed,
}

/// The mapper: mappings, macros and the learn state.
#[derive(Clone, Debug)]
pub struct Engine {
    /// The mappings, in the order the operator sees them.
    pub mappings: Vec<Mapping>,
    /// The Performance Macros.
    pub macros: MacroSet,
    /// Answer `/strobe/...` OSC addresses that no mapping claims.
    pub builtin_osc: bool,
    learn: Option<Armed>,
    conflict: Option<Conflict>,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            mappings: Vec::new(),
            macros: MacroSet::default(),
            builtin_osc: true,
            learn: None,
            conflict: None,
        }
    }
}

impl Engine {
    // ------------------------------------------------------------- learn

    /// Arms learn for a new mapping to `target`. The next usable message
    /// binds.
    pub fn arm_new(&mut self, target: Target) {
        self.conflict = None;
        self.learn = Some(Armed::New(target));
    }

    /// Arms learn to replace the source of mapping `index`.
    pub fn arm_relearn(&mut self, index: usize) {
        self.conflict = None;
        if index < self.mappings.len() {
            self.learn = Some(Armed::Relearn(index));
        }
    }

    /// Turns learn off, and drops an unanswered conflict.
    pub fn cancel_learn(&mut self) {
        self.learn = None;
        self.conflict = None;
    }

    /// What learn is waiting for.
    pub fn armed(&self) -> Option<&Armed> {
        self.learn.as_ref()
    }

    /// Whether learn is armed for a new mapping to exactly `target`.
    pub fn is_learning_target(&self, target: &Target) -> bool {
        matches!(&self.learn, Some(Armed::New(armed)) if armed == target)
    }

    /// Whether learn is armed for mapping `index`'s source.
    pub fn is_relearning(&self, index: usize) -> bool {
        matches!(&self.learn, Some(Armed::Relearn(i)) if *i == index)
    }

    /// The unanswered conflict, if the last learn found one.
    pub fn conflict(&self) -> Option<&Conflict> {
        self.conflict.as_ref()
    }

    /// Answers a conflict. `replace` removes the mappings that held the
    /// source and binds it as asked; otherwise nothing changes. Returns the
    /// bound mapping's index.
    pub fn resolve_conflict(&mut self, replace: bool) -> Option<usize> {
        let conflict = self.conflict.take()?;
        if !replace {
            return None;
        }
        let keep = match &conflict.armed {
            Armed::Relearn(i) => Some(*i),
            Armed::New(_) => None,
        };
        let doomed: Vec<usize> = self
            .mappings
            .iter()
            .enumerate()
            .filter(|(i, m)| Some(*i) != keep && m.enabled && m.source.overlaps(&conflict.source))
            .map(|(i, _)| i)
            .collect();
        let armed = match conflict.armed {
            Armed::Relearn(i) => Armed::Relearn(i - doomed.iter().filter(|d| **d < i).count()),
            new @ Armed::New(_) => new,
        };
        for index in doomed.into_iter().rev() {
            self.mappings.remove(index);
        }
        Some(self.bind(armed, conflict.source))
    }

    fn bind(&mut self, armed: Armed, source: Source) -> usize {
        match armed {
            Armed::New(target) => {
                let mut mapping = Mapping::new(source, target);
                if mapping.source.is_osc() {
                    // OSC senders are absolute and have no way to be picked
                    // up; a jump is what they expect.
                    mapping.takeover = Takeover::Jump;
                }
                self.mappings.push(mapping);
                self.mappings.len() - 1
            }
            Armed::Relearn(index) => {
                if let Some(mapping) = self.mappings.get_mut(index) {
                    if source.is_osc() && !mapping.source.is_osc() {
                        mapping.takeover = Takeover::Jump;
                    }
                    mapping.source = source;
                    mapping.runtime = Runtime::default();
                }
                index
            }
        }
    }

    fn capture(&mut self, source: &Source, signal: Signal) -> Outcome {
        let learnable = matches!(signal, Signal::Analog(_) | Signal::Button(true));
        let Some(armed) = self.learn.clone() else {
            return Outcome::Ignored;
        };
        if !learnable {
            return Outcome::Ignored;
        }
        self.learn = None;
        let except = match &armed {
            Armed::Relearn(i) => Some(*i),
            Armed::New(_) => None,
        };
        if let Some(existing) = find_conflict(&self.mappings, source, except) {
            let same = match &armed {
                Armed::New(target) => self
                    .mappings
                    .get(existing)
                    .is_some_and(|m| m.target == *target && m.source == *source),
                Armed::Relearn(_) => false,
            };
            if same {
                return Outcome::Learned(existing);
            }
            self.conflict = Some(Conflict {
                source: source.clone(),
                existing,
                armed,
            });
            return Outcome::Conflict;
        }
        Outcome::Learned(self.bind(armed, source.clone()))
    }

    // ------------------------------------------------------------ mapping

    /// Handles one message. Control writes go straight to `banks`;
    /// everything else is appended to `out`.
    pub fn handle(
        &mut self,
        source: &Source,
        signal: Signal,
        banks: &mut Banks<'_>,
        out: &mut Vec<Output>,
    ) -> Outcome {
        if self.learn.is_some() {
            return self.capture(source, signal);
        }
        let mut matched = false;
        let mut applied = 0_usize;
        let mut waiting = false;
        let mut unavailable: Option<String> = None;
        for mapping in &mut self.mappings {
            if !mapping.enabled || !mapping.source.matches(source) {
                continue;
            }
            matched = true;
            let cfg = mapping.cfg();
            let result = match flatten(&mapping.target, &self.macros) {
                Ok(flat) => drive(&cfg, &mut mapping.runtime, &flat, signal, banks, out),
                Err(resolution) => Drive::Unavailable(resolution.reason),
            };
            match result {
                Drive::Applied => applied += 1,
                Drive::Waiting => waiting = true,
                Drive::Idle => {}
                Drive::Unavailable(why) => unavailable = Some(why),
            }
        }
        if !matched
            && self.builtin_osc
            && let Source::Osc { address } = source
            && let Some(target) = osc::builtin_target(address, &self.macros)
        {
            let result = match flatten(&target, &self.macros) {
                Ok(flat) => drive(
                    &Cfg {
                        takeover: Takeover::Jump,
                        button: ButtonMode::Momentary,
                        stateless: true,
                        ..Cfg::default()
                    },
                    &mut Runtime::default(),
                    &flat,
                    signal,
                    banks,
                    out,
                ),
                Err(resolution) => Drive::Unavailable(resolution.reason),
            };
            return match result {
                Drive::Applied => Outcome::Applied(1),
                Drive::Waiting | Drive::Idle => Outcome::Applied(0),
                Drive::Unavailable(why) => Outcome::Unavailable(why),
            };
        }
        if !matched {
            Outcome::Unmapped
        } else if applied > 0 {
            Outcome::Applied(applied)
        } else if waiting {
            Outcome::Waiting
        } else if let Some(why) = unavailable {
            Outcome::Unavailable(why)
        } else {
            Outcome::Applied(0)
        }
    }
}

/// What driving one target did.
#[derive(Debug)]
enum Drive {
    Applied,
    Waiting,
    Idle,
    Unavailable(String),
}

fn drive(
    cfg: &Cfg,
    runtime: &mut Runtime,
    flat: &Flat<'_>,
    signal: Signal,
    banks: &mut Banks<'_>,
    out: &mut Vec<Output>,
) -> Drive {
    match flat.target {
        Target::Control { .. } | Target::Meta { .. } => {
            let found = locate_control(flat, banks.scene.as_deref(), banks.meta.as_deref());
            let found = match found {
                Ok(found) => found,
                Err(resolution) => return Drive::Unavailable(resolution.reason),
            };
            let bank = match found.bank {
                BankId::Scene => banks.scene.as_deref_mut(),
                BankId::Meta => banks.meta.as_deref_mut(),
            };
            match bank {
                Some(bank) => drive_control(cfg, runtime, flat.range, signal, bank, found),
                None => Drive::Unavailable("the bank went away".to_string()),
            }
        }
        Target::Command { id } => fire(cfg, runtime, signal, out, || Output::Command(id.clone())),
        Target::Cue { number } => fire(cfg, runtime, signal, out, || Output::Cue(*number)),
        Target::FavSlot { slot } => fire(cfg, runtime, signal, out, || Output::FavSlot(*slot)),
        Target::Preset { name } => fire(cfg, runtime, signal, out, || Output::Preset(name.clone())),
        Target::Macro { .. } => Drive::Unavailable("a macro cannot point at a macro".to_string()),
    }
}

fn fire(
    cfg: &Cfg,
    runtime: &mut Runtime,
    signal: Signal,
    out: &mut Vec<Output>,
    make: impl FnOnce() -> Output,
) -> Drive {
    if runtime.edge(signal, cfg.threshold).pressed {
        out.push(make());
        Drive::Applied
    } else {
        Drive::Idle
    }
}

fn drive_control(
    cfg: &Cfg,
    runtime: &mut Runtime,
    outer: Option<[f32; 2]>,
    signal: Signal,
    bank: &mut ControlBank,
    found: ControlRef,
) -> Drive {
    let index = found.index;
    let Some(def) = bank.defs().get(index) else {
        return Drive::Idle;
    };
    let kind = def.kind;
    let options = def.values.len().max(1);
    let edge = runtime.edge(signal, cfg.threshold);
    match kind {
        ControlKind::Bang(_) => {
            if edge.pressed {
                bank.press(index);
                Drive::Applied
            } else {
                Drive::Idle
            }
        }
        ControlKind::Toggle { .. } => {
            if matches!(signal, Signal::Button(_)) && cfg.button == ButtonMode::Toggle {
                if edge.pressed {
                    bank.press(index);
                    return Drive::Applied;
                }
                return Drive::Idle;
            }
            if edge.pressed || edge.released || cfg.stateless {
                let value = cfg.shape(if edge.down { 1.0 } else { 0.0 }, outer);
                bank.set_normalized(index, Some(0), &[value]);
                Drive::Applied
            } else {
                Drive::Idle
            }
        }
        ControlKind::Dropdown { .. } => match signal {
            Signal::Analog(x) => {
                let value = cfg.shape(x, outer);
                bank.set_normalized(index, Some(0), &[value]);
                Drive::Applied
            }
            Signal::Button(_) => {
                if !edge.pressed {
                    return Drive::Idle;
                }
                let current = bank
                    .target(index)
                    .map_or(0, |target| target[0].round().max(0.0) as usize);
                let next = (current + 1) % options;
                bank.set(index, Some(0), &[next as f32]);
                Drive::Applied
            }
        },
        ControlKind::Slider(_)
        | ControlKind::Knob(_)
        | ControlKind::Xy(_)
        | ControlKind::Color { .. } => {
            let (travel, continuous) = match signal {
                Signal::Analog(x) => (x, true),
                Signal::Button(_) => {
                    if cfg.button == ButtonMode::Toggle {
                        if !edge.pressed {
                            return Drive::Idle;
                        }
                        runtime.latched = !runtime.latched;
                        (if runtime.latched { 1.0 } else { 0.0 }, false)
                    } else if edge.pressed || edge.released {
                        (if edge.down { 1.0 } else { 0.0 }, false)
                    } else {
                        return Drive::Idle;
                    }
                }
            };
            let wanted = cfg.shape(travel, outer);
            let value = if continuous && cfg.takeover == Takeover::Pickup {
                let component = found.component.unwrap_or(0);
                let current =
                    bank.defs()
                        .get(index)
                        .zip(bank.target(index))
                        .map_or(0.0, |(def, target)| {
                            def.normalize(component, target.get(component).copied().unwrap_or(0.0))
                        });
                match runtime.pickup.filter(cfg.takeover, wanted, current) {
                    Some(value) => value,
                    None => return Drive::Waiting,
                }
            } else {
                wanted
            };
            bank.set_normalized(index, found.component, &[value]);
            Drive::Applied
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::Curve;
    use crate::macros::Macro;
    use anyhow::{Result, anyhow};
    use strobe_scene::control::ControlDef;
    use strobe_scene::json::RawControl;

    fn scene() -> Result<ControlBank> {
        let mut defs = Vec::new();
        for json in [
            r#"{ "NAME": "speed", "TYPE": "slider", "MIN": 0, "MAX": 10, "DEFAULT": 5 }"#,
            r#"{ "NAME": "glow", "TYPE": "slider smooth", "PARAMS": 0.1 }"#,
            r#"{ "NAME": "on", "TYPE": "toggle" }"#,
            r#"{ "NAME": "flash", "TYPE": "bang" }"#,
            r#"{ "NAME": "mode", "TYPE": "dropdown", "LABELS": ["a","b","c"], "VALUES": [0,1,2] }"#,
            r#"{ "NAME": "pad", "TYPE": "xy", "MIN": [0,0], "MAX": [1,10] }"#,
        ] {
            let raw: RawControl = serde_json::from_str(json)?;
            defs.push(ControlDef::from_raw(&raw)?);
        }
        Ok(ControlBank::new(defs)?)
    }

    struct Rig {
        scene: ControlBank,
        meta: ControlBank,
        engine: Engine,
        out: Vec<Output>,
    }

    impl Rig {
        fn new() -> Result<Self> {
            Ok(Self {
                scene: scene()?,
                meta: strobe_scene::meta::bank()?,
                engine: Engine::default(),
                out: Vec::new(),
            })
        }

        fn map(&mut self, controller: u8, target: Target) -> &mut Mapping {
            let source = Source::MidiCc {
                device: None,
                channel: 1,
                controller,
            };
            self.engine.mappings.push(Mapping::new(source, target));
            let last = self.engine.mappings.len() - 1;
            &mut self.engine.mappings[last]
        }

        fn cc(&mut self, controller: u8, value: u8) -> Outcome {
            self.send(
                Source::MidiCc {
                    device: Some("pad".into()),
                    channel: 1,
                    controller,
                },
                Signal::Analog(f32::from(value) / 127.0),
            )
        }

        fn note(&mut self, note: u8, down: bool) -> Outcome {
            self.send(
                Source::MidiNote {
                    device: Some("pad".into()),
                    channel: 1,
                    note,
                },
                Signal::Button(down),
            )
        }

        fn send(&mut self, source: Source, signal: Signal) -> Outcome {
            let mut banks = Banks {
                scene: Some(&mut self.scene),
                meta: Some(&mut self.meta),
            };
            self.engine
                .handle(&source, signal, &mut banks, &mut self.out)
        }

        fn osc(&mut self, address: &str, signal: Signal) -> Outcome {
            self.send(
                Source::Osc {
                    address: address.into(),
                },
                signal,
            )
        }

        fn value(&self, name: &str) -> Result<f32> {
            let i = self.scene.find(name).ok_or_else(|| anyhow!("no {name}"))?;
            Ok(self.scene.target(i).ok_or_else(|| anyhow!("target"))?[0])
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn a_fader_drives_a_slider_across_its_range() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(7, Target::control("speed")).takeover = Takeover::Jump;
        assert_eq!(rig.cc(7, 127), Outcome::Applied(1), "outcome");
        assert!(
            near(rig.value("speed")?, 10.0),
            "top: {}",
            rig.value("speed")?
        );
        rig.cc(7, 0);
        assert!(near(rig.value("speed")?, 0.0), "bottom");
        Ok(())
    }

    #[test]
    fn range_curve_and_invert_shape_what_the_control_gets() -> Result<()> {
        let mut rig = Rig::new()?;
        let m = rig.map(7, Target::control("speed"));
        m.takeover = Takeover::Jump;
        m.range = [0.0, 0.5];
        m.curve = Curve::Exp;
        m.invert = true;
        rig.cc(7, 0);
        // travel 0 inverted is 1; exp(1) is 1; times the range's top (0.5) of 10.
        assert!(near(rig.value("speed")?, 5.0), "{}", rig.value("speed")?);
        rig.cc(7, 127);
        assert!(
            near(rig.value("speed")?, 0.0),
            "inverted top is the low end"
        );
        Ok(())
    }

    #[test]
    fn writes_go_through_the_bank_so_smoothing_still_eases() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(1, Target::control("glow")).takeover = Takeover::Jump;
        rig.cc(1, 127);
        let i = rig.scene.find("glow").ok_or_else(|| anyhow!("glow"))?;
        assert!(near(rig.value("glow")?, 1.0), "the target is set at once");
        rig.scene.tick(1.0 / 60.0);
        let output = rig.scene.output(i).ok_or_else(|| anyhow!("output"))?[0];
        assert!(
            output > 0.0 && output < 0.5,
            "the output eases toward it: {output}"
        );
        Ok(())
    }

    #[test]
    fn pickup_ignores_the_fader_until_it_reaches_the_control() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(7, Target::control("speed"));
        // speed is at 5 of 10 (0.5). A fader at the bottom must not drag it.
        assert_eq!(rig.cc(7, 0), Outcome::Waiting, "far below");
        assert!(near(rig.value("speed")?, 5.0), "unchanged");
        assert_eq!(rig.cc(7, 30), Outcome::Waiting, "approaching");
        assert!(near(rig.value("speed")?, 5.0), "still unchanged");
        // Sweeping past 0.5 crosses it: from here the fader drives.
        assert_eq!(rig.cc(7, 100), Outcome::Applied(1), "crossed");
        assert!(
            near(rig.value("speed")?, 10.0 * 100.0 / 127.0),
            "{}",
            rig.value("speed")?
        );
        Ok(())
    }

    #[test]
    fn a_change_from_the_ui_makes_the_fader_pick_the_control_up_again() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(7, Target::control("speed"));
        rig.cc(7, 64);
        assert!(near(rig.value("speed")?, 5.04), "engaged at the middle");
        let speed = rig.scene.find("speed").ok_or_else(|| anyhow!("speed"))?;
        rig.scene.set(speed, None, &[1.0]);
        assert_eq!(
            rig.cc(7, 70),
            Outcome::Waiting,
            "a recalled preset moved it; the fader must catch up"
        );
        assert!(near(rig.value("speed")?, 1.0), "not dragged");
        Ok(())
    }

    #[test]
    fn a_pad_toggles_a_toggle_and_momentary_follows_the_pad() -> Result<()> {
        let mut rig = Rig::new()?;
        let source = Source::MidiNote {
            device: None,
            channel: 1,
            note: 36,
        };
        rig.engine
            .mappings
            .push(Mapping::new(source, Target::control("on")));
        rig.note(36, true);
        rig.note(36, false);
        assert!(near(rig.value("on")?, 1.0), "latched on after one press");
        rig.note(36, true);
        rig.note(36, false);
        assert!(near(rig.value("on")?, 0.0), "second press turns it off");
        rig.engine.mappings[0].button = ButtonMode::Momentary;
        rig.note(36, true);
        assert!(near(rig.value("on")?, 1.0), "on while held");
        rig.note(36, false);
        assert!(near(rig.value("on")?, 0.0), "off on release");
        Ok(())
    }

    #[test]
    fn a_pad_presses_a_bang_once_per_press() -> Result<()> {
        let mut rig = Rig::new()?;
        let source = Source::MidiNote {
            device: None,
            channel: 1,
            note: 37,
        };
        rig.engine
            .mappings
            .push(Mapping::new(source, Target::control("flash")));
        rig.note(37, true);
        rig.note(37, false);
        rig.note(37, true);
        rig.scene.tick(1.0 / 60.0);
        let i = rig.scene.find("flash").ok_or_else(|| anyhow!("flash"))?;
        let output = rig.scene.output(i).ok_or_else(|| anyhow!("out"))?[0];
        assert!(near(output, 1.0), "the bang fires for its frame: {output}");
        rig.scene.tick(1.0 / 60.0);
        let output = rig.scene.output(i).ok_or_else(|| anyhow!("out"))?[0];
        assert!(near(output, 0.0), "and only that frame");
        Ok(())
    }

    #[test]
    fn a_fader_can_drive_a_toggle_and_a_dropdown() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(1, Target::control("on"));
        rig.map(2, Target::control("mode")).takeover = Takeover::Jump;
        rig.cc(1, 127);
        assert!(near(rig.value("on")?, 1.0), "above the threshold is on");
        rig.cc(1, 0);
        assert!(near(rig.value("on")?, 0.0), "below is off");
        rig.cc(2, 127);
        assert!(near(rig.value("mode")?, 2.0), "top picks the last option");
        rig.cc(2, 64);
        assert!(near(rig.value("mode")?, 1.0), "middle picks the middle one");
        Ok(())
    }

    #[test]
    fn a_pad_steps_a_dropdown_and_wraps() -> Result<()> {
        let mut rig = Rig::new()?;
        let source = Source::MidiNote {
            device: None,
            channel: 1,
            note: 40,
        };
        rig.engine
            .mappings
            .push(Mapping::new(source, Target::control("mode")));
        for expected in [1.0, 2.0, 0.0] {
            rig.note(40, true);
            rig.note(40, false);
            assert!(near(rig.value("mode")?, expected), "steps to {expected}");
        }
        Ok(())
    }

    #[test]
    fn one_fader_drives_one_component_of_a_pad() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(
            5,
            Target::Control {
                name: "pad".into(),
                component: Some(1),
            },
        )
        .takeover = Takeover::Jump;
        rig.cc(5, 127);
        let i = rig.scene.find("pad").ok_or_else(|| anyhow!("pad"))?;
        let target = rig.scene.target(i).ok_or_else(|| anyhow!("t"))?;
        assert!(near(target[0], 0.0), "x untouched");
        assert!(near(target[1], 10.0), "y driven: {}", target[1]);
        Ok(())
    }

    #[test]
    fn a_mapping_without_a_component_on_a_pad_does_nothing_and_says_why() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(5, Target::control("pad"));
        let outcome = rig.cc(5, 100);
        let Outcome::Unavailable(why) = outcome else {
            return Err(anyhow!("expected unavailable, got {outcome:?}"));
        };
        assert!(why.contains("components"), "{why}");
        Ok(())
    }

    #[test]
    fn a_missing_control_is_unavailable_and_changes_nothing() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(7, Target::control("not_in_this_scene"));
        let outcome = rig.cc(7, 100);
        let Outcome::Unavailable(why) = outcome else {
            return Err(anyhow!("expected unavailable, got {outcome:?}"));
        };
        assert!(why.contains("not_in_this_scene"), "{why}");
        assert!(near(rig.value("speed")?, 5.0), "nothing moved");
        Ok(())
    }

    #[test]
    fn no_scene_playing_makes_scene_targets_unavailable_but_meta_still_works() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(1, Target::control("speed"));
        rig.map(2, Target::meta("hue")).takeover = Takeover::Jump;
        let mut banks = Banks {
            scene: None,
            meta: Some(&mut rig.meta),
        };
        let source = |controller| Source::MidiCc {
            device: Some("d".into()),
            channel: 1,
            controller,
        };
        let a = rig
            .engine
            .handle(&source(1), Signal::Analog(1.0), &mut banks, &mut rig.out);
        assert!(matches!(a, Outcome::Unavailable(_)), "scene target: {a:?}");
        let b = rig
            .engine
            .handle(&source(2), Signal::Analog(1.0), &mut banks, &mut rig.out);
        assert_eq!(b, Outcome::Applied(1), "meta target");
        let hue = rig.meta.find("hue").ok_or_else(|| anyhow!("hue"))?;
        assert!(
            near(rig.meta.target(hue).ok_or_else(|| anyhow!("t"))?[0], 1.0),
            "hue moved"
        );
        Ok(())
    }

    #[test]
    fn commands_fire_on_the_press_edge_only_even_from_a_shaky_fader() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(9, Target::command("next_scene"));
        for value in [10, 70, 80, 62, 66, 80, 20, 100] {
            rig.cc(9, value);
        }
        assert_eq!(
            rig.out,
            vec![
                Output::Command("next_scene".into()),
                Output::Command("next_scene".into())
            ],
            "two presses: the wobble around the line did not machine-gun"
        );
        Ok(())
    }

    #[test]
    fn cue_fav_and_preset_targets_produce_outputs() -> Result<()> {
        let mut rig = Rig::new()?;
        for (note, target) in [
            (60, Target::Cue { number: 4 }),
            (61, Target::FavSlot { slot: 2 }),
            (
                62,
                Target::Preset {
                    name: "slow".into(),
                },
            ),
        ] {
            let source = Source::MidiNote {
                device: None,
                channel: 1,
                note,
            };
            rig.engine.mappings.push(Mapping::new(source, target));
            rig.note(note, true);
            rig.note(note, false);
        }
        assert_eq!(
            rig.out,
            vec![
                Output::Cue(4),
                Output::FavSlot(2),
                Output::Preset("slow".into())
            ],
            "outputs in order"
        );
        Ok(())
    }

    #[test]
    fn a_macro_target_drives_the_macros_control_within_its_range() -> Result<()> {
        let mut rig = Rig::new()?;
        let mut m = Macro::new("Speed", Target::control("speed"));
        m.range = Some([0.0, 0.5]);
        let id = rig.engine.macros.add(m);
        rig.map(7, Target::Macro { id }).takeover = Takeover::Jump;
        rig.cc(7, 127);
        assert!(
            near(rig.value("speed")?, 5.0),
            "the top of the macro's range: {}",
            rig.value("speed")?
        );
        rig.engine.macros.set_enabled(id, false);
        assert!(
            matches!(rig.cc(7, 0), Outcome::Unavailable(_)),
            "a disabled macro ignores the mapping"
        );
        Ok(())
    }

    #[test]
    fn a_disabled_mapping_is_silent() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(7, Target::control("speed")).enabled = false;
        assert_eq!(rig.cc(7, 100), Outcome::Unmapped, "off");
        Ok(())
    }

    #[test]
    fn a_device_specific_mapping_ignores_other_devices() -> Result<()> {
        let mut rig = Rig::new()?;
        let m = rig.map(7, Target::control("speed"));
        m.takeover = Takeover::Jump;
        m.source = Source::MidiCc {
            device: Some("other".into()),
            channel: 1,
            controller: 7,
        };
        assert_eq!(rig.cc(7, 100), Outcome::Unmapped, "pad is not other");
        Ok(())
    }

    #[test]
    fn unmapped_midi_is_just_unmapped() -> Result<()> {
        let mut rig = Rig::new()?;
        assert_eq!(rig.cc(99, 1), Outcome::Unmapped, "nothing");
        assert!(rig.out.is_empty(), "no outputs");
        Ok(())
    }

    // -------------------------------------------------------- built-in OSC

    #[test]
    fn the_builtin_osc_space_drives_controls_meta_and_commands_with_no_mapping() -> Result<()> {
        let mut rig = Rig::new()?;
        assert_eq!(
            rig.osc("/strobe/control/speed", Signal::Analog(0.5)),
            Outcome::Applied(1),
            "control"
        );
        assert!(near(rig.value("speed")?, 5.0), "jumped to half");
        rig.osc("/strobe/meta/hue", Signal::Analog(0.25));
        let hue = rig.meta.find("hue").ok_or_else(|| anyhow!("hue"))?;
        assert!(
            near(rig.meta.target(hue).ok_or_else(|| anyhow!("t"))?[0], 0.25),
            "meta"
        );
        rig.osc("/strobe/cmd/next_scene", Signal::Button(true));
        rig.osc("/strobe/cmd/next_scene", Signal::Analog(1.0));
        rig.osc("/strobe/cmd/next_scene", Signal::Analog(0.0));
        assert_eq!(
            rig.out,
            vec![
                Output::Command("next_scene".into()),
                Output::Command("next_scene".into())
            ],
            "a press fires; a zero does not"
        );
        Ok(())
    }

    #[test]
    fn builtin_toggles_follow_their_argument_and_bangs_press() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.osc("/strobe/control/on", Signal::Analog(1.0));
        assert!(near(rig.value("on")?, 1.0), "on");
        rig.osc("/strobe/control/on", Signal::Analog(0.0));
        assert!(near(rig.value("on")?, 0.0), "off");
        rig.osc("/strobe/control/on", Signal::Button(true));
        assert!(near(rig.value("on")?, 1.0), "true is on");
        rig.osc("/strobe/control/on", Signal::Button(false));
        assert!(near(rig.value("on")?, 0.0), "false is off");
        assert_eq!(
            rig.osc("/strobe/control/flash", Signal::Button(true)),
            Outcome::Applied(1),
            "bang"
        );
        Ok(())
    }

    #[test]
    fn a_mapping_for_an_address_beats_its_builtin_meaning_and_the_space_can_be_off() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.engine.mappings.push(Mapping::new(
            Source::Osc {
                address: "/strobe/control/speed".into(),
            },
            Target::control("glow"),
        ));
        rig.engine.mappings[0].takeover = Takeover::Jump;
        rig.osc("/strobe/control/speed", Signal::Analog(1.0));
        assert!(near(rig.value("glow")?, 1.0), "the mapping drove glow");
        assert!(near(rig.value("speed")?, 5.0), "and not speed");
        rig.engine.builtin_osc = false;
        assert_eq!(
            rig.osc("/strobe/control/speed", Signal::Analog(0.0)),
            Outcome::Applied(1),
            "mapped address still works"
        );
        assert_eq!(
            rig.osc("/strobe/meta/hue", Signal::Analog(1.0)),
            Outcome::Unmapped,
            "built-ins are off"
        );
        Ok(())
    }

    #[test]
    fn an_unknown_control_over_osc_is_unavailable() -> Result<()> {
        let mut rig = Rig::new()?;
        assert!(
            matches!(
                rig.osc("/strobe/control/nope", Signal::Analog(1.0)),
                Outcome::Unavailable(_)
            ),
            "reported, not applied"
        );
        Ok(())
    }

    // -------------------------------------------------------------- learn

    #[test]
    fn learn_binds_the_next_message_and_then_the_mapping_drives_the_target() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.engine.arm_new(Target::control("speed"));
        assert!(
            rig.engine.is_learning_target(&Target::control("speed")),
            "armed"
        );
        assert_eq!(rig.cc(21, 64), Outcome::Learned(0), "bound");
        assert!(rig.engine.armed().is_none(), "learn is over");
        let m = &rig.engine.mappings[0];
        assert_eq!(
            m.source,
            Source::MidiCc {
                device: Some("pad".into()),
                channel: 1,
                controller: 21
            },
            "the source remembers the device"
        );
        assert!(
            near(rig.value("speed")?, 5.0),
            "learning did not move the control"
        );
        // The fader is at the control's value, so it has picked it up.
        rig.cc(21, 66);
        rig.cc(21, 100);
        assert!(
            rig.value("speed")? > 7.0,
            "now it drives: {}",
            rig.value("speed")?
        );
        Ok(())
    }

    #[test]
    fn a_note_off_cannot_be_learned_so_a_pad_binds_on_its_press() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.engine.arm_new(Target::command("toggle_blackout"));
        assert_eq!(rig.note(36, false), Outcome::Ignored, "a stray release");
        assert!(rig.engine.armed().is_some(), "still armed");
        assert_eq!(rig.note(36, true), Outcome::Learned(0), "the press binds");
        Ok(())
    }

    #[test]
    fn osc_learn_binds_an_address_with_jump_takeover() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.engine.arm_new(Target::control("speed"));
        rig.osc("/1/fader1", Signal::Analog(0.3));
        assert_eq!(
            rig.engine.mappings[0].source,
            Source::Osc {
                address: "/1/fader1".into()
            },
            "address"
        );
        assert_eq!(rig.engine.mappings[0].takeover, Takeover::Jump, "OSC jumps");
        Ok(())
    }

    #[test]
    fn learning_a_source_that_is_taken_reports_a_conflict_and_changes_nothing() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(21, Target::control("speed"));
        rig.engine.arm_new(Target::control("glow"));
        assert_eq!(rig.cc(21, 64), Outcome::Conflict, "reported");
        assert_eq!(rig.engine.mappings.len(), 1, "no second mapping");
        let conflict = rig
            .engine
            .conflict()
            .ok_or_else(|| anyhow!("a pending conflict"))?;
        assert_eq!(conflict.existing, 0, "names the holder");
        assert!(rig.engine.armed().is_none(), "learn stopped");
        Ok(())
    }

    #[test]
    fn answering_a_conflict_with_cancel_leaves_everything_as_it_was() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(21, Target::control("speed"));
        rig.engine.arm_new(Target::control("glow"));
        rig.cc(21, 64);
        assert_eq!(rig.engine.resolve_conflict(false), None, "cancelled");
        assert!(rig.engine.conflict().is_none(), "answered");
        assert_eq!(
            rig.engine.mappings[0].target,
            Target::control("speed"),
            "unchanged"
        );
        Ok(())
    }

    #[test]
    fn answering_a_conflict_with_replace_rebinds_the_source() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(21, Target::control("speed"));
        rig.map(22, Target::control("zoom"));
        rig.engine.arm_new(Target::control("glow"));
        rig.cc(21, 64);
        assert_eq!(
            rig.engine.resolve_conflict(true),
            Some(1),
            "the new mapping is last"
        );
        assert_eq!(rig.engine.mappings.len(), 2, "the old holder is gone");
        let targets: Vec<&Target> = rig.engine.mappings.iter().map(|m| &m.target).collect();
        assert_eq!(
            targets,
            vec![&Target::control("zoom"), &Target::control("glow")],
            "order"
        );
        Ok(())
    }

    #[test]
    fn relearning_a_row_keeps_its_shaping_and_moves_its_source() -> Result<()> {
        let mut rig = Rig::new()?;
        let m = rig.map(21, Target::control("speed"));
        m.range = [0.2, 0.8];
        m.curve = Curve::Log;
        rig.map(22, Target::control("glow"));
        rig.engine.arm_relearn(0);
        assert!(rig.engine.is_relearning(0), "armed on row 0");
        assert_eq!(rig.cc(30, 10), Outcome::Learned(0), "bound");
        assert_eq!(rig.engine.mappings[0].range, [0.2, 0.8], "range kept");
        assert_eq!(rig.engine.mappings[0].curve, Curve::Log, "curve kept");
        assert!(
            matches!(
                &rig.engine.mappings[0].source,
                Source::MidiCc { controller: 30, .. }
            ),
            "new source"
        );
        Ok(())
    }

    #[test]
    fn relearning_a_rows_own_source_is_not_a_conflict() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(21, Target::control("speed"));
        rig.engine.arm_relearn(0);
        assert_eq!(rig.cc(21, 10), Outcome::Learned(0), "same CC, same row");
        Ok(())
    }

    #[test]
    fn learning_the_same_source_for_the_same_target_is_a_quiet_success() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.engine.arm_new(Target::control("speed"));
        rig.cc(21, 1);
        rig.engine.arm_new(Target::control("speed"));
        assert_eq!(rig.cc(21, 1), Outcome::Learned(0), "already so");
        assert_eq!(rig.engine.mappings.len(), 1, "not duplicated");
        Ok(())
    }

    #[test]
    fn cancelling_learn_lets_messages_through_again() -> Result<()> {
        let mut rig = Rig::new()?;
        rig.map(21, Target::control("speed")).takeover = Takeover::Jump;
        rig.engine.arm_new(Target::control("glow"));
        rig.engine.cancel_learn();
        assert_eq!(rig.cc(21, 127), Outcome::Applied(1), "normal again");
        Ok(())
    }

    #[test]
    fn a_macro_card_can_be_learned_like_any_target() -> Result<()> {
        let mut rig = Rig::new()?;
        let id = rig
            .engine
            .macros
            .add(Macro::new("Speed", Target::control("speed")));
        rig.engine.arm_new(Target::Macro { id });
        rig.cc(40, 0);
        assert_eq!(
            rig.engine.mappings[0].target,
            Target::Macro { id },
            "bound to the macro"
        );
        Ok(())
    }
}
