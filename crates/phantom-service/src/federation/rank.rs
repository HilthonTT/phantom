use futures::StreamExt;
use phantom_core::{debug_warn, implement, stream::IterStream};
use ruma::OwnedServerName;
use smallvec::SmallVec;

use super::{Service, ShouldAttempt};

pub type Candidates = SmallVec<[OwnedServerName; 3]>;

type Verdicts = SmallVec<[(OwnedServerName, ShouldAttempt); 3]>;

#[derive(Clone, Copy, Debug)]
pub enum WhenAllBackedOff {
    Attempt,

    Fail,
}

#[implement(Service)]
pub async fn rank_candidates(
    &self,
    eligible: Candidates,
    when_all: WhenAllBackedOff,
) -> Candidates {
    let verdicts: Verdicts = eligible
        .into_iter()
        .stream()
        .then(async |server| {
            let verdict = self.should_attempt(&server).await;

            (server, verdict)
        })
        .collect()
        .await;

    rank_from_verdicts(verdicts, when_all).collect()
}

fn rank_from_verdicts(
    mut verdicts: Verdicts,
    when_all: WhenAllBackedOff,
) -> impl Iterator<Item = OwnedServerName> {
    let all_backed_off = verdicts
        .iter()
        .all(|(_, verdict)| matches!(verdict, ShouldAttempt::No { .. }));

    let keep_backed_off = all_backed_off && matches!(when_all, WhenAllBackedOff::Attempt);

    if keep_backed_off && !verdicts.is_empty() {
        debug_warn!(
            n = verdicts.len(),
            "Every candidate is backed off; attempting anyway"
        );
    }

    verdicts.sort_by_key(|(_, verdict)| verdict.rank());

    verdicts
        .into_iter()
        .filter(move |(_, verdict)| keep_backed_off || !matches!(verdict, ShouldAttempt::No { .. }))
        .map(|(server, _)| server)
}

#[implement(ShouldAttempt)]
#[inline]
fn rank(self) -> u8 {
    match self {
        ShouldAttempt::Yes => 0,
        ShouldAttempt::Deprioritize => 1,
        ShouldAttempt::No { .. } => 2,
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use ruma::{OwnedServerName, owned_server_name};
    use smallvec::smallvec;

    use super::{Verdicts, WhenAllBackedOff, rank_from_verdicts};
    use crate::federation::ShouldAttempt;

    fn no() -> ShouldAttempt {
        ShouldAttempt::No {
            earliest_retry: SystemTime::UNIX_EPOCH,
        }
    }

    fn names(servers: &[OwnedServerName]) -> Vec<&str> {
        servers.iter().map(AsRef::as_ref).collect()
    }

    #[test]
    fn all_clean_preserves_the_callers_order() {
        let verdicts: Verdicts = smallvec![
            (owned_server_name!("a.test"), ShouldAttempt::Yes),
            (owned_server_name!("b.test"), ShouldAttempt::Yes),
            (owned_server_name!("c.test"), ShouldAttempt::Yes),
        ];

        let ranked: Vec<_> = rank_from_verdicts(verdicts, WhenAllBackedOff::Attempt).collect();

        assert_eq!(names(&ranked), ["a.test", "b.test", "c.test"]);
    }

    #[test]
    fn backed_off_is_dropped_where_there_is_an_alternative() {
        let verdicts: Verdicts = smallvec![
            (owned_server_name!("a.test"), ShouldAttempt::Yes),
            (owned_server_name!("b.test"), no()),
            (owned_server_name!("c.test"), ShouldAttempt::Yes),
        ];

        let ranked: Vec<_> = rank_from_verdicts(verdicts, WhenAllBackedOff::Attempt).collect();

        assert_eq!(names(&ranked), ["a.test", "c.test"]);
    }

    #[test]
    fn all_backed_off_falls_through_when_asked_to() {
        let verdicts: Verdicts = smallvec![
            (owned_server_name!("a.test"), no()),
            (owned_server_name!("b.test"), no()),
        ];

        let ranked: Vec<_> = rank_from_verdicts(verdicts, WhenAllBackedOff::Attempt).collect();

        assert_eq!(names(&ranked), ["a.test", "b.test"]);
    }

    #[test]
    fn all_backed_off_empties_the_pool_otherwise() {
        let verdicts: Verdicts = smallvec![
            (owned_server_name!("a.test"), no()),
            (owned_server_name!("b.test"), no()),
        ];

        assert!(
            rank_from_verdicts(verdicts, WhenAllBackedOff::Fail)
                .next()
                .is_none()
        );
    }

    #[test]
    fn a_recently_failed_server_sorts_after_a_clean_one() {
        let verdicts: Verdicts = smallvec![
            (owned_server_name!("d.test"), ShouldAttempt::Deprioritize),
            (owned_server_name!("y.test"), ShouldAttempt::Yes),
            (owned_server_name!("n.test"), no()),
        ];

        let ranked: Vec<_> = rank_from_verdicts(verdicts, WhenAllBackedOff::Attempt).collect();

        assert_eq!(names(&ranked), ["y.test", "d.test"]);
    }

    #[test]
    fn a_deprioritized_server_keeps_the_pool_from_collapsing() {
        let verdicts: Verdicts = smallvec![
            (owned_server_name!("n.test"), no()),
            (owned_server_name!("d.test"), ShouldAttempt::Deprioritize),
        ];

        let ranked: Vec<_> = rank_from_verdicts(verdicts, WhenAllBackedOff::Attempt).collect();

        assert_eq!(names(&ranked), ["d.test"]);
    }
}
