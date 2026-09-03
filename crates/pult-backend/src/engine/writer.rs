//! The disk, off the actor.
//!
//! `persist`, `oplog::append` and `order::save` used to be awaited inside the engine
//! actor's command arm, against a pool capped at one connection. So one operator's
//! edit waited behind another operator's fsync — not behind the *work* of it, behind
//! the disk itself. On a busy tech that is an operator watching a fader not move
//! because somebody else patched a fixture.
//!
//! This is a single writer task with an ordered queue. Still ordered, still durable,
//! no longer between a command and its reply.
//!
//! ## What a command still promises
//!
//! Exactly what it promised before: **a write that was acknowledged is on the disk.**
//! The reply waits for the commit, and the non-goal from the original plan holds —
//! no new durability guarantee, and none taken away either.
//!
//! Which is why this batches rather than merely moving the await. A queue where each
//! reply waits for its own fsync serialises operators just as thoroughly as the actor
//! did; the queue would be shorter and the wait would be the same. So the writer
//! commits a *group*.
//!
//! ## The batch has no constant in it
//!
//! While a commit is in flight everything that arrives queues up, and when it lands
//! they all go into the next one. That is the whole rule. There is no window in
//! milliseconds and no batch size, deliberately: on a fast disk with one operator the
//! batch degenerates to a single write per commit and adds no latency at all, and on
//! a slow one under load it grows exactly as much as the disk is making it grow. A
//! constant would have to be right for somebody else's hardware, and would be read
//! off this machine.
//!
//! ## Two pools, because the showfile is WAL
//!
//! The writer holds its own pool to the same file. WAL allows one writer alongside
//! readers, so the oplog being read for a peer's catch-up does not queue behind a
//! commit and never sees an uncommitted row. A station whose showfile is in memory
//! shares the one pool instead — every `sqlite::memory:` connection is a *different*
//! database, so a second pool there would be a second, empty show.
//!
//! ## What is not in the batch
//!
//! `order::save` opens its own transaction, and SQLite has no nested `BEGIN`. It is
//! also not the hot path: an order changes when something is created, deleted or
//! moved, never when a value does. So order writes are applied after the batch
//! commits, each as itself, which also keeps them ordered behind the entity write
//! they belong to.

use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use pult_schema::{events::operation::Operation, registry::EntityMeta};
use uuid::Uuid;

use crate::infra::showfile::{oplog, order};

/// One thing to put on the disk.
pub enum WriteJob {
    /// One entity, as JSON, written through its own registered upsert.
    Upsert { meta: &'static EntityMeta, entity: serde_json::Value },
    Delete { meta: &'static EntityMeta, id: Uuid },
    /// A collection's order. Applied outside the batch; see the module note.
    Order { table: String, ids: Vec<Uuid> },
    /// One entity appended to the end of a collection's order.
    ///
    /// Separate from `Order` because it is the case that happens per created row, and
    /// rewriting the whole collection for it is what made patching a rig quadratic.
    OrderAppend { table: String, id: Uuid },
    /// A replicated operation, for a peer that reconnects.
    Oplog { op: Box<Operation> },
    /// Nothing at all, for a caller that only wants to know when the queue in front
    /// of it has landed.
    ///
    /// The queue is ordered, so a receipt for a batch containing one of these is a
    /// receipt for everything submitted before it. What wants that is a version:
    /// copying the show before its own row is on the disk would give a snapshot that
    /// does not contain the version it is a snapshot of, and every restore would
    /// quietly forget the point it restored to.
    Barrier,
}

/// A batch as it is handed over: the jobs, and who to tell when they are durable.
struct Submission {
    jobs: Vec<WriteJob>,
    done: oneshot::Sender<Result<(), String>>,
}

/// What the engine holds. Cloneable, and dropping the last one stops the task.
#[derive(Clone)]
pub struct WriteHandle {
    tx: mpsc::Sender<Submission>,
}

/// How many submissions may be waiting before a caller has to wait to enqueue.
///
/// Deep enough that an import's thousands of writes do not take turns at the door,
/// shallow enough that a runaway producer is felt rather than buffered for ever. A
/// full queue applies backpressure to the *caller* here rather than dropping, unlike
/// `OutputHandle::push`: a frame that is skipped is redrawn a fortieth of a second
/// later, and a showfile write that is skipped is gone.
const QUEUE_DEPTH: usize = 1024;

impl WriteHandle {
    /// Put a batch on the queue and wait for it to be durable.
    ///
    /// The await is what keeps the acknowledgement honest. What has moved is *whose*
    /// fsync a caller waits behind: its own group's, rather than every write that
    /// happened to be ahead of it in an actor.
    pub async fn write(&self, jobs: Vec<WriteJob>) -> Result<(), String> {
        match self.submit(jobs).await {
            Some(wait) => {
                wait.await.map_err(|_| "the showfile writer dropped a write".to_string())?
            }
            None => Ok(()),
        }
    }

    /// Hand a batch over and get back the receipt, **without waiting for it**.
    ///
    /// This is what takes the disk out of the engine actor's critical path, and it is
    /// the half that was missing. Moving the write to another task was never enough on
    /// its own: while the actor still `await`ed the result before reading its next
    /// command, the writer only ever held one submission and had nothing to group. So
    /// a five-thousand-row import was five thousand commits, and every one of them
    /// stood between two operators.
    ///
    /// The caller still waits for durability — the engine hands this receipt to a task
    /// that replies when it lands — so **an acknowledged write is still on the disk**.
    /// What no longer waits is the actor.
    ///
    /// `None` when there was nothing to write, which is not a failure.
    pub async fn submit(
        &self,
        jobs: Vec<WriteJob>,
    ) -> Option<oneshot::Receiver<Result<(), String>>> {
        if jobs.is_empty() {
            return None;
        }
        let (done, wait) = oneshot::channel();
        // Awaited only for room in the queue, which is a thousand deep and normally
        // free. That await is the backpressure, and it is meant to be felt by whoever
        // is writing faster than the disk can take it.
        if self.tx.send(Submission { jobs, done }).await.is_err() {
            return None;
        }
        Some(wait)
    }
}

/// Start the writer, and hand back what the engine talks to it with.
///
/// `write_pool` is the writer's own; see the module note about WAL and about a show
/// that lives in memory.
pub fn start(write_pool: Arc<SqlitePool>) -> WriteHandle {
    let (tx, mut rx) = mpsc::channel::<Submission>(QUEUE_DEPTH);

    tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            // Everything already waiting joins this commit. `try_recv` rather than a
            // timer: what is taken is what arrived while the last commit was in
            // flight, which is the whole of the batching rule.
            let mut batch = vec![first];
            while let Ok(next) = rx.try_recv() {
                batch.push(next);
                // A bound so one enormous import cannot hold a transaction open long
                // enough to look like a stall. Not a tuning knob for latency — the
                // batch is normally far under this — but a ceiling on how long any
                // one commit may take.
                if batch.len() >= QUEUE_DEPTH {
                    break;
                }
            }

            let result = commit(&write_pool, &batch).await;

            // Every submission in the group hears the same answer, because they all
            // rode the same commit: if it rolled back, none of them landed.
            for submission in batch {
                let _ = submission.done.send(result.clone());
            }
        }
    });

    WriteHandle { tx }
}

