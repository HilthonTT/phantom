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

pub(crate) struct Pool {
    server: Arc<Server>,

    queues: Vec<Sender<Cmd>>,

    workers: Mutex<Vec<JoinHandle<()>>>,

    topology: Vec<usize>,

    busy: AtomicUsize,

    queued_max: AtomicUsize,
}

pub(crate) enum Cmd {
    Get(Get),

    Iter(Seek),
}

pub(crate) struct Get {
    pub(crate) map: Arc<Map>,
    pub(crate) key: BatchQuery,
    pub(crate) res: Option<ResultSender<BatchResult<'static>>>,
}

pub(crate) struct Seek {
    pub(crate) state: cursor::State<'static>,
    pub(crate) map: Arc<Map>,
    pub(crate) dir: Direction,
    pub(crate) key: Option<KeyBuf>,
    pub(crate) res: Option<ResultSender<cursor::State<'static>>>,
}

type ResultSender<T> = oneshot::Sender<T>;

pub(crate) type BatchQuery = SmallVec<[KeyBuf; BATCH_INLINE]>;
pub(crate) type BatchResult<'a> = SmallVec<[Result<Handle<'a>>; BATCH_INLINE]>;

const BATCH_INLINE: usize = 1;

const WORKER_LIMIT: (usize, usize) = (1, 1024);

const QUEUE_LIMIT: (usize, usize) = (1, 4096);

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

    let this_thread = thread::current().id();

    workers
        .into_iter()
        .filter(|worker| worker.thread().id() != this_thread)
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
        .filter(|_| self.queues.len() > 1)
        .filter(|_| self.server.config.database.db_pool_affinity)
        .filter_map(|(core_id, &queue_id)| (group == queue_id).then_some(core_id))
        .filter_map(nth_core_available);

    set_affinity(affinity.clone());

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
        return;
    };

    let result = cmd.map.get_blocking(&cmd.key[0]);

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

#[inline]
#[allow(unsafe_code)]
fn send_get(result: BatchResult<'_>) -> BatchResult<'static> {
    unsafe { std::mem::transmute(result) }
}

#[inline]
#[allow(unsafe_code)]
fn recv_get<'a>(result: BatchResult<'static>) -> BatchResult<'a> {
    unsafe { std::mem::transmute(result) }
}

#[inline]
#[allow(unsafe_code)]
pub(crate) fn send_seek(state: cursor::State<'_>) -> cursor::State<'static> {
    unsafe { std::mem::transmute(state) }
}

#[inline]
#[allow(unsafe_code)]
fn recv_seek<'a>(state: cursor::State<'static>) -> cursor::State<'a> {
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
