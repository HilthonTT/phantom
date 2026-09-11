use std::collections::{BTreeMap, BTreeSet};

use futures::{Stream, pin_mut};
use phantom_core::stream::ReadyExt;
use ruma::OwnedServerName;

use super::{Fault, Outcome};

pub type Origins = BTreeSet<OwnedServerName>;

pub type Faults = BTreeMap<OwnedServerName, Fault>;

pub struct Grid<K> {
    pub data: BTreeMap<K, Origins>,

    pub empty: Origins,

    pub faults: Faults,
}

pub struct Tally {
    pub ok: Origins,

    pub faults: Faults,
}

pub trait OutcomeExt<R>
where
    Self: Stream<Item = Outcome<R>> + Send + Sized,
    R: Send,
{
    fn merge<T, F>(self, init: T, merge: F) -> impl Future<Output = (T, Faults)> + Send
    where
        T: Send,
        F: Fn(T, R) -> T + Send;

    fn grid<K, I, F>(self, extract: F) -> impl Future<Output = Grid<K>> + Send
    where
        K: Ord + Send,
        I: IntoIterator<Item = K>,
        F: Fn(R) -> I + Send;

    fn tally(self) -> impl Future<Output = Tally> + Send;

    fn first_acceptable<F>(
        self,
        accept: F,
    ) -> impl Future<Output = Option<(OwnedServerName, R)>> + Send
    where
        F: Fn(&R) -> bool + Send;
}

impl<S, R> OutcomeExt<R> for S
where
    S: Stream<Item = Outcome<R>> + Send + Sized,
    R: Send,
{
    fn merge<T, F>(self, init: T, merge: F) -> impl Future<Output = (T, Faults)> + Send
    where
        T: Send,
        F: Fn(T, R) -> T + Send,
    {
        self.ready_fold(
            (init, Faults::new()),
            move |(merged, mut faults), outcome| match outcome.result {
                Ok(response) => (merge(merged, response), faults),
                Err(fault) => {
                    faults.insert(outcome.origin, fault);
                    (merged, faults)
                }
            },
        )
    }

    fn grid<K, I, F>(self, extract: F) -> impl Future<Output = Grid<K>> + Send
    where
        K: Ord + Send,
        I: IntoIterator<Item = K>,
        F: Fn(R) -> I + Send,
    {
        let grid = Grid {
            data: BTreeMap::new(),
            empty: Origins::new(),
            faults: Faults::new(),
        };

        self.ready_fold(grid, move |mut grid, outcome| {
            match outcome.result {
                Ok(response) => {
                    let mut data = extract(response).into_iter();

                    if let Some(mut datum) = data.next() {
                        for next in data {
                            grid.data
                                .entry(datum)
                                .or_default()
                                .insert(outcome.origin.clone());

                            datum = next;
                        }

                        grid.data.entry(datum).or_default().insert(outcome.origin);
                    } else {
                        grid.empty.insert(outcome.origin);
                    }
                }
                Err(fault) => {
                    grid.faults.insert(outcome.origin, fault);
                }
            }

            grid
        })
    }

    fn tally(self) -> impl Future<Output = Tally> + Send {
        let tally = Tally {
            ok: Origins::new(),
            faults: Faults::new(),
        };

        self.ready_fold(tally, |mut tally, outcome| {
            match outcome.result {
                Ok(_response) => {
                    tally.ok.insert(outcome.origin);
                }
                Err(fault) => {
                    tally.faults.insert(outcome.origin, fault);
                }
            }

            tally
        })
    }

    async fn first_acceptable<F>(self, accept: F) -> Option<(OwnedServerName, R)>
    where
        F: Fn(&R) -> bool + Send,
    {
        let outcomes = self;

        pin_mut!(outcomes);
        outcomes
            .ready_find_map(move |outcome| match outcome.result {
                Ok(response) if accept(&response) => Some((outcome.origin, response)),
                _ => None,
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use phantom_core::stream::IterStream;
    use ruma::{OwnedServerName, owned_server_name};

    use super::{Fault, Outcome, OutcomeExt};

    fn ok(origin: &str, response: &str) -> Outcome<String> {
        Outcome {
            origin: OwnedServerName::try_from(origin).expect("valid"),
            elapsed: Duration::ZERO,
            result: Ok(response.to_owned()),
        }
    }

    fn failed(origin: &str) -> Outcome<String> {
        Outcome {
            origin: OwnedServerName::try_from(origin).expect("valid"),
            elapsed: Duration::ZERO,
            result: Err(Fault::Elapsed),
        }
    }

    #[tokio::test]
    async fn merge_keeps_the_failures_aside() {
        let outcomes = vec![ok("a.test", "one"), failed("b.test"), ok("c.test", "two")];

        let (merged, faults) = outcomes
            .into_iter()
            .stream()
            .merge(Vec::new(), |mut acc: Vec<String>, response| {
                acc.push(response);
                acc
            })
            .await;

        assert_eq!(merged, ["one", "two"]);
        assert_eq!(faults.len(), 1);
        assert!(faults.contains_key(&owned_server_name!("b.test")));
    }

    #[tokio::test]
    async fn the_grid_inverts_responses_onto_their_origins() {
        let outcomes = vec![
            ok("a.test", "shared"),
            ok("b.test", "shared"),
            ok("c.test", "alone"),
        ];

        let grid = outcomes
            .into_iter()
            .stream()
            .grid(|response: String| [response])
            .await;

        assert_eq!(grid.data.len(), 2);
        assert_eq!(
            grid.data["shared"],
            [owned_server_name!("a.test"), owned_server_name!("b.test")]
                .into_iter()
                .collect()
        );
        assert_eq!(grid.data["alone"].len(), 1);
        assert!(grid.empty.is_empty());
    }

    #[tokio::test]
    async fn an_empty_response_is_not_a_fault() {
        let outcomes = vec![ok("a.test", "datum"), failed("b.test")];

        let grid = outcomes
            .into_iter()
            .stream()
            .grid(|response: String| {
                let _ = response;
                Vec::<String>::new()
            })
            .await;

        assert_eq!(
            grid.empty,
            [owned_server_name!("a.test")].into_iter().collect()
        );
        assert_eq!(grid.faults.len(), 1);
        assert!(grid.data.is_empty());
    }

    #[tokio::test]
    async fn the_tally_splits_answered_from_not() {
        let outcomes = vec![ok("a.test", "one"), failed("b.test"), ok("c.test", "two")];

        let tally = outcomes.into_iter().stream().tally().await;

        assert_eq!(
            tally.ok,
            [owned_server_name!("a.test"), owned_server_name!("c.test")]
                .into_iter()
                .collect()
        );
        assert_eq!(tally.faults.len(), 1);
    }

    #[tokio::test]
    async fn first_acceptable_skips_what_it_does_not_want() {
        let outcomes = vec![
            ok("a.test", "no"),
            failed("b.test"),
            ok("c.test", "yes"),
            ok("d.test", "yes"),
        ];

        let found = outcomes
            .into_iter()
            .stream()
            .first_acceptable(|response: &String| response == "yes")
            .await;

        assert_eq!(
            found,
            Some((owned_server_name!("c.test"), "yes".to_owned()))
        );
    }

    #[tokio::test]
    async fn first_acceptable_finds_nothing_in_an_all_failed_sweep() {
        let outcomes = vec![failed("a.test"), failed("b.test")];

        let found = outcomes
            .into_iter()
            .stream()
            .first_acceptable(|_: &String| true)
            .await;

        assert!(found.is_none());
    }
}
