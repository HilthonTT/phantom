//! The thread pool that blocking database work is offloaded to.
//!
//! A read which misses the block cache blocks the calling thread until the
//! storage answers. Doing that on a tokio worker would stall every other task
//! sharing it, so the map layer tries the cache first and, on a miss, submits
//! the work here instead: operating-system threads whose whole job is to sit
//! in that wait.
//!
//! The pool is sized and laid out after the storage device rather than after
//! the CPU — see [`configure`] — because the thing being waited on is the
//! device's own queue depth.

mod configure;

use std::{
    mem::take,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
};

use async_channel::{Receiver, RecvError, Sender};
use futures::{TryFutureExt, channel::oneshot};
use phantom_core::{
    Error, Result, debug, err, error, implement,
    result::DebugInspect,
    server::Server,
    sys::compute::{get_affinity, nth_core_available, set_affinity},
    trace,
};
use rocksdb::Direction;
use smallvec::SmallVec;

use self::configure::configure;
use crate::{Handle, cursor, keyval::KeyBuf, map::Map};

/// Frontend to the worker threads.
pub(crate) struct Pool {
    server: Arc<Server>,

    /// One queue per hardware queue on the storage device. Which one a request
    /// goes to is decided by the core it was submitted from, so that a request
    /// stays on the node whose device queue will carry it.
    queues: Vec<Sender<Cmd>>,

    workers: Mutex<Vec<JoinHandle<()>>>,

    /// Maps a core to the queue serving it; the inverse of the affinity each
    /// worker sets for itself.
    topology: Vec<usize>,

    /// Workers not currently parked waiting for a request. Observed by the
    /// tracing spans; not load-bearing.
    busy: AtomicUsize,

    /// High-water mark of any queue's depth, kept under debug only, to show
    /// whether the queue sizing is anywhere near being hit.
    queued_max: AtomicUsize,
}

/// What a worker can be asked to do.
pub(crate) enum Cmd {
    /// Read one or more keys from a column.
    Get(Get),

    /// Position a cursor, which is the step of an iteration that may block.
    Iter(Seek),
}

/// A read of one or more keys, all from the same column.
pub(crate) struct Get {
    pub(crate) map: Arc<Map>,
    pub(crate) key: BatchQuery,
    pub(crate) res: Option<ResultSender<BatchResult<'static>>>,
}

/// Positioning a cursor.
///
/// Only the initial seek is submitted. The steps after it are taken on the
/// caller's thread, on the assumption that the engine's readahead has the next
/// block in hand by then; a step that does block is the price of not paying a
/// queue round trip per entry.
pub(crate) struct Seek {
    pub(crate) map: Arc<Map>,
    pub(crate) state: cursor::State<'static>,
    pub(crate) dir: Direction,
    pub(crate) key: Option<KeyBuf>,
    pub(crate) res: Option<ResultSender<cursor::State<'static>>>,
}

type ResultSender<T> = oneshot::Sender<T>;

pub(crate) type BatchQuery = SmallVec<[KeyBuf; BATCH_INLINE]>;
pub(crate) type BatchResult<'a> = SmallVec<[Result<Handle<'a>>; BATCH_INLINE]>;

/// Batches of one are the common case — a single `get` — so that is what the
/// batch buffers hold before spilling.
const BATCH_INLINE: usize = 1;

/// Bounds on the derived worker count, whatever the device claims.
const WORKER_LIMIT: (usize, usize) = (1, 1024);

/// Bounds on the derived queue depth. The lower bound matters: a zero-capacity
/// queue would rendezvous, serializing every submission against a worker.
const QUEUE_LIMIT: (usize, usize) = (1, 4096);

/// Workers only ever call into the engine, which keeps its own stacks.
const WORKER_STACK_SIZE: usize = 1_048_576;

const WORKER_NAME: &str = "phantom:db";

