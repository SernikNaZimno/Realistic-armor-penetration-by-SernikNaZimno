//! The states of a run that are kept, so that the user can go back and
//! forth through it.
//!
//! A state of a large model takes hundreds of megabytes, so the states kept
//! are limited by the memory they take: when a new one would pass the
//! limit, every second one kept so far is dropped. The run is then still
//! covered from start to end, more coarsely.

use std::mem::size_of;

use rap_solver::Particle;

use crate::runner::Snapshot;
use crate::view3d::Cloud;

/// States of a run in the order of their times, and which one is shown.
#[derive(Debug)]
pub(crate) struct Frames {
    items: Vec<Snapshot>,
    /// Most memory the states may take, in bytes.
    budget: usize,
    /// Index of the state shown; `None` to show the latest as it arrives.
    shown: Option<usize>,
}

/// Memory `snapshot` takes, in bytes.
fn size(snapshot: &Snapshot) -> usize {
    let cloud = snapshot
        .cloud
        .as_ref()
        .map_or(0, |cloud: &Cloud| size_of_val(cloud.points.as_slice()));
    snapshot.particles.len() * size_of::<Particle>() + cloud
}

impl Frames {
    /// The states of a run that starts at `first`, limited to `budget`
    /// bytes.
    pub(crate) fn new(first: Snapshot, budget: usize) -> Self {
        Self {
            items: vec![first],
            budget,
            shown: None,
        }
    }

    /// Adds the state the run has reached.
    pub(crate) fn push(&mut self, snapshot: Snapshot) {
        self.items.push(snapshot);
        while self.items.len() > 2 && self.items.iter().map(size).sum::<usize>() > self.budget {
            let last = self.items.len() - 1;
            let mut index = 0;
            self.items.retain(|_| {
                let keep = index % 2 == 0 || index == last;
                index += 1;
                keep
            });
            // An odd state that was shown gives way to the one before it.
            self.shown = self.shown.map(|shown| shown / 2);
        }
    }

    /// Number of states kept; at least one.
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    /// Index of the state shown.
    pub(crate) fn shown(&self) -> usize {
        self.shown
            .unwrap_or(self.items.len() - 1)
            .min(self.items.len() - 1)
    }

    /// Shows the state `index`; the last one means following the run.
    pub(crate) fn show(&mut self, index: usize) {
        self.shown = (index + 1 < self.items.len()).then_some(index);
    }

    /// Whether the latest state is shown as it arrives.
    pub(crate) fn follows(&self) -> bool {
        self.shown.is_none()
    }

    /// The state shown.
    pub(crate) fn current(&self) -> &Snapshot {
        &self.items[self.shown()]
    }

    /// The latest state.
    pub(crate) fn latest(&self) -> &Snapshot {
        &self.items[self.items.len() - 1]
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use glam::{DMat2, DVec2};
    use rap_analysis::ImpactSummary;
    use rap_solver::{Deviator, Particle};

    use super::Frames;
    use crate::runner::Snapshot;

    fn particle() -> Particle {
        Particle {
            position: DVec2::ZERO,
            velocity: DVec2::ZERO,
            velocity_gradient: DMat2::ZERO,
            mass: 1.0,
            volume: 1.0,
            pressure: 0.0,
            viscous_pressure: 0.0,
            deviator: Deviator::default(),
            specific_energy: 0.0,
            plastic_strain: 0.0,
            plastic_strain_rate: 0.0,
            temperature: 294.0,
            damage: 0.0,
            sound_speed: 0.0,
            body: 0,
            material: 0,
        }
    }

    /// A state of two particles at `time`.
    fn state(time: f64) -> Snapshot {
        Snapshot {
            particles: vec![particle(); 2],
            cloud: None,
            summary: ImpactSummary {
                time,
                penetration_depth: 0.0,
                armor_path_length: 0.1,
                perforated: false,
                residual_speed: 0.0,
                residual_mass_fraction: 0.0,
                projectile_failed_fraction: 0.0,
                armor_failed_fraction: 0.0,
            },
        }
    }

    fn times(frames: &Frames) -> Vec<f64> {
        frames
            .items
            .iter()
            .map(|snapshot| snapshot.summary.time)
            .collect()
    }

    #[test]
    fn the_latest_state_is_shown_until_another_is_picked() {
        let mut frames = Frames::new(state(0.0), usize::MAX);
        frames.push(state(1.0));
        frames.push(state(2.0));
        assert!(frames.follows());
        assert!((frames.current().summary.time - 2.0).abs() < f64::EPSILON);

        frames.show(1);
        frames.push(state(3.0));
        assert!(!frames.follows());
        assert_eq!(frames.shown(), 1);
        assert!((frames.current().summary.time - 1.0).abs() < f64::EPSILON);
        assert!((frames.latest().summary.time - 3.0).abs() < f64::EPSILON);

        // Picking the last one follows the run again.
        frames.show(3);
        frames.push(state(4.0));
        assert!(frames.follows());
        assert_eq!(frames.shown(), 4);
        assert_eq!(frames.len(), 5);
    }

    #[test]
    fn every_second_state_is_dropped_when_memory_runs_out() {
        // Room for five states of two particles.
        let budget = 5 * 2 * size_of::<Particle>();
        let mut frames = Frames::new(state(0.0), budget);
        for time in 1..=4 {
            frames.push(state(f64::from(time)));
        }
        assert_eq!(times(&frames), [0.0, 1.0, 2.0, 3.0, 4.0]);
        frames.show(3);

        // The sixth does not fit: the odd ones go, the first and the
        // newest stay, and the view moves to the state before the one it
        // showed.
        frames.push(state(5.0));
        assert_eq!(times(&frames), [0.0, 2.0, 4.0, 5.0]);
        assert_eq!(frames.shown(), 1);
        frames.push(state(6.0));
        assert_eq!(times(&frames), [0.0, 2.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn two_states_are_kept_whatever_they_take() {
        let mut frames = Frames::new(state(0.0), 1);
        for time in 1..=5 {
            frames.push(state(f64::from(time)));
        }
        assert_eq!(times(&frames), [0.0, 5.0]);
        assert!(frames.follows());
    }
}
