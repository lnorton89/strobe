// ============================================================================
// Module:       strobe_app::remote_controls
// Description:  The controls as data over the desktop remote: the host
//               publishes its banks and applies the controller's edits; the
//               controller keeps a copy of each bank, draws it itself and
//               sends back what changed.
//
// Dependencies: strobe-remote (desktop::{Desktop, SyncedBank, ControlEdit});
//               strobe-scene (ControlBank, ControlDef, RawControl);
//               crate::ui::controls; egui; serde_json
// ============================================================================

//! Controlling another Strobe without a picture for every change (D513).
//!
//! The desktop remote mirrors the host's whole control window as JPEG
//! frames (D275). That is right for the editor and settings, and slow for
//! the thing done most during a show: moving a control. Every step of a
//! drag waited for the controller's input batch, the host's next control
//! frame and the next captured picture, a few hundred milliseconds, on a
//! host whose CPU also had to compress each picture.
//!
//! So the controls also travel as data:
//!
//! - **The host** ([`Publisher`]) publishes the playing scene's bank and the
//!   output's bank to the desktop holding the lease, once it asks: each
//!   bank's definitions as `scene.json` entries the first time a connection
//!   sees them, and every control's target after that, at most every
//!   [`PUBLISH_PERIOD`] and only when something moved. Each bank has an id
//!   that changes with its definitions, so an edit made against a scene
//!   that has since been replaced is dropped rather than applied to
//!   whatever control now sits at its index.
//! - **The controller** ([`Mirror`]) rebuilds each bank from the
//!   definitions and draws it with the same rows the host's Controls page
//!   uses ([`crate::ui::controls::rows`]). What the person changed goes back
//!   as [`ControlEdit`]s: a control's new target, or a bang's press. A
//!   value the host sends back for a control the person moved within
//!   [`HOLD_LOCAL`] is not taken, so a drag does not jump back to an echo
//!   of where it was a moment ago.

use std::time::{Duration, Instant};

use strobe_remote::desktop::{ControlEdit, Desktop, MAX_SYNCED_CONTROLS, SyncedBank};
use strobe_scene::control::{ControlBank, ControlDef, ControlKind};

/// The playing scene's banks, as the host lends them to [`Publisher::tick`].
pub struct Live<'a> {
    /// The scene's title, which heads its bank on the controller.
    pub title: &'a str,
    /// The scene's own controls.
    pub scene: &'a mut ControlBank,
    /// The output's controls (D180).
    pub output: &'a mut ControlBank,
}

/// How often the host publishes moved values at most: 20 times a second,
/// as fast as a hand reads a slider and well under the output's rate.
pub const PUBLISH_PERIOD: Duration = Duration::from_millis(50);

/// How long a control the person moved here ignores the host's values for
/// it: longer than an edit's round trip, so the echo of an older value
/// cannot pull the control back mid-drag.
pub const HOLD_LOCAL: Duration = Duration::from_millis(600);

/// The heading of the output's bank (D180).
const OUTPUT_TITLE: &str = "Output";

/// One bank as the host last published it.
#[derive(Default)]
struct Slot {
    /// The id the definitions below were given; 0 before the first.
    id: u64,
    defs: Vec<ControlDef>,
    /// `defs` as `scene.json` entries, made once per id.
    json: Vec<serde_json::Value>,
    values: Vec<[f32; 3]>,
}

impl Slot {
    /// Brings the slot up to `bank`: a new id if the definitions changed.
    /// Returns whether anything did.
    fn refresh(&mut self, bank: &ControlBank, next_id: &mut u64) -> bool {
        let mut changed = false;
        if self.id == 0 || self.defs.as_slice() != bank.defs() {
            *next_id += 1;
            self.id = *next_id;
            self.defs = bank.defs().to_vec();
            self.json = self.defs.iter().map(ControlDef::to_json).collect();
            changed = true;
        }
        let count = bank.defs().len();
        if self.values.len() != count {
            self.values.resize(count, [0.0; 3]);
            changed = true;
        }
        for (index, slot) in self.values.iter_mut().enumerate() {
            let target = bank.target(index).copied().unwrap_or_default();
            if *slot != target {
                *slot = target;
                changed = true;
            }
        }
        changed
    }