#[implement(Pool)]
pub(crate) fn new(server: &Arc<Server>) -> Result<Arc<Self>> {
    let (total_workers, queue_sizes, topology) = configure(server);

    let (senders, receivers): (Vec<_>, Vec<_>) =
        queue_sizes.into_iter().map(async_channel::bounded).unzip();

    let pool = Arc::new(Self {
        server: server.clone(),
        queues: senders,
        workers: Vec::new().into(),
        topology,
        busy: AtomicUsize::default(),
        queued_max: AtomicUsize::default(),
    });

    pool.spawn_until(&receivers, total_workers)?;

    Ok(pool)
}

/// Closes the queues and joins every worker.
///
/// Separate from `Drop` because the engine has to outlive the workers — they
/// hold column handles into it — so this is driven before the database is
/// dropped rather than as a consequence of it.
#[implement(Pool)]
#[tracing::instrument(skip_all)]
pub(crate) fn close(&self) {
    let workers = take(&mut *self.workers.lock().expect("workers lock is not poisoned"));

    for queue in &self.queues {
        queue.close();
    }

    if workers.is_empty() {
        return;
    }

    debug!(
        queues = self.queues.len(),
        workers = workers.len(),
        "Closing pool. Waiting for workers to join..."
    );

    workers
        .into_iter()
        .map(JoinHandle::join)
        .map(|result| result.map_err(Error::from_panic))
        .enumerate()
        .for_each(|(id, result)| match result {
            Ok(()) => trace!(?id, "worker joined"),
            Err(error) => error!(?id, "worker joined with error: {error}"),
        });
}

#[implement(Pool)]
fn spawn_until(self: &Arc<Self>, recv: &[Receiver<Cmd>], count: usize) -> Result {
    let mut workers = self.workers.lock().expect("workers lock is not poisoned");

    while workers.len() < count {
        self.clone().spawn_one(&mut workers, recv)?;
    }

    Ok(())
}

#[implement(Pool)]
#[tracing::instrument(
    name = "spawn",
    level = "trace",
    skip_all,
    fields(id = %workers.len()),
)]
fn spawn_one(self: Arc<Self>, workers: &mut Vec<JoinHandle<()>>, recv: &[Receiver<Cmd>]) -> Result {
    debug_assert!(!self.queues.is_empty(), "must have at least one queue");
    debug_assert!(!recv.is_empty(), "must have at least one receiver");

    let id = workers.len();
    let group = id.wrapping_rem(self.queues.len());
    let recv = recv[group].clone();

    let handle = thread::Builder::new()
        .name(WORKER_NAME.into())
        .stack_size(WORKER_STACK_SIZE)
        .spawn(move || self.worker(id, recv))?;

    workers.push(handle);

    Ok(())
}

/// Runs a read on a worker and awaits its result.
#[implement(Pool)]
#[tracing::instrument(level = "trace", name = "get", skip(self, cmd))]
pub(crate) async fn execute_get(self: &Arc<Self>, mut cmd: Get) -> Result<BatchResult<'_>> {
    let (send, recv) = oneshot::channel();
    _ = cmd.res.insert(send);

    let queue = self.select_queue();
    self.execute(queue, Cmd::Get(cmd))
        .and_then(move |()| {
            recv.map_ok(recv_get)
                .map_err(|e| err!(error!("database worker dropped the request: {e:?}")))
        })
        .await
}

/// Positions a cursor on a worker and awaits it.
#[implement(Pool)]
#[tracing::instrument(level = "trace", name = "iter", skip(self, cmd))]
pub(crate) async fn execute_iter(self: &Arc<Self>, mut cmd: Seek) -> Result<cursor::State<'_>> {
    let (send, recv) = oneshot::channel();
    _ = cmd.res.insert(send);

    let queue = self.select_queue();
    self.execute(queue, Cmd::Iter(cmd))
        .and_then(|()| {
            recv.map_ok(recv_seek)
                .map_err(|e| err!(error!("database worker dropped the request: {e:?}")))
        })
        .await
}