/// Apply one group inside one transaction, then the order writes behind it.
async fn commit(pool: &SqlitePool, batch: &[Submission]) -> Result<(), String> {
    // One order per table per commit, not one per row.
    //
    // `order::save` rewrites a whole collection — a DELETE and an INSERT per id — and
    // the engine asks for one after *every* create. So patching a rig was quadratic:
    // 5000 fixtures meant about 12.5 million inserts, and seeding one took over two
    // minutes where seeding 2000 took twenty seconds. Within a commit only the last
    // order for a table can win, since each is the whole list as it stood, so keeping
    // only that one is not an optimisation that changes anything — it is dropping
    // writes that were already dead.
    let mut orders: Vec<(&str, &[Uuid])> = Vec::new();

    if let Err(e) = sqlx::query("BEGIN IMMEDIATE").execute(pool).await {
        return Err(format!("could not begin a showfile write: {e}"));
    }

    let mut failed: Option<String> = None;
    'outer: for submission in batch {
        for job in &submission.jobs {
            let outcome = match job {
                WriteJob::Upsert { meta, entity } => match meta.upsert_one {
                    Some(upsert) => upsert(pool.clone(), entity.clone()).await.map_err(|e| e.to_string()),
                    None => Ok(()),
                },
                WriteJob::Delete { meta, id } => match meta.delete_one {
                    Some(delete) => delete(pool.clone(), *id).await.map_err(|e| e.to_string()),
                    None => Ok(()),
                },
                WriteJob::Oplog { op } => {
                    oplog::append(pool, op).await.map_err(|e| e.to_string())
                }
                WriteJob::OrderAppend { table, id } => {
                    // Inside the batch, unlike a full rewrite: it is one INSERT and
                    // opens no transaction of its own.
                    order::append(pool, table, *id).await.map_err(|e| e.to_string())
                }
                // Nothing to do is the whole of it: what a barrier is for is being
                // in this batch at all.
                WriteJob::Barrier => Ok(()),
                WriteJob::Order { table, ids } => {
                    // Held back until the batch has committed; see the module note.
                    // Replacing rather than appending is what makes a burst of creates
                    // cost one order write instead of one each.
                    if let Some(slot) = orders.iter_mut().find(|(name, _)| *name == table) {
                        slot.1 = ids.as_slice();
                    } else {
                        orders.push((table.as_str(), ids.as_slice()));
                    }
                    Ok(())
                }
            };
            if let Err(e) = outcome {
                failed = Some(e);
                break 'outer;
            }
        }
    }

    if let Some(e) = failed {
        // A write that fails takes the rest of its group back, which is the rule
        // `interop/apply.rs` already follows for an import: a half-applied batch is
        // worse than a refused one, because nothing above knows which half.
        if let Err(rollback) = sqlx::query("ROLLBACK").execute(pool).await {
            warn!("[writer] could not roll back after a failed write: {rollback}");
        }
        return Err(e);
    }

    if let Err(e) = sqlx::query("COMMIT").execute(pool).await {
        return Err(format!("could not commit a showfile write: {e}"));
    }

    // Orders, after the commit and each in its own transaction. A failure here is
    // logged rather than failing the write, which is what the engine did with them
    // before: losing the order of a list is not a reason to reject the fixture that
    // was just patched.
    for (table, ids) in orders {
        if let Err(e) = order::save(pool, table, ids).await {
            warn!("[writer] could not save {table} order: {e}");
        }
    }

    Ok(())
}
