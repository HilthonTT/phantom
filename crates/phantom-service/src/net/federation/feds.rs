mod fold;

use std::{fmt::Debug, num::NonZeroUsize, time::Duration};

use futures::{Stream, StreamExt, future::Either};
use phantom_core::{
    Error, Result, implement,
    math::effective_cap,
    stream::{BroadbandExt, ReadyExt},
    time::now_secs,
};
use ruma::{
    OwnedServerName, RoomId, ServerName,
    api::{
        Metadata, OutgoingRequest, federation::authentication::ServerSignatures,
        path_builder::SinglePath,
    },
};
use tokio::time::{Instant, timeout};

pub use self::fold::{Faults, Grid, Origins, OutcomeExt, Tally};
use super::{Classification, Service};

const WIDTH_DEFAULT: NonZeroUsize = NonZeroUsize::new(32).expect("width is nonzero");

const TIMEOUT_DEFAULT: Duration = Duration::from_secs(15);

pub trait Request:
    OutgoingRequest
    + Metadata<Authentication = ServerSignatures, PathBuilder = SinglePath>
    + Debug
    + Send
{
}

impl<T> Request for T where
    T: OutgoingRequest
        + Metadata<Authentication = ServerSignatures, PathBuilder = SinglePath>
        + Debug
        + Send
{
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Opts {
    pub width: Option<NonZeroUsize>,

    pub timeout: Option<Duration>,

    pub sweep_deadline: Option<Duration>,

    pub exclude_self: bool,

    pub skip_backoff: bool,

    pub record: Record,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Record {
    #[default]
    Observe,

    Contribute,
}

#[derive(Debug)]
pub struct Outcome<R> {
    pub origin: OwnedServerName,

    pub elapsed: Duration,

    pub result: Result<R, Fault>,
}

#[derive(Debug)]
pub enum Fault {
    Elapsed,

    NotAttempted,

    Backoff {
        class: Classification,

        age: Duration,

        retry: Duration,
    },

    Error(Error),
}

#[implement(Service)]
pub fn for_room<'a, F, R>(
    &'a self,
    room_id: &'a RoomId,
    make: F,
    opts: Opts,
) -> impl Stream<Item = Outcome<R::IncomingResponse>> + Send + 'a
where
    F: Fn(&ServerName) -> R + Send + 'a,
    R: Request + 'a,
    R::IncomingResponse: Send,
{
    let dests = self
        .services
        .state_cache
        .room_servers(room_id)
        .ready_filter(move |server| {
            !opts.exclude_self || !self.services.server_state.server_is_ours(server)
        })
        .map(ToOwned::to_owned);

    self.fanout_to(dests, make, opts)
}

#[implement(Service)]
pub fn fanout_to<'a, D, F, R>(
    &'a self,
    dests: D,
    make: F,
    opts: Opts,
) -> impl Stream<Item = Outcome<R::IncomingResponse>> + Send + 'a
where
    D: Stream<Item = OwnedServerName> + Send + 'a,
    F: Fn(&ServerName) -> R + Send + 'a,
    R: Request + 'a,
    R::IncomingResponse: Send,
{
    let pairs = dests.map(move |origin| {
        let request = make(&origin);

        (origin, request)
    });

    self.fanout(pairs, opts)
}

#[implement(Service)]
pub fn fanout<'a, S, R>(
    &'a self,
    pairs: S,
    opts: Opts,
) -> impl Stream<Item = Outcome<R::IncomingResponse>> + Send + 'a
where
    S: Stream<Item = (OwnedServerName, R)> + Send + 'a,
    R: Request + 'a,
    R::IncomingResponse: Send,
{
    let config = &self.services.server.config.network;
    let opts = resolve_opts(opts, config.feds_max_width, config.feds_timeout);
    let client = &self.services.client.federation;
    let record = opts.record;

    self.gate(pairs, opts)
        .broadn_then(
            opts.width.map(NonZeroUsize::get),
            move |request| async move {
                let (origin, start, remaining, request) = match request {
                    Either::Left(outcome) => return outcome,
                    Either::Right(dispatch) => dispatch,
                };

                let sent = async {
                    match record {
                        Record::Observe => self.execute_uncounted(client, &origin, request).await,
                        Record::Contribute => self.execute_with(client, &origin, request).await,
                    }
                };

                let result = match timeout(remaining, sent).await {
                    Ok(result) => result.map_err(Fault::Error),
                    Err(_elapsed) => Err(Fault::Elapsed),
                };

                Outcome {
                    origin,
                    elapsed: start.elapsed(),
                    result,
                }
            },
        )
        .inspect(move |outcome| {
            if record == Record::Contribute && matches!(&outcome.result, Err(Fault::Elapsed)) {
                self.record_failure(&outcome.origin, Classification::Transient);
            }
        })
}

