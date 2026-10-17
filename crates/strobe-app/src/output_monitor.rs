// ============================================================================
// Module:       strobe_app::output_monitor
// Description:  The monitor chosen for the output, followed across monitor
//               list refreshes by what it is (name, position, size) rather
//               than by its place in the list. Pure, no winit.
//
// Dependencies: none
// ============================================================================

//! Which monitor the output belongs on (D400).
//!
//! The app re-reads the monitor list every couple of seconds. A place in
//! that list is not a monitor: unplug a projector, or let Windows renumber
//! its displays after a change, and the same place names another monitor,
//! possibly the operator's own, which F11 or the picker would then cover
//! with the show. The handle is no better, since Windows can issue a new
//! one after a display change. So the choice is kept as an [`Identity`]
//! (the name the system gives the monitor, its position on the desktop and
//! its size) and looked up again in each new list ([`Chosen::follow`]).
//!
//! - **Found as it was:** the same name, position and size.
//! - **Found changed:** exactly one monitor with the same (known) name and
//!   either the same position (its mode changed) or the same size (the
//!   desktop was rearranged). A name is needed for this, and only one
//!   candidate may fit, so a guess between two is never made.
//! - **Gone:** nothing is chosen until it comes back. F11 then does what it
//!   does when nothing was ever chosen: fullscreen on the monitor the
//!   output is on. The identity is remembered, so a projector that blinks
//!   off and on is chosen again when it returns; choosing another monitor
//!   replaces it.

/// What identifies a monitor across refreshes of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The name the system reports (`\\.\DISPLAY2` on Windows), if any.
    pub name: Option<String>,
    /// The top-left corner on the desktop, in physical pixels.
    pub position: [i32; 2],
    /// The size in physical pixels.
    pub size: [u32; 2],
}

/// What a refresh did to the choice, for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Followed {
    /// Nothing is chosen, or the chosen monitor kept its place.
    Same,
    /// The chosen monitor is at another place in the list now.
    Moved {
        /// Its old place.
        from: usize,
        /// Its new place.
        to: usize,
    },
    /// The chosen monitor is not in the list.
    Lost,
    /// The chosen monitor, gone before, is back at this place.
    Found(usize),
}

/// The output's monitor: what was chosen, and where it is in the current
/// list.
#[derive(Debug, Default)]
pub struct Chosen {
    /// What was chosen, kept while it is gone.
    identity: Option<Identity>,
    /// Its place in the current list, while it is in it.
    index: Option<usize>,
}

impl Chosen {
    /// The chosen monitor's place in the current list, if it is in it.
    pub fn index(&self) -> Option<usize> {
        self.index
    }

    /// Chooses the monitor at `index` of `monitors`. Returns whether there
    /// is one there; if not, the choice is unchanged.
    pub fn choose(&mut self, index: usize, monitors: &[Identity]) -> bool {
        let Some(monitor) = monitors.get(index) else {
            return false;
        };
        self.identity = Some(monitor.clone());
        self.index = Some(index);
        true
    }

    /// Finds the chosen monitor in a newly read list.
    pub fn follow(&mut self, monitors: &[Identity]) -> Followed {
        let Some(identity) = self.identity.as_mut() else {
            return Followed::Same;
        };
        let before = self.index;
        let found = find(identity, monitors);
        if let Some(monitor) = found.and_then(|at| monitors.get(at)) {
            // A changed mode or place is the monitor's new identity, so the
            // next change is measured from it.
            monitor.clone_into(identity);
        }
        self.index = found;
        match (before, found) {
            (Some(from), Some(to)) if from != to => Followed::Moved { from, to },
            (Some(_), Some(_)) | (None, None) => Followed::Same,
            (Some(_), None) => Followed::Lost,
            (None, Some(at)) => Followed::Found(at),
        }
    }
}