    fn synced(&self, title: &str) -> SyncedBank {
        SyncedBank {
            id: self.id,
            title: title.to_string(),
            defs: Some(self.json.clone()),
            values: self.values.clone(),
        }
    }
}

/// The host's half: publishes the banks while a controller wants them, and
/// applies its edits.
#[derive(Default)]
pub struct Publisher {
    scene: Slot,
    output: Slot,
    next_id: u64,
    due: Option<Instant>,
    edits: Vec<ControlEdit>,
}

impl Publisher {
    /// Once a frame, with the desktop bridge while the remote runs. Edits
    /// are applied first, so the publication that follows carries them.
    pub fn tick(&mut self, desktop: &Desktop, live: Option<Live<'_>>, now: Instant) {
        self.edits.clear();
        desktop.drain_controls(&mut self.edits);
        let Some(live) = live else {
            return;
        };
        // An id stands for the controls published under it: if the bank now
        // holds others (a scene loaded this frame), its edits are stale.
        let scene_same = self.scene.defs.as_slice() == live.scene.defs();
        let output_same = self.output.defs.as_slice() == live.output.defs();
        for edit in &self.edits {
            let bank = if edit.bank == self.scene.id && scene_same {
                &mut *live.scene
            } else if edit.bank == self.output.id && output_same {
                &mut *live.output
            } else {
                // Made against controls that are gone: another scene, or
                // the scene's own controls edited since.
                log::debug!("remote controls: an edit for bank {} dropped", edit.bank);
                continue;
            };
            bank.set(edit.index as usize, None, &edit.values);
        }
        if !desktop.controls_wanted() {
            return;
        }
        // A new owner's first publication goes out at once and whole.
        let fresh = desktop.controls().is_none();
        if !fresh && self.due.is_some_and(|due| now < due) {
            return;
        }
        let scene_moved = self.scene.refresh(live.scene, &mut self.next_id);
        let output_moved = self.output.refresh(live.output, &mut self.next_id);
        if !(fresh || scene_moved || output_moved) {
            return;
        }
        self.due = Some(now + PUBLISH_PERIOD);
        desktop.publish_controls(vec![
            self.scene.synced(live.title),
            self.output.synced(OUTPUT_TITLE),
        ]);
    }
}

/// One bank as the controller holds it.
struct Copy {
    id: u64,
    title: String,
    bank: ControlBank,
    /// When the person last moved each control here.
    touched: Vec<Option<Instant>>,
    /// Each bang's presses seen, so a new one is told from them.
    presses: Vec<u32>,
}

/// The controller's half: a copy of each of the host's banks, drawn here.
#[derive(Default)]
pub struct Mirror {
    banks: Vec<Copy>,
    revision: Option<u64>,
}

impl Mirror {
    /// Whether there is anything to draw.
    pub fn is_empty(&self) -> bool {
        self.banks.is_empty()
    }

    /// Takes a publication from the host. `banks` carry their definitions
    /// whenever their id is new to this copy (the worker keeps them from
    /// earlier publications). A bank that cannot be rebuilt is left out,
    /// and said once.
    pub fn take(&mut self, revision: u64, banks: &[SyncedBank], now: Instant) {
        if self.revision == Some(revision) {
            return;
        }
        self.revision = Some(revision);
        let mut next = Vec::with_capacity(banks.len());
        for synced in banks {
            let kept = self
                .banks
                .iter()
                .position(|copy| copy.id == synced.id)
                .map(|at| self.banks.swap_remove(at));
            let mut copy = match kept {
                Some(copy) => copy,
                None => match rebuild(synced) {
                    Ok(copy) => copy,
                    Err(error) => {
                        log::warn!("remote controls: cannot show \"{}\": {error}", synced.title);
                        continue;
                    }
                },
            };
            copy.title.clone_from(&synced.title);
            for (index, values) in synced.values.iter().enumerate() {
                let held = copy
                    .touched
                    .get(index)
                    .copied()
                    .flatten()
                    .is_some_and(|at| now.saturating_duration_since(at) < HOLD_LOCAL);
                if held {
                    continue;
                }
                let is_bang = copy
                    .bank
                    .defs()
                    .get(index)
                    .is_some_and(|def| matches!(def.kind, ControlKind::Bang(_)));
                if !is_bang && copy.bank.target(index) != Some(values) {
                    copy.bank.set(index, None, values);
                }
            }
            next.push(copy);
        }
        self.banks = next;
    }

