//! Waking a task when a column is written under a prefix it cares about.
//!
//! This is what lets a long-poll — a client parked on `/sync` — sleep until
//! something it would report actually lands, rather than waking on a timer to
//! find nothing changed.

use std::{
    collections::{HashMap, hash_map::Entry},
    future::Future,
    sync::RwLock,
};

use tokio::sync::watch;

/// The prefixes currently being waited on, and the channel that wakes each
/// one's waiters.
///
/// A prefix is registered on first use and removed as soon as it fires: the
/// waiters have all been woken by then, and anyone still interested registers
/// again on their next pass. So the map holds only prefixes with a waiter
/// parked on them right now, which is what keeps [`Watchers::wake`] cheap on
/// the write path.
#[derive(Default)]
pub(crate) struct Watchers {
    watchers: RwLock<HashMap<Vec<u8>, watch::Sender<()>>>,
}

impl Watchers {
    /// A future that completes once a key beginning with `prefix` is written.
    ///
    /// The returned future borrows nothing, so it may be held across anything
    /// — including the drop of the map it came from, in which case it simply
    /// never completes.
    pub(crate) fn watch(&self, prefix: &[u8]) -> impl Future<Output = ()> + Send + use<> {
        let mut rx = match self
            .watchers
            .write()
            .expect("watchers lock is never held across a panic")
            .entry(prefix.to_owned())
        {
            Entry::Occupied(occupied) => occupied.get().subscribe(),
            Entry::Vacant(vacant) => vacant.insert(watch::channel(()).0).subscribe(),
        };

        async move {
            rx.changed().await.ok();
        }
    }

    /// Wakes everything waiting on a prefix of `key`.
    ///
    /// Runs on every write, so the no-waiters case — which is nearly all of
    /// them — must not cost more than one uncontended read lock.
    pub(crate) fn wake(&self, key: &[u8]) {
        let watchers = self
            .watchers
            .read()
            .expect("watchers lock is never held across a panic");

        if watchers.is_empty() {
            return;
        }

        let triggered: Vec<_> = if watchers.len() <= key.len() {
            watchers
                .keys()
                .filter(|prefix| key.starts_with(prefix))
                .cloned()
                .collect()
        } else {
            (0..=key.len())
                .map(|len| &key[..len])
                .filter(|prefix| watchers.contains_key(*prefix))
                .map(<[u8]>::to_owned)
                .collect()
        };

        drop(watchers);

        if triggered.is_empty() {
            return;
        }

        let mut watchers = self
            .watchers
            .write()
            .expect("watchers lock is never held across a panic");

        for prefix in triggered {
            if let Some(tx) = watchers.remove(&prefix) {
                tx.send(()).ok();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::FutureExt;

    use super::*;

    #[tokio::test]
    async fn a_write_under_the_prefix_wakes_the_waiter() {
        let watchers = Watchers::default();
        let watch = watchers.watch(b"!room").boxed();

        watchers.wake(b"!room\xFFevent");

        tokio::time::timeout(Duration::from_secs(5), watch)
            .await
            .expect("the waiter was woken");
    }

    #[tokio::test]
    async fn a_write_elsewhere_leaves_the_waiter_parked() {
        let watchers = Watchers::default();
        let mut watch = watchers.watch(b"!room").boxed();

        watchers.wake(b"!other\xFFevent");

        assert!(
            watch.as_mut().now_or_never().is_none(),
            "an unrelated write must not wake the waiter"
        );
    }

    /// The key itself counts as one of its own prefixes, so an exact-match
    /// watch fires too.
    #[tokio::test]
    async fn an_exact_key_wakes_its_watcher() {
        let watchers = Watchers::default();
        let watch = watchers.watch(b"!room").boxed();

        watchers.wake(b"!room");

        tokio::time::timeout(Duration::from_secs(5), watch)
            .await
            .expect("the waiter was woken");
    }

    /// Several tasks sharing a prefix share one channel and all wake together.
    #[tokio::test]
    async fn every_waiter_on_a_prefix_wakes() {
        let watchers = Watchers::default();
        let first = watchers.watch(b"!room").boxed();
        let second = watchers.watch(b"!room").boxed();

        assert_eq!(
            watchers.watchers.read().expect("unpoisoned").len(),
            1,
            "a shared prefix is one entry, not one per waiter"
        );

        watchers.wake(b"!room\xFFevent");

        tokio::time::timeout(Duration::from_secs(5), futures::future::join(first, second))
            .await
            .expect("both waiters were woken");
    }

    /// Both branches of the strategy choice in `wake` have to agree; which one
    /// runs is decided by the watcher count against the key length.
    #[tokio::test]
    async fn both_lookup_strategies_find_the_same_prefixes() {
        for keys in [1_usize, 64] {
            let watchers = Watchers::default();
            let watch = watchers.watch(b"a").boxed();

            for i in 0..keys {
                drop(watchers.watch(format!("z{i}").as_bytes()));
            }

            watchers.wake(b"ab");

            tokio::time::timeout(Duration::from_secs(5), watch)
                .await
                .expect("the waiter was woken");
        }
    }

    #[test]
    fn a_fired_prefix_is_forgotten() {
        let watchers = Watchers::default();
        drop(watchers.watch(b"!room"));

        watchers.wake(b"!room");

        assert!(
            watchers.watchers.read().expect("unpoisoned").is_empty(),
            "a fired prefix must not keep costing the write path"
        );
    }
}