/// The queue serving the core this task is running on.
#[implement(Pool)]
fn select_queue(&self) -> &Sender<Cmd> {
    let core_id = get_affinity().next().unwrap_or(0);
    let chan_id = self.topology.get(core_id).copied().unwrap_or(0);

    self.queues.get(chan_id).unwrap_or(&self.queues[0])
}

#[implement(Pool)]
#[tracing::instrument(
    level = "trace",
    name = "execute",
    skip(self, cmd),
    fields(
        task = ?tokio::task::try_id(),
        receivers = queue.receiver_count(),
        queued = queue.len(),
        queued_max = self.queued_max.load(Ordering::Relaxed),
    ),
)]
async fn execute(&self, queue: &Sender<Cmd>, cmd: Cmd) -> Result {
    if cfg!(debug_assertions) {
        self.queued_max.fetch_max(queue.len(), Ordering::Relaxed);
    }

    // Awaits when the queue is full, which is the backpressure that stops
    // requests being accepted faster than the storage can answer them.
    queue
        .send(cmd)
        .await
        .map_err(|e| err!(error!("database queue closed: {e:?}")))
}

#[implement(Pool)]
#[tracing::instrument(
    parent = None,
    level = "debug",
    skip(self, recv),
    fields(tid = ?thread::current().id()),
)]
fn worker(self: Arc<Self>, id: usize, recv: Receiver<Cmd>) {
    self.worker_init(id);
    self.worker_loop(&recv);
}

#[implement(Pool)]
fn worker_init(&self, id: usize) {
    let group = id.wrapping_rem(self.queues.len());
    let affinity = self
        .topology
        .iter()
        .enumerate()
        // A single queue means every core serves it, so pinning would only
        // take cores away from the scheduler for nothing.
        .filter(|_| self.queues.len() > 1)
        .filter(|_| self.server.config.db_pool_affinity)
        .filter_map(|(core_id, &queue_id)| (group == queue_id).then_some(core_id))
        .filter_map(nth_core_available);

    set_affinity(affinity.clone());

    // Where jemalloc is partitioning arenas by core and this worker is pinned
    // to exactly one, put it on that core's arena: the values it allocates are
    // freed by whichever tokio worker consumes them, and crossing arenas on
    // every read is what that partitioning exists to avoid.
    #[cfg(all(not(target_env = "msvc"), feature = "jemalloc"))]
    if affinity.clone().count() == 1 && phantom_core::alloc::je::is_affine_arena() {
        use phantom_core::{
            alloc::je::this_thread::{arena_id, set_arena},
            result::LogDebugErr,
        };

        let id = affinity.clone().next().expect("exactly one core");

        if arena_id().is_ok_and(|arena| arena != id) {
            set_arena(id).log_debug_err().ok();
        }
    }

    debug!(
        ?group,
        affinity = ?affinity.collect::<Vec<_>>(),
        "worker ready"
    );
}

#[implement(Pool)]
fn worker_loop(self: &Arc<Self>, recv: &Receiver<Cmd>) {
    // The wait span reports `busy` as it decrements on entry, so the count has
    // to start out including this worker.
    self.busy.fetch_add(1, Ordering::Relaxed);

    while let Ok(cmd) = self.worker_wait(recv) {
        match cmd {
            Cmd::Get(cmd) if cmd.key.len() == 1 => self.handle_get(cmd),
            Cmd::Get(cmd) => self.handle_batch(cmd),
            Cmd::Iter(cmd) => self.handle_iter(cmd),
        }
    }
}

#[implement(Pool)]
#[tracing::instrument(
    name = "wait",
    level = "trace",
    skip_all,
    fields(
        queued = recv.len(),
        busy = self.busy.fetch_sub(1, Ordering::Relaxed) - 1,
    ),
)]
fn worker_wait(self: &Arc<Self>, recv: &Receiver<Cmd>) -> Result<Cmd, RecvError> {
    recv.recv_blocking().debug_inspect(|_| {
        self.busy.fetch_add(1, Ordering::Relaxed);
    })
}