    /// Draws every bank, its groups under their names, and adds what the
    /// person changed to `edits`. `dt` eases the copy's outputs, which only
    /// the drawing reads.
    pub fn show(&mut self, ui: &mut egui::Ui, dt: f32, now: Instant, edits: &mut Vec<ControlEdit>) {
        let theme = crate::ui::theme::palette();
        let scale = crate::ui::controls::Scale::Panel;
        for (slot, copy) in self.banks.iter_mut().enumerate() {
            if copy.bank.defs().is_empty() {
                continue;
            }
            ui.push_id(("remote bank", slot), |ui| {
                ui.label(egui::RichText::new(&copy.title).strong());
                let before: Vec<[f32; 3]> = (0..copy.bank.defs().len())
                    .map(|index| copy.bank.target(index).copied().unwrap_or_default())
                    .collect();
                for (group, members) in crate::ui::controls::group_order(copy.bank.defs()) {
                    if members.is_empty() {
                        continue;
                    }
                    if let Some(group) = &group {
                        ui.label(
                            egui::RichText::new(crate::ui::controls::display_name(group))
                                .small()
                                .color(crate::ui::theme::rgb(theme.text_muted)),
                        );
                    }
                    ui.push_id(("group", group.as_deref().unwrap_or("")), |ui| {
                        crate::ui::controls::rows(
                            ui,
                            &theme,
                            &mut copy.bank,
                            &members,
                            None,
                            scale,
                        );
                    });
                }
                copy.settle(&before, dt, now, edits);
                ui.add_space(8.0);
            });
        }
    }
}

impl Copy {
    /// After the rows drew: what the person changed since `before` (each
    /// control's target then) becomes an edit, and the copy eases on by
    /// `dt`.
    fn settle(&mut self, before: &[[f32; 3]], dt: f32, now: Instant, edits: &mut Vec<ControlEdit>) {
        for (index, was) in before.iter().enumerate() {
            let presses = self.bank.presses(index);
            let seen = self.presses.get(index).copied().unwrap_or_default();
            let target = self.bank.target(index).copied().unwrap_or_default();
            let dims = self.bank.defs().get(index).map_or(1, ControlDef::dims);
            let values = if presses > seen {
                Vec::new()
            } else if target != *was {
                target.get(..dims).unwrap_or_default().to_vec()
            } else {
                continue;
            };
            if let Some(touched) = self.touched.get_mut(index) {
                *touched = Some(now);
            }
            edits.push(ControlEdit {
                bank: self.id,
                index: index as u32,
                values,
            });
        }
        self.bank.tick(dt);
        for (index, seen) in self.presses.iter_mut().enumerate() {
            *seen = self.bank.presses(index);
        }
    }
}