/// Where `identity` is in `monitors`: exactly, or as the one monitor with
/// its name and either its position or its size.
fn find(identity: &Identity, monitors: &[Identity]) -> Option<usize> {
    if let Some(at) = monitors.iter().position(|monitor| monitor == identity) {
        return Some(at);
    }
    let name = identity.name.as_ref()?;
    let mut candidates = monitors.iter().enumerate().filter(|(_, monitor)| {
        monitor.name.as_ref() == Some(name)
            && (monitor.position == identity.position || monitor.size == identity.size)
    });
    let (at, _) = candidates.next()?;
    candidates.next().is_none().then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn monitor(name: &str, x: i32, width: u32) -> Identity {
        Identity {
            name: Some(name.to_owned()),
            position: [x, 0],
            size: [width, 1080],
        }
    }

    /// The venue: the operator's laptop panel, then two projectors.
    fn venue() -> Vec<Identity> {
        vec![
            monitor(r"\\.\DISPLAY1", 0, 1920),
            monitor(r"\\.\DISPLAY2", 1920, 1920),
            monitor(r"\\.\DISPLAY3", 3840, 1280),
        ]
    }

    #[test]
    fn an_unplugged_monitor_never_leaves_its_place_to_another() -> Result<()> {
        let mut monitors = venue();
        let mut chosen = Chosen::default();
        assert!(chosen.choose(2, &monitors), "the third monitor exists");
        // DISPLAY2 is unplugged: the list closes up, and place 2 is gone,
        // while place 1 is now the third projector.
        monitors.remove(1);
        assert_eq!(
            chosen.follow(&monitors),
            Followed::Moved { from: 2, to: 1 },
            "it moved up when the one before it went"
        );
        assert_eq!(
            chosen.index(),
            Some(1),
            "the same projector, at its new place"
        );

        // Now the chosen one goes: nothing is chosen, not the next one along.
        let mut chosen = Chosen::default();
        let mut monitors = venue();
        assert!(chosen.choose(1, &monitors), "the second monitor exists");
        monitors.remove(1);
        assert_ne!(
            monitors.get(1),
            Some(&venue()[1]),
            "the old place names another monitor now"
        );
        assert_eq!(
            chosen.follow(&monitors),
            Followed::Lost,
            "the chosen monitor went"
        );
        assert_eq!(chosen.index(), None, "nothing chosen while it is gone");
        assert_eq!(chosen.follow(&monitors), Followed::Same, "said once");

        // Plugged back in, at the end of the list: chosen again.
        monitors.push(venue()[1].clone());
        assert_eq!(chosen.follow(&monitors), Followed::Found(2), "it came back");
        assert_eq!(chosen.index(), Some(2), "followed back");
        Ok(())
    }

    #[test]
    fn a_reordered_list_keeps_the_same_monitor() -> Result<()> {
        let mut monitors = venue();
        let mut chosen = Chosen::default();
        assert!(chosen.choose(0, &monitors), "the first monitor exists");
        monitors.reverse();
        assert_eq!(
            chosen.follow(&monitors),
            Followed::Moved { from: 0, to: 2 },
            "it moved to the end"
        );
        assert_eq!(
            chosen.index().and_then(|at| monitors.get(at)),
            Some(&venue()[0]),
            "still the operator's chosen monitor"
        );
        Ok(())
    }

    #[test]
    fn a_monitor_whose_mode_or_place_changed_is_followed_by_its_name() -> Result<()> {
        let mut monitors = venue();
        let mut chosen = Chosen::default();
        assert!(chosen.choose(1, &monitors), "the second monitor exists");
        // The projector came back at another resolution.
        if let Some(projector) = monitors.get_mut(1) {
            projector.size = [1280, 720];
        }
        assert_eq!(
            chosen.follow(&monitors),
            Followed::Same,
            "a new mode is the same monitor"
        );
        assert_eq!(chosen.index(), Some(1), "same name and position");
        // Then the desktop was rearranged: same name and size, new place.
        if let Some(projector) = monitors.get_mut(1) {
            projector.position = [-1280, 0];
        }
        assert_eq!(chosen.index(), Some(1), "unchanged before the refresh");
        assert_eq!(
            chosen.follow(&monitors),
            Followed::Same,
            "a new place is the same monitor"
        );
        assert_eq!(chosen.index(), Some(1), "same name and size");
        // Both at once is another monitor as far as anyone can tell.
        if let Some(projector) = monitors.get_mut(1) {
            projector.position = [5000, 0];
            projector.size = [3840, 2160];
        }
        assert_eq!(
            chosen.follow(&monitors),
            Followed::Lost,
            "a new mode and place is not"
        );
        Ok(())
    }

    #[test]
    fn a_partial_match_is_never_a_guess_between_two_or_without_a_name() -> Result<()> {
        // Two monitors that share a name (a system that reports a model
        // name, say) and the chosen one's size: which one it became cannot
        // be told, so neither is chosen.
        let twin = |x| monitor("Projector", x, 1920);
        let mut monitors = vec![twin(0), twin(1920), twin(3840)];
        let mut chosen = Chosen::default();
        assert!(chosen.choose(2, &monitors), "the third monitor exists");
        monitors.remove(2);
        assert_eq!(chosen.follow(&monitors), Followed::Lost, "two candidates");

        let unnamed = |x| Identity {
            name: None,
            position: [x, 0],
            size: [1920, 1080],
        };
        let mut monitors = vec![unnamed(0), unnamed(1920)];
        let mut chosen = Chosen::default();
        assert!(chosen.choose(1, &monitors), "the second monitor exists");
        monitors.remove(1);
        assert_eq!(
            chosen.follow(&monitors),
            Followed::Lost,
            "a size alone is not a monitor"
        );
        Ok(())
    }

    #[test]
    fn choosing_past_the_end_changes_nothing() -> Result<()> {
        let monitors = venue();
        let mut chosen = Chosen::default();
        assert!(!chosen.choose(3, &monitors), "there is no fourth monitor");
        assert_eq!(chosen.index(), None, "nothing chosen");
        assert!(chosen.choose(1, &monitors), "the second exists");
        assert!(!chosen.choose(9, &monitors), "nor a tenth");
        assert_eq!(chosen.index(), Some(1), "the earlier choice stands");
        assert_eq!(chosen.follow(&monitors), Followed::Same, "nothing moved");
        Ok(())
    }
}
