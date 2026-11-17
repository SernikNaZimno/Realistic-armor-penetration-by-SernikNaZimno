//! Runs a simulation on a background thread and reports its progress.

use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rap_analysis::ImpactSummary;
use rap_app::Simulation;
use rap_solver::Particle;

use crate::view3d::Cloud;

/// Number of progress updates that may wait for the interface. Further
/// intermediate updates are dropped rather than queued.
const PENDING_UPDATES: usize = 2;

/// Most points of the cloud made of an axisymmetric model for the
/// three-dimensional view; more would make every update slow to draw.
const CLOUD_BUDGET: usize = 600_000;

/// State of the simulation at one instant.
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub(crate) particles: Vec<Particle>,
    /// The model as a cloud for the three-dimensional view, if it has one.
    pub(crate) cloud: Option<Cloud>,
    pub(crate) summary: ImpactSummary,
}

impl Snapshot {
    /// The current state of `simulation`.
    pub(crate) fn of(simulation: &Simulation) -> Self {
        Self {
            particles: simulation.particles().into_owned(),
            cloud: rap_app::picture::cloud(simulation, CLOUD_BUDGET),
            summary: simulation.summary(),
        }
    }
}

/// Message from the worker to the interface.
#[derive(Debug, Clone)]
pub(crate) enum Update {
    /// The simulation advanced.
    Progress(Snapshot),
    /// The simulation reached its end time.
    Finished(Snapshot),
    /// The simulation failed and stopped.
    Failed(String),
}

enum Command {
    Pause,
    Resume,
    Stop,
}

/// Handle to a simulation running on its own thread. Dropping it stops the
/// simulation.
pub(crate) struct Runner {
    commands: Sender<Command>,
    updates: Receiver<Update>,
    worker: Option<JoinHandle<()>>,
}

impl Runner {
    /// Starts `simulation` and advances it to its end time in `slices`
    /// equal steps of time, reporting after each. `notify` is called from
    /// the worker thread whenever an update is ready.
    pub(crate) fn start(
        simulation: Simulation,
        slices: u32,
        notify: impl Fn() + Send + 'static,
    ) -> Self {
        let (commands, command_receiver) = mpsc::channel();
        let (update_sender, updates) = mpsc::sync_channel(PENDING_UPDATES);
        let worker = thread::spawn(move || {
            work(
                simulation,
                slices,
                &command_receiver,
                &update_sender,
                &notify,
            );
        });
        Self {
            commands,
            updates,
            worker: Some(worker),
        }
    }

    /// Suspends the simulation after its current slice.
    pub(crate) fn pause(&self) {
        // A send fails only if the worker has already finished.
        let _ = self.commands.send(Command::Pause);
    }

    /// Continues a suspended simulation.
    pub(crate) fn resume(&self) {
        let _ = self.commands.send(Command::Resume);
    }

    /// Updates that arrived since the last call, oldest first.
    pub(crate) fn poll(&self) -> Vec<Update> {
        self.updates.try_iter().collect()
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            // Keep emptying the channel so that a worker waiting to deliver
            // its last update can finish.
            while !worker.is_finished() {
                while self.updates.try_recv().is_ok() {}
                thread::sleep(Duration::from_millis(1));
            }
            // The worker only panics if the solver does; nothing to add here.
            let _ = worker.join();
        }
    }
}

/// Applies pending commands; blocks while paused. Returns `false` when the
/// worker should stop.
fn handle_commands(commands: &Receiver<Command>, paused: &mut bool) -> bool {
    loop {
        let command = if *paused {
            match commands.recv() {
                Ok(command) => command,
                Err(_) => return false,
            }
        } else {
            match commands.try_recv() {
                Ok(command) => command,
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        };
        match command {
            Command::Pause => *paused = true,
            Command::Resume => *paused = false,
            Command::Stop => return false,
        }
    }
}

fn work(
    mut simulation: Simulation,
    slices: u32,
    commands: &Receiver<Command>,
    updates: &SyncSender<Update>,
    notify: &impl Fn(),
) {
    let end_time = simulation.layout().end_time;
    let mut paused = false;
    for slice in 1..=slices {
        if !handle_commands(commands, &mut paused) {
            return;
        }
        let time = end_time * f64::from(slice) / f64::from(slices);
        if let Err(error) = simulation.advance_to(time) {
            // The receiver may be gone; then nobody needs the message.
            let _ = updates.send(Update::Failed(error.to_string()));
            notify();
            return;
        }
        let snapshot = Snapshot::of(&simulation);
        if slice == slices {
            let _ = updates.send(Update::Finished(snapshot));
        } else if let Err(TrySendError::Disconnected(_)) =
            updates.try_send(Update::Progress(snapshot))
        {
            return;
        }
        notify();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use rap_app::{Simulation, presets};

    use super::{Runner, Update};

    /// A coarse, short simulation: these tests exercise the threading.
    fn simulation() -> Simulation {
        let mut file = presets::apfsds_against_steel();
        file.settings.cells_per_radius = 2.0;
        file.settings.armor_extent_radii = 4.0;
        file.settings.end_time = Some(4.0e-5);
        Simulation::new(&file.scenario, &file.settings, &file.materials).expect("a valid preset")
    }

    fn wait_for<T>(mut poll: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(value) = poll() {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "the simulation did not respond in time"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn simulation_runs_to_the_end_and_notifies() {
        let notifications = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&notifications);
        let runner = Runner::start(simulation(), 8, move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let mut last_time = 0.0;
        let finished = wait_for(|| {
            runner.poll().into_iter().find_map(|update| match update {
                Update::Progress(snapshot) => {
                    assert!(snapshot.summary.time > last_time);
                    last_time = snapshot.summary.time;
                    None
                }
                Update::Finished(snapshot) => Some(snapshot),
                Update::Failed(message) => panic!("the simulation failed: {message}"),
            })
        });
        assert!((finished.summary.time - 4.0e-5).abs() < 1e-12);
        assert!(finished.particles.len() > 100);
        assert_eq!(notifications.load(Ordering::Relaxed), 8);
    }

    #[test]
    fn paused_simulation_waits_until_resumed() {
        let runner = Runner::start(simulation(), 400, || {});
        runner.pause();
        // Let the slice in flight finish and drain what was produced.
        thread::sleep(Duration::from_millis(300));
        runner.poll();
        thread::sleep(Duration::from_millis(200));
        assert!(
            runner.poll().is_empty(),
            "a paused simulation must not advance"
        );
        runner.resume();
        wait_for(|| (!runner.poll().is_empty()).then_some(()));
    }

    #[test]
    fn dropping_the_runner_stops_the_worker() {
        let runner = Runner::start(simulation(), 100_000, || {});
        let started = Instant::now();
        drop(runner);
        assert!(started.elapsed() < Duration::from_secs(20));
    }
}