/// A copy of `synced` rebuilt from its definitions.
fn rebuild(synced: &SyncedBank) -> Result<Copy, String> {
    let json = synced
        .defs
        .as_ref()
        .ok_or("the host sent values for controls it has not described")?;
    if json.len() > MAX_SYNCED_CONTROLS {
        return Err(format!("{} controls is more than a bank holds", json.len()));
    }
    let defs = json
        .iter()
        .map(|value| {
            let raw: strobe_scene::json::RawControl =
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
            ControlDef::from_raw(&raw).map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    let count = defs.len();
    let bank = ControlBank::new(defs).map_err(|error| error.to_string())?;
    Ok(Copy {
        id: synced.id,
        title: synced.title.clone(),
        bank,
        touched: vec![None; count],
        presses: vec![0; count],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn bank(controls: &[&str]) -> Result<ControlBank> {
        let defs = controls
            .iter()
            .map(|json| {
                let raw: strobe_scene::json::RawControl = serde_json::from_str(json)?;
                Ok(ControlDef::from_raw(&raw)?)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ControlBank::new(defs)?)
    }

    const SPEED: &str =
        r#"{ "NAME": "speed", "TYPE": "slider", "MIN": 0, "MAX": 2, "DEFAULT": 1 }"#;
    const FLASH: &str = r#"{ "NAME": "flash", "TYPE": "bang" }"#;
    const TINT: &str = r#"{ "NAME": "tint", "TYPE": "color", "DEFAULT": [1, 0, 0] }"#;

    /// A desktop whose owner (connection 1) asked for the controls.
    fn desktop() -> Result<Desktop> {
        let desktop = Desktop::default();
        desktop.enable(true);
        anyhow::ensure!(desktop.acquire(1), "the owner's lease");
        anyhow::ensure!(desktop.want_controls(1), "the owner asks");
        Ok(desktop)
    }

    fn published(desktop: &Desktop) -> Result<Vec<SyncedBank>> {
        desktop
            .controls()
            .map(|(_, banks)| banks.to_vec())
            .ok_or_else(|| anyhow::anyhow!("nothing published"))
    }

    fn edit(bank: u64, index: u32, values: &[f32]) -> ControlEdit {
        ControlEdit {
            bank,
            index,
            values: values.to_vec(),
        }
    }

    #[test]
    fn the_host_publishes_both_banks_and_applies_edits_to_the_right_one() -> Result<()> {
        let desktop = desktop()?;
        let mut scene = bank(&[SPEED, FLASH])?;
        let mut output = bank(&[TINT])?;
        let mut publisher = Publisher::default();
        let start = Instant::now();
        publisher.tick(
            &desktop,
            Some(Live {
                title: "Acid Mirror",
                scene: &mut scene,
                output: &mut output,
            }),
            start,
        );
        let banks = published(&desktop)?;
        let titles: Vec<&str> = banks.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Acid Mirror", OUTPUT_TITLE],
            "the scene's bank, then the output's"
        );
        let (scene_id, output_id) = match banks.as_slice() {
            [scene, output] => (scene.id, output.id),
            _ => anyhow::bail!("two banks"),
        };
        anyhow::ensure!(
            desktop.submit_controls(
                1,
                vec![
                    edit(scene_id, 0, &[1.5]),
                    edit(output_id, 0, &[0.0, 1.0, 0.0])
                ]
            ),
            "edits queued"
        );
        publisher.tick(
            &desktop,
            Some(Live {
                title: "Acid Mirror",
                scene: &mut scene,
                output: &mut output,
            }),
            start + PUBLISH_PERIOD,
        );
        assert_eq!(
            scene.target(0).map(|t| t[0]),
            Some(1.5),
            "the scene's speed moved"
        );
        assert_eq!(
            output.target(0).copied(),
            Some([0.0, 1.0, 0.0]),
            "the output's tint moved"
        );
        let speed = published(&desktop)?
            .first()
            .and_then(|bank| bank.values.first().copied());
        assert_eq!(speed.map(|v| v[0]), Some(1.5), "and the host says so");
        Ok(())
    }

    #[test]
    fn an_edit_for_controls_that_were_replaced_is_dropped() -> Result<()> {
        let desktop = desktop()?;
        let mut scene = bank(&[SPEED])?;
        let mut output = bank(&[TINT])?;
        let mut publisher = Publisher::default();
        let start = Instant::now();
        publisher.tick(
            &desktop,
            Some(Live {
                title: "One",
                scene: &mut scene,
                output: &mut output,
            }),
            start,
        );
        let before = published(&desktop)?
            .first()
            .map(|bank| bank.id)
            .ok_or_else(|| anyhow::anyhow!("a scene bank"))?;
        // Another scene is loaded, its first control a bang where speed was.
        let mut other = bank(&[FLASH, SPEED])?;
        anyhow::ensure!(
            desktop.submit_controls(1, vec![edit(before, 0, &[2.0])]),
            "edit queued"
        );
        publisher.tick(
            &desktop,
            Some(Live {
                title: "Two",
                scene: &mut other,
                output: &mut output,
            }),
            start + PUBLISH_PERIOD,
        );
        assert_eq!(other.presses(0), 0, "the old scene's edit pressed nothing");
        assert_eq!(
            other.target(1).map(|t| t[0]),
            Some(1.0),
            "nor moved anything"
        );
        let after = published(&desktop)?.first().map(|bank| bank.id);
        assert_ne!(after, Some(before), "the new controls have a new id");
        Ok(())
    }

    #[test]
    fn values_are_published_no_faster_than_the_period_and_only_when_moved() -> Result<()> {
        let desktop = desktop()?;
        let mut scene = bank(&[SPEED])?;
        let mut output = bank(&[TINT])?;
        let mut publisher = Publisher::default();
        let start = Instant::now();
        let mut tick = |scene: &mut ControlBank, at: Instant| {
            publisher.tick(
                &desktop,
                Some(Live {
                    title: "S",
                    scene,
                    output: &mut output,
                }),
                at,
            );
            desktop.controls().map(|(revision, _)| revision)
        };
        let first = tick(&mut scene, start);
        scene.set(0, None, &[0.5]);
        assert_eq!(
            tick(&mut scene, start + Duration::from_millis(10)),
            first,
            "too soon"
        );
        let second = tick(&mut scene, start + PUBLISH_PERIOD);
        assert_ne!(second, first, "due, and moved");
        assert_eq!(
            tick(&mut scene, start + PUBLISH_PERIOD * 3),
            second,
            "due, but nothing moved"
        );
        Ok(())
    }

    fn synced(host: &ControlBank, id: u64, with_defs: bool) -> SyncedBank {
        SyncedBank {
            id,
            title: "Scene".into(),
            defs: with_defs.then(|| host.defs().iter().map(ControlDef::to_json).collect()),
            values: (0..host.defs().len())
                .map(|index| host.target(index).copied().unwrap_or_default())
                .collect(),
        }
    }

    #[test]
    fn the_copy_sends_what_the_person_changed_and_keeps_a_drag_from_its_echo() -> Result<()> {
        let mut host = bank(&[SPEED, FLASH])?;
        let mut mirror = Mirror::default();
        let start = Instant::now();
        mirror.take(1, &[synced(&host, 9, true)], start);
        let copy = mirror
            .banks
            .first_mut()
            .ok_or_else(|| anyhow::anyhow!("the bank was rebuilt"))?;
        assert_eq!(copy.bank.defs(), host.defs(), "the same controls");
        // The person drags speed and presses flash, as the rows would.
        let before = [[1.0; 3], [0.0; 3]];
        copy.bank.set(0, None, &[1.8]);
        copy.bank.press(1);
        let mut edits = Vec::new();
        copy.settle(&before, 0.016, start, &mut edits);
        assert_eq!(
            edits,
            vec![edit(9, 0, &[1.8]), edit(9, 1, &[])],
            "the slider's value and the bang's press"
        );
        let mut again = Vec::new();
        let now: Vec<[f32; 3]> = (0..2)
            .map(|index| copy.bank.target(index).copied().unwrap_or_default())
            .collect();
        copy.settle(&now, 0.016, start, &mut again);
        assert!(again.is_empty(), "nothing new, nothing sent: {again:?}");
        // The host's echo of an older value arrives mid-drag, then later.
        host.set(0, None, &[1.2]);
        mirror.take(2, &[synced(&host, 9, false)], start + HOLD_LOCAL / 2);
        let speed = |mirror: &Mirror| {
            mirror
                .banks
                .first()
                .and_then(|copy| copy.bank.target(0).map(|t| t[0]))
        };
        assert_eq!(speed(&mirror), Some(1.8), "the drag keeps its value");
        mirror.take(3, &[synced(&host, 9, false)], start + HOLD_LOCAL * 2);
        assert_eq!(speed(&mirror), Some(1.2), "after the hold, the host's");
        Ok(())
    }

    #[test]
    fn values_for_controls_never_described_are_not_drawn() {
        let mut mirror = Mirror::default();
        mirror.take(
            1,
            &[SyncedBank {
                id: 4,
                title: "Ghost".into(),
                defs: None,
                values: vec![[0.5; 3]],
            }],
            Instant::now(),
        );
        assert!(mirror.is_empty(), "nothing to rebuild it from");
    }
}
