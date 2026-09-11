use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    num::NonZeroUsize,
    sync::Arc,
};

use phantom_core::implement;
use ruma::{
    MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedRoomId, OwnedServerName, RoomVersionId,
    api::Direction,
};
use tokio::sync::watch::Receiver;

use super::{Failure, FanoutGrowth, Op, Opts, Outcome};

#[derive(Clone, Debug)]
pub(super) struct Key {
    fingerprint: u64,

    opts: Arc<Opts>,
}

#[derive(Eq, Hash, PartialEq)]
struct Identity<'a> {
    op: Op,
    room_id: &'a Option<OwnedRoomId>,
    event_id: &'a Option<OwnedEventId>,
    earliest_events: &'a [OwnedEventId],
    latest_events: &'a [OwnedEventId],
    ts: &'a Option<MilliSecondsSinceUnixEpoch>,
    dir: Option<bool>,
    hint: &'a Option<OwnedServerName>,
    candidates: &'a [OwnedServerName],
    room_version: &'a Option<RoomVersionId>,
    attempt_limit: &'a Option<NonZeroUsize>,
    backfill_limit: &'a Option<NonZeroUsize>,
    fanout_growth: &'a FanoutGrowth,
    fanout_max_width: &'a Option<NonZeroUsize>,
    fanout_rounds: &'a Option<NonZeroUsize>,
    check_event_id: bool,
    check_conforms: bool,
    check_hashes: bool,
    authoritative_redaction: bool,
    check_signature: bool,
}

pub(super) type SharedResult = Result<Arc<Outcome>, Failure>;

pub(super) type Subscription = (Receiver<Option<SharedResult>>, Arc<()>);

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
            && (Arc::ptr_eq(&self.opts, &other.opts)
                || identity(&self.opts) == identity(&other.opts))
    }
}

impl Eq for Key {}

impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.fingerprint.hash(state);
    }
}

#[implement(Key)]
pub(super) fn new(mut opts: Opts) -> Self {
    if matches!(opts.op, Op::MissingEvents) {
        opts.earliest_events.sort_unstable();
        opts.latest_events.sort_unstable();
    }

    let opts = Arc::new(opts);
    let fingerprint = fingerprint(&identity(&opts));

    Self { fingerprint, opts }
}

#[implement(Key)]
#[inline]
pub(super) fn opts(&self) -> Arc<Opts> {
    self.opts.clone()
}

fn identity(opts: &Opts) -> Identity<'_> {
    let windows = matches!(opts.op, Op::MissingEvents)
        .then_some((
            opts.earliest_events.as_slice(),
            opts.latest_events.as_slice(),
        ))
        .unwrap_or_default();

    let (earliest_events, latest_events) = windows;
    let dir = opts.dir.map(|dir| matches!(dir, Direction::Forward));

    Identity {
        op: opts.op,
        room_id: &opts.room_id,
        event_id: &opts.event_id,
        earliest_events,
        latest_events,
        ts: &opts.ts,
        dir,
        hint: &opts.hint,
        candidates: &opts.candidates,
        room_version: &opts.room_version,
        attempt_limit: &opts.attempt_limit,
        backfill_limit: &opts.backfill_limit,
        fanout_growth: &opts.fanout_growth,
        fanout_max_width: &opts.fanout_max_width,
        fanout_rounds: &opts.fanout_rounds,
        check_event_id: opts.check_event_id,
        check_conforms: opts.check_conforms,
        check_hashes: opts.check_hashes,
        authoritative_redaction: opts.authoritative_redaction,
        check_signature: opts.check_signature,
    }
}

fn fingerprint(identity: &Identity<'_>) -> u64 {
    let mut state = DefaultHasher::new();

    identity.hash(&mut state);
    state.finish()
}
