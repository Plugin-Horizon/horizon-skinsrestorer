//! Database work off the tick: lookups go to a worker thread and come back on the next tick.
use crate::property::Property;
use crate::storage::{Lookup, Storage};
use std::sync::mpsc;
use std::sync::Arc;

/// Why a player's skin is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The join's lookup did not finish on the connection's thread.
    Join,
    /// The periodic check.
    Poll,
}

#[derive(Clone, Debug)]
pub struct Job {
    pub player: u128,
    pub reason: Reason,
}

#[derive(Clone, Debug)]
pub struct Done {
    pub player: u128,
    pub reason: Reason,
    pub result: Result<Answer, String>,
}

/// A lookup's answer, with the account skin when there is no selection (what a cleared
/// selection goes back to).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub lookup: Lookup,
    pub own: Option<Property>,
}

pub fn answer(storage: &Storage, player: u128) -> Result<Answer, String> {
    let lookup = storage.skin_of(player)?;
    let own = match lookup {
        Lookup::NoSelection => storage.own_skin(player)?,
        _ => None,
    };
    Ok(Answer { lookup, own })
}

pub trait Jobs: Send {
    fn submit(&mut self, job: Job);
    fn done(&mut self) -> Vec<Done>;
}

/// The server's: one worker thread, in order.
pub struct ThreadJobs {
    tx: mpsc::Sender<Job>,
    rx: mpsc::Receiver<Done>,
}

impl ThreadJobs {
    pub fn start(storage: Arc<Storage>) -> Self {
        let (tx, jobs) = mpsc::channel::<Job>();
        let (results, rx) = mpsc::channel::<Done>();
        let spawned = std::thread::Builder::new()
            .name("skinsrestorer-db".into())
            .spawn(move || {
                for job in jobs {
                    let result = answer(&storage, job.player);
                    if results
                        .send(Done {
                            player: job.player,
                            reason: job.reason,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        if let Err(e) = spawned {
            log::error!("[SkinsRestorer] could not start the database thread: {e}");
        }
        Self { tx, rx }
    }
}

impl Jobs for ThreadJobs {
    fn submit(&mut self, job: Job) {
        let _ = self.tx.send(job);
    }
    fn done(&mut self) -> Vec<Done> {
        self.rx.try_iter().collect()
    }
}

/// The tests': runs at once.
pub struct InlineJobs {
    pub storage: Arc<Storage>,
    pub out: Vec<Done>,
    pub submitted: Vec<Job>,
}

impl Jobs for InlineJobs {
    fn submit(&mut self, job: Job) {
        let result = answer(&self.storage, job.player);
        self.out.push(Done {
            player: job.player,
            reason: job.reason,
            result,
        });
        self.submitted.push(job);
    }
    fn done(&mut self) -> Vec<Done> {
        std::mem::take(&mut self.out)
    }
}