#[implement(Service)]
fn gate<'a, S, R>(
    &'a self,
    pairs: S,
    opts: Opts,
) -> impl Stream<Item = Either<Outcome<R::IncomingResponse>, Dispatch<R>>> + Send + 'a
where
    S: Stream<Item = (OwnedServerName, R)> + Send + 'a,
    R: Request + 'a,
    R::IncomingResponse: Send,
{
    let request_timeout = opts.timeout.unwrap_or(TIMEOUT_DEFAULT);
    let deadline = opts
        .sweep_deadline
        .and_then(|duration| Instant::now().checked_add(duration));

    pairs.then(move |(origin, request)| async move {
        let start = Instant::now();

        if deadline.is_some_and(|deadline| start >= deadline) {
            return Either::Left(Outcome {
                origin,
                elapsed: Duration::ZERO,
                result: Err(Fault::NotAttempted),
            });
        }

        if opts.skip_backoff
            && let Some(fault) = self.backoff_fault(&origin).await
        {
            return Either::Left(Outcome {
                origin,
                elapsed: Duration::ZERO,
                result: Err(fault),
            });
        }

        let remaining = deadline.map_or(request_timeout, |deadline| {
            deadline
                .saturating_duration_since(start)
                .min(request_timeout)
        });

        Either::Right((origin, start, remaining, request))
    })
}

#[implement(Service)]
async fn backoff_fault(&self, server: &ServerName) -> Option<Fault> {
    let backoff = self.peer_backoff(server).await?;
    let now = now_secs();

    let earliest = backoff.anchor_secs.saturating_add(backoff.delay_secs);
    if now >= earliest {
        return None;
    }

    Some(Fault::Backoff {
        class: backoff.class,
        age: Duration::from_secs(now.saturating_sub(backoff.oldest_secs)),
        retry: Duration::from_secs(earliest.saturating_sub(now)),
    })
}

type Dispatch<R> = (OwnedServerName, Instant, Duration, R);

fn resolve_opts(opts: Opts, config_width: usize, config_timeout: u64) -> Opts {
    let width = NonZeroUsize::new(effective_cap(opts.width, config_width))
        .filter(|_| opts.width.is_some() || config_width != 0)
        .unwrap_or(WIDTH_DEFAULT);

    Opts {
        width: Some(width),
        timeout: Some(
            opts.timeout
                .unwrap_or_else(|| Duration::from_secs(config_timeout)),
        ),
        ..opts
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, time::Duration};

    use super::{Opts, WIDTH_DEFAULT, resolve_opts};

    const FOUR: NonZeroUsize = NonZeroUsize::new(4).expect("nonzero");
    const SIXTY_FOUR: NonZeroUsize = NonZeroUsize::new(64).expect("nonzero");

    #[test]
    fn neither_side_naming_a_width_takes_the_builtin() {
        let opts = resolve_opts(Opts::default(), 0, 15);

        assert_eq!(opts.width, Some(WIDTH_DEFAULT));
    }

    #[test]
    fn the_operators_width_applies_when_the_caller_is_silent() {
        let opts = resolve_opts(Opts::default(), 8, 15);

        assert_eq!(opts.width.map(NonZeroUsize::get), Some(8));
    }

    #[test]
    fn the_smaller_of_the_two_widths_wins() {
        let wide = Opts {
            width: Some(SIXTY_FOUR),
            ..Opts::default()
        };
        assert_eq!(
            resolve_opts(wide, 8, 15).width.map(NonZeroUsize::get),
            Some(8)
        );

        let narrow = Opts {
            width: Some(FOUR),
            ..Opts::default()
        };
        assert_eq!(
            resolve_opts(narrow, 8, 15).width.map(NonZeroUsize::get),
            Some(4)
        );
    }

    #[test]
    fn an_unconfigured_operator_defers_to_the_caller() {
        let opts = Opts {
            width: Some(SIXTY_FOUR),
            ..Opts::default()
        };

        assert_eq!(
            resolve_opts(opts, 0, 15).width.map(NonZeroUsize::get),
            Some(64)
        );
    }

    #[test]
    fn the_timeout_falls_back_to_the_configured_one() {
        let opts = resolve_opts(Opts::default(), 0, 15);
        assert_eq!(opts.timeout, Some(Duration::from_secs(15)));

        let named = Opts {
            timeout: Some(Duration::from_secs(3)),
            ..Opts::default()
        };
        assert_eq!(
            resolve_opts(named, 0, 15).timeout,
            Some(Duration::from_secs(3))
        );
    }
}