#[implement(Pool)]
#[tracing::instrument(name = "get", level = "trace", skip_all, fields(%cmd.map))]
fn handle_get(&self, mut cmd: Get) {
    debug_assert_eq!(cmd.key.len(), 1, "should have exactly one key");
    debug_assert!(!cmd.key[0].is_empty(), "querying for an empty key");

    let Some(chan) = cmd.res.take().filter(|chan| !chan.is_canceled()) else {
        // The caller's future was dropped while this sat in the queue, so the
        // query can be skipped outright.
        return;
    };

    // Goes back through the map layer rather than to the engine directly, so
    // that a read costs the same wherever it was issued from.
    let result = cmd.map.get_blocking(&cmd.key[0]);

    // Fails if the caller gave up between the check above and now, which is
    // as acceptable here as it was there.
    chan.send(send_get([result].into())).ok();
}

#[implement(Pool)]
#[tracing::instrument(
    name = "batch",
    level = "trace",
    skip_all,
    fields(%cmd.map, keys = %cmd.key.len()),
)]
fn handle_batch(self: &Arc<Self>, mut cmd: Get) {
    debug_assert!(cmd.key.len() > 1, "should have more than one key");
    debug_assert!(
        !cmd.key.iter().any(SmallVec::is_empty),
        "querying for an empty key"
    );

    let Some(chan) = cmd.res.take().filter(|chan| !chan.is_canceled()) else {
        return;
    };

    let result = cmd.map.get_batch_blocking(cmd.key.iter()).collect();

    chan.send(send_get(result)).ok();
}

#[implement(Pool)]
#[tracing::instrument(name = "iter", level = "trace", skip_all, fields(%cmd.map))]
fn handle_iter(&self, mut cmd: Seek) {
    let Some(chan) = cmd.res.take().filter(|chan| !chan.is_canceled()) else {
        return;
    };

    let from = cmd.key.as_deref();
    let state = match cmd.dir {
        Direction::Forward => cmd.state.init::<{ cursor::FORWARD }>(from),
        Direction::Reverse => cmd.state.init::<{ cursor::REVERSE }>(from),
    };

    chan.send(send_seek(state)).ok();
}

// The four functions below launder the lifetime on a handle or a cursor across
// the channel between a worker and its caller. `send_` erases it on the way
// out of a thread, `recv_` restores it on the way in.
//
// # Safety
//
// Both borrow into the open database: a handle pins a cache block, a cursor
// holds a column handle. The lifetime is how `rocksdb` states that, and a
// channel cannot carry it, so it is erased on the way out and restored on the
// way in.
//
// What makes the round trip sound is that the caller of `execute_get` or
// `execute_iter` is awaiting a borrow of the pool, which is owned by the
// engine: the database cannot be dropped while a result is in flight, and the
// lifetime the caller ends up with is bounded by that borrow rather than by
// the `'static` the channel saw.

#[inline]
#[allow(unsafe_code)]
fn send_get(result: BatchResult<'_>) -> BatchResult<'static> {
    // SAFETY: see above.
    unsafe { std::mem::transmute(result) }
}

#[inline]
#[allow(unsafe_code)]
fn recv_get<'a>(result: BatchResult<'static>) -> BatchResult<'a> {
    // SAFETY: see above.
    unsafe { std::mem::transmute(result) }
}

#[inline]
#[allow(unsafe_code)]
pub(crate) fn send_seek(state: cursor::State<'_>) -> cursor::State<'static> {
    // SAFETY: see above.
    unsafe { std::mem::transmute(state) }
}

#[inline]
#[allow(unsafe_code)]
fn recv_seek<'a>(state: cursor::State<'static>) -> cursor::State<'a> {
    // SAFETY: see above.
    unsafe { std::mem::transmute(state) }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.close();

        debug_assert!(
            self.queues.iter().all(Sender::is_empty),
            "no requests should be queued once the pool is dropped"
        );
        debug_assert!(
            self.queues.iter().all(Sender::is_closed),
            "queues should be closed once the pool is dropped"
        );
    }
}
