use std::num::NonZeroUsize;

use bytes::Bytes;
use ruma::{
    MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedRoomId, OwnedServerName, RoomVersionId,
    api::Direction,
};
use smallvec::SmallVec;

use crate::federation::Candidates;

pub type EventWindow = SmallVec<[OwnedEventId; 1]>;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Op {
    Event,

    AuthEvent,

    AuthChain,

    Backfill,

    StateIds,

    MissingEvents,

    TimestampToEvent,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FanoutGrowth {
    Fixed(NonZeroUsize),

    Linear {
        base: NonZeroUsize,

        step: NonZeroUsize,
    },

    Geometric {
        base: NonZeroUsize,

        factor: NonZeroUsize,
    },
}

impl FanoutGrowth {
    #[must_use]
    pub fn round_width(self, round: usize) -> usize {
        match self {
            Self::Fixed(width) => width.get(),
            Self::Linear { base, step } => {
                base.get().saturating_add(step.get().saturating_mul(round))
            }
            Self::Geometric { base, factor } => {
                let exp = u32::try_from(round).unwrap_or(u32::MAX);

                base.get().saturating_mul(factor.get().saturating_pow(exp))
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Opts {
    pub op: Op,

    pub room_id: Option<OwnedRoomId>,

    pub event_id: Option<OwnedEventId>,

    pub ts: Option<MilliSecondsSinceUnixEpoch>,

    pub dir: Option<Direction>,

    pub earliest_events: EventWindow,

    pub latest_events: EventWindow,

    pub hint: Option<OwnedServerName>,

    pub candidates: Candidates,

    pub room_version: Option<RoomVersionId>,

    pub attempt_limit: Option<NonZeroUsize>,

    pub backfill_limit: Option<NonZeroUsize>,

    pub fanout_growth: FanoutGrowth,

    pub fanout_max_width: Option<NonZeroUsize>,

    pub fanout_rounds: Option<NonZeroUsize>,

    pub check_event_id: bool,

    pub check_conforms: bool,

    pub check_hashes: bool,

    pub authoritative_redaction: bool,

    pub check_signature: bool,
}

impl Opts {
    #[must_use]
    pub fn new(op: Op, room_id: OwnedRoomId) -> Self {
        Self::with_room_id(op, Some(room_id))
    }

    #[must_use]
    pub fn unscoped(op: Op) -> Self {
        Self::with_room_id(op, None)
    }

    fn with_room_id(op: Op, room_id: Option<OwnedRoomId>) -> Self {
        Self {
            op,
            room_id,
            event_id: None,
            ts: None,
            dir: None,
            earliest_events: EventWindow::new(),
            latest_events: EventWindow::new(),
            hint: None,
            candidates: Candidates::new(),
            room_version: None,
            attempt_limit: None,
            backfill_limit: None,
            fanout_growth: FanoutGrowth::Fixed(NonZeroUsize::MIN),
            fanout_max_width: None,
            fanout_rounds: None,
            check_event_id: true,
            check_conforms: true,
            check_hashes: true,
            authoritative_redaction: true,
            check_signature: true,
        }
    }

    #[must_use]
    pub fn event_id(self, event_id: OwnedEventId) -> Self {
        Self {
            event_id: Some(event_id),
            ..self
        }
    }

    #[must_use]
    pub fn ts(self, ts: MilliSecondsSinceUnixEpoch) -> Self {
        Self {
            ts: Some(ts),
            ..self
        }
    }

    #[must_use]
    pub fn dir(self, dir: Direction) -> Self {
        Self {
            dir: Some(dir),
            ..self
        }
    }

    #[must_use]
    pub fn earliest_events<I>(self, earliest_events: I) -> Self
    where
        I: IntoIterator<Item = OwnedEventId>,
    {
        Self {
            earliest_events: earliest_events.into_iter().collect(),
            ..self
        }
    }

    #[must_use]
    pub fn latest_events<I>(self, latest_events: I) -> Self
    where
        I: IntoIterator<Item = OwnedEventId>,
    {
        Self {
            latest_events: latest_events.into_iter().collect(),
            ..self
        }
    }

    #[must_use]
    pub fn hint(self, hint: OwnedServerName) -> Self {
        Self {
            hint: Some(hint),
            ..self
        }
    }

    #[must_use]
    pub fn candidates<I>(self, candidates: I) -> Self
    where
        I: IntoIterator<Item = OwnedServerName>,
    {
        Self {
            candidates: candidates.into_iter().collect(),
            ..self
        }
    }

    #[must_use]
    pub fn room_version(self, room_version: RoomVersionId) -> Self {
        Self {
            room_version: Some(room_version),
            ..self
        }
    }

    #[must_use]
    pub fn attempt_limit(self, attempt_limit: NonZeroUsize) -> Self {
        Self {
            attempt_limit: Some(attempt_limit),
            ..self
        }
    }

    #[must_use]
    pub fn backfill_limit(self, backfill_limit: NonZeroUsize) -> Self {
        Self {
            backfill_limit: Some(backfill_limit),
            ..self
        }
    }

    #[must_use]
    pub fn fanout(self, growth: FanoutGrowth) -> Self {
        Self {
            fanout_growth: growth,
            ..self
        }
    }

    #[must_use]
    pub fn fanout_max_width(self, max_width: NonZeroUsize) -> Self {
        Self {
            fanout_max_width: Some(max_width),
            ..self
        }
    }

    #[must_use]
    pub fn fanout_rounds(self, rounds: NonZeroUsize) -> Self {
        Self {
            fanout_rounds: Some(rounds),
            ..self
        }
    }

    #[must_use]
    pub fn fanout_for_op(self) -> Self {
        use FanoutGrowth::{Geometric, Linear};

        const ONE: NonZeroUsize = NonZeroUsize::new(1).expect("one is nonzero");
        const TWO: NonZeroUsize = NonZeroUsize::new(2).expect("two is nonzero");
        const THREE: NonZeroUsize = NonZeroUsize::new(3).expect("three is nonzero");
        const FOUR: NonZeroUsize = NonZeroUsize::new(4).expect("four is nonzero");
        const FIVE: NonZeroUsize = NonZeroUsize::new(5).expect("five is nonzero");

        match self.op {
            Op::AuthEvent => self
                .fanout(Geometric {
                    base: ONE,
                    factor: TWO,
                })
                .fanout_max_width(FOUR)
                .fanout_rounds(FIVE),
            Op::AuthChain => self
                .fanout(Linear {
                    base: ONE,
                    step: ONE,
                })
                .fanout_max_width(TWO)
                .fanout_rounds(TWO),
            Op::StateIds => self
                .fanout(Linear {
                    base: ONE,
                    step: ONE,
                })
                .fanout_max_width(THREE)
                .fanout_rounds(THREE),
            Op::MissingEvents => self
                .fanout(Geometric {
                    base: ONE,
                    factor: TWO,
                })
                .fanout_rounds(THREE),
            Op::Event | Op::Backfill | Op::TimestampToEvent => self,
        }
    }

    #[must_use]
    pub fn checks(self, enabled: bool) -> Self {
        Self {
            check_event_id: enabled,
            check_conforms: enabled,
            check_hashes: enabled,
            check_signature: enabled,
            ..self
        }
    }
}

#[derive(Debug)]
pub struct Outcome {
    pub bytes: Bytes,

    pub origin: OwnedServerName,
}
