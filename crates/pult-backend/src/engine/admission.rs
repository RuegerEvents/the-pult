//! Who gets to talk to the engine, and how much.
//!
//! Everything reached the engine through one 256-deep channel with no priority. A
//! plugin in a write loop, a browser fetching the whole show and a peer catching up
//! after an absence all queued together, first come first served — so the way to make
//! an operator's fader stop responding was for somebody else's plugin to be busy.
//!
//! This is a bounded queue per *source class*, serviced in weighted turns. A plugin
//! that floods fills the plugin queue and nothing else, and the operator's next edit
//! is behind at most a few messages rather than behind all of them.
//!
//! ## In front of the engine rather than inside it
//!
//! The engine still reads one channel and still knows nothing about where a command
//! came from. A router task sits in front, holding one queue per class and forwarding
//! into that channel in turn. So admission is a policy in one file, `EngineCommand`
//! is unchanged, and no call site had to learn a new shape — `EngineHandle` is still
//! a sender, it is just a sender into a particular class's queue.
//!
//! ## Queues bound, they do not drop
//!
//! `OutputHandle::push` is the model for *never blocking the engine*, and it drops
//! when the consumer is behind because a skipped frame is redrawn a fortieth of a
//! second later. A skipped write is gone. So a full class queue makes its own
//! senders wait, which is backpressure landing on the source that caused it — which
//! is the whole point, since the source that caused it is the one that should feel
//! it.
//!
//! ## The weights are turns, not priorities
//!
//! Strict priority starves. A peer that is catching up after twenty minutes away has
//! thousands of operations to replay and would never finish if an operator's queue
//! always won. So each class gets a number of turns per cycle: the operator gets the
//! most, and everybody still moves.

use tokio::sync::mpsc;

use super::EngineCommand;

/// Where a command came from, which is all the router needs to know about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// A person at a desk or a tablet, through the WebSocket. The one class whose
    /// latency somebody is watching.
    Operator,
    /// A WASM plugin through `host_impls`.
    Plugin,
    /// Another station, replaying what this one missed.
    Peer,
    /// This station's own machinery: playback, flows, the station reporter.
    Station,
}

impl Source {
    /// How many messages this class may forward before the router moves on.
    ///
    /// Not a priority. Every class gets turns every cycle, so a peer's catch-up
    /// finishes even while somebody is programming; it simply finishes slower than
    /// the programming happens.
    const fn turns(self) -> usize {
        match self {
            // The one a person is waiting on.
            Source::Operator => 8,
            // The console's own work. Cheap, frequent, and nothing is blocked on it.
            Source::Station => 4,
            // Bulk, and latency-insensitive by nature: it is replaying the past.
            Source::Peer => 3,
            // Bulk, and the class most likely to be a runaway loop.
            Source::Plugin => 2,
        }
    }

    const ALL: [Source; 4] = [Source::Operator, Source::Station, Source::Peer, Source::Plugin];
}

/// How deep one class's queue is.
///
/// Per class rather than shared, which is the entire mechanism: 256 was the old
/// shared depth, and the failure it allowed was one source filling all of it.
const CLASS_DEPTH: usize = 256;

/// The senders, one per class. Handed out by `EngineHandle::for_source`.
pub struct Admission {
    senders: Vec<(Source, mpsc::Sender<EngineCommand>)>,
}

impl Admission {
    /// This class's queue.
    pub fn sender(&self, source: Source) -> mpsc::Sender<EngineCommand> {
        self.senders
            .iter()
            .find(|(each, _)| *each == source)
            .map(|(_, tx)| tx.clone())
            .expect("every Source has a queue")
    }
}

/// Start the router in front of `into`, and hand back the per-class senders.
pub fn start(into: mpsc::Sender<EngineCommand>) -> Admission {
    let mut senders = Vec::new();
    let mut receivers = Vec::new();
    for source in Source::ALL {
        let (tx, rx) = mpsc::channel::<EngineCommand>(CLASS_DEPTH);
        senders.push((source, tx));
        receivers.push((source, rx));
    }

    tokio::spawn(async move {
        loop {
            let mut moved = 0usize;

            // One cycle: each class in turn, up to its share, taking only what is
            // already queued. `try_recv` rather than `recv` so an empty class costs
            // nothing and does not hold up the ones behind it.
            for (source, rx) in receivers.iter_mut() {
                for _ in 0..source.turns() {
                    match rx.try_recv() {
                        Ok(command) => {
                            if into.send(command).await.is_err() {
                                return; // the engine has stopped
                            }
                            moved += 1;
                        }
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        // A class whose senders have all gone is not a reason to stop
                        // routing: a station outlives any one plugin or peer.
                        Err(mpsc::error::TryRecvError::Disconnected) => break,
                    }
                }
            }

            if moved > 0 {
                continue;
            }

            // Everything is empty. Wait for whichever class speaks next rather than
            // spinning — a console with nobody touching it must cost nothing.
            let waiting = futures::future::select_all(
                receivers.iter_mut().map(|(_, rx)| Box::pin(rx.recv())),
            );
            let (first, index, _) = waiting.await;
            match first {
                Some(command) => {
                    if into.send(command).await.is_err() {
                        return;
                    }
                }
                None => {
                    // That class is closed for good. If every one is, there is nobody
                    // left to admit and the router is done.
                    let _ = index;
                    if receivers.iter_mut().all(|(_, rx)| rx.is_closed()) {
                        return;
                    }
                }
            }
        }
    });

    Admission { senders }
}
