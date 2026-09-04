//! Tests against a real database.
//!
//! Each opens one under a temporary directory with two columns rather than the
//! whole schema, and drops it — and the directory — at the end. The engine is
//! what these are testing against: the map layer's behaviour around cache
//! misses, iteration bounds and record separators is not reproducible against
//! a stand-in.

use std::sync::Arc;

use figment::{
    Figment,
    providers::{Format, Toml},
};
use futures::TryStreamExt;
use phantom_core::{
    Config, Result,
    log::{Log, LogLevelReloadHandles, capture},
    server::Server,
};
use tempfile::TempDir;

use crate::{
    Database, Deserialized, Interfix, Txn,
    engine::descriptor::{self, Descriptor},
    keyval::serialize_key,
};

/// A column with values written across the keyspace, which is what most of
/// the schema looks like.
static RANDOM: Descriptor = Descriptor {
    name: "random",
    ..descriptor::RANDOM_SMALL
};

/// A second column, to check that they do not see each other's entries.
static OTHER: Descriptor = Descriptor {
    name: "other",
    ..descriptor::RANDOM_SMALL
};

static COLUMNS: &[Descriptor] = &[RANDOM, OTHER];

/// An open database, and the directory it will be removed with.
struct TestDb {
    db: Arc<Database>,
    _dir: TempDir,
}

impl TestDb {
    fn open() -> Result<Self> {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().display();

        let config = Config::new(
            &Figment::new().merge(
                Toml::string(&format!(
                    r#"
            [global]
            server_name = "phantom.test"
            database_path = "{path}"

            # One worker and one queue: the tests care about what comes back,
            # not about how wide the pool is, and this keeps them cheap.
            db_pool_workers = 1
            db_pool_queue_mult = 1
            db_cache_capacity_mb = 8.0
            db_write_buffer_capacity_mb = 8.0
            "#
                ))
                .nested(),
            ),
        )?;

        let log = Log {
            reload: LogLevelReloadHandles::default(),
            capture: Arc::new(capture::State::new()),
        };

        let server = Arc::new(Server::new(config, None, log));

        Ok(Self {
            db: Database::open_list(&server, COLUMNS)?,
            _dir: dir,
        })
    }
}

fn db() -> TestDb {
    TestDb::open().expect("the database opened")
}

#[tokio::test]
async fn a_written_value_reads_back() {
    let test = db();
    let map = &test.db["random"];

    map.put(("room", 1_u64), ("value",)).expect("written");

    let handle = map.qry(&("room", 1_u64)).await.expect("found");

    let borrowed: (&str,) = handle.de().expect("deserialized");
    assert_eq!(borrowed, ("value",));

    let owned: (String,) = handle.deserialized().expect("deserialized");
    assert_eq!(owned, ("value".to_owned(),));
}

#[tokio::test]
async fn a_missing_key_is_not_found() {
    let test = db();
    let map = &test.db["random"];

    let err = map.qry(&("absent", 1_u64)).await.expect_err("not found");

    assert!(
        err.is_not_found(),
        "a missing key reports not-found, got: {err}"
    );
    assert!(!map.contains(&("absent", 1_u64)).await);
}

#[tokio::test]
async fn columns_do_not_see_each_other() {
    let test = db();

    test.db["random"].put(("k",), ("v",)).expect("written");

    assert!(test.db["random"].contains(&("k",)).await);
    assert!(
        !test.db["other"].contains(&("k",)).await,
        "a key written to one column must not appear in another"
    );
}

#[tokio::test]
async fn a_removed_key_is_gone() {
    let test = db();
    let map = &test.db["random"];

    map.put(("k",), ("v",)).expect("written");
    map.del(("k",)).expect("removed");

    assert!(!map.contains(&("k",)).await);

    map.del(("k",)).expect("removing an absent key is fine");
}

#[tokio::test]
async fn entries_iterate_in_key_order() {
    let test = db();
    let map = &test.db["random"];

    for i in [3_u64, 1, 2] {
        map.put(("room", i), (i,)).expect("written");
    }

    let keys: Vec<(String, u64)> = map
        .keys::<(&str, u64)>()
        .map_ok(|(room, i)| (room.to_owned(), i))
        .try_collect()
        .await
        .expect("iterated");

    assert_eq!(
        keys,
        [
            ("room".to_owned(), 1),
            ("room".to_owned(), 2),
            ("room".to_owned(), 3),
        ]
    );
}

#[tokio::test]
async fn a_reverse_iteration_is_the_forward_one_backwards() {
    let test = db();
    let map = &test.db["random"];

    for i in 1_u64..=3 {
        map.put(("room", i), (i,)).expect("written");
    }

    let forward: Vec<u64> = map
        .keys::<(&str, u64)>()
        .map_ok(|(_, i)| i)
        .try_collect()
        .await
        .expect("iterated");

    let reverse: Vec<u64> = map
        .rev_keys::<(&str, u64)>()
        .map_ok(|(_, i)| i)
        .try_collect()
        .await
        .expect("iterated");

    assert_eq!(forward, [1, 2, 3]);
    assert_eq!(reverse, [3, 2, 1]);
}

/// The bug this exists for: seeking backwards from a prefix lands *before*
/// the prefix's range, so a reverse prefix iteration that seeks to the prefix
/// yields nothing. It has to start at the end of the range instead.
#[tokio::test]
async fn a_reverse_prefix_iteration_starts_at_the_end_of_the_prefix() {
    let test = db();
    let map = &test.db["random"];

    for i in 1_u64..=3 {
        map.put(("room", i), (i,)).expect("written");
    }

    let found: Vec<u64> = map
        .rev_keys_prefix::<(&str, u64), _>(&("room", Interfix))
        .map_ok(|(_, i)| i)
        .try_collect()
        .await
        .expect("iterated");

    assert_eq!(found, [3, 2, 1], "every entry, newest first");
}

#[tokio::test]
async fn a_prefix_iteration_stops_at_the_end_of_the_prefix() {
    let test = db();
    let map = &test.db["random"];

    map.put(("room", 1_u64), (1_u64,)).expect("written");
    map.put(("room", 2_u64), (2_u64,)).expect("written");

    map.put(("roomier", 1_u64), (9_u64,)).expect("written");

    let forward: Vec<u64> = map
        .keys_prefix::<(&str, u64), _>(&("room", Interfix))
        .map_ok(|(_, i)| i)
        .try_collect()
        .await
        .expect("iterated");

    let reverse: Vec<u64> = map
        .rev_keys_prefix::<(&str, u64), _>(&("room", Interfix))
        .map_ok(|(_, i)| i)
        .try_collect()
        .await
        .expect("iterated");

    assert_eq!(forward, [1, 2]);
    assert_eq!(reverse, [2, 1]);
    assert_eq!(map.count_prefix(&("room", Interfix)).await, 2);
}

#[tokio::test]
async fn iterating_from_a_bound_skips_what_precedes_it() {
    let test = db();
    let map = &test.db["random"];

    for i in 1_u64..=4 {
        map.put(("room", i), (i,)).expect("written");
    }

    let forward: Vec<u64> = map
        .keys_from::<(&str, u64), _>(&("room", 3_u64))
        .map_ok(|(_, i)| i)
        .try_collect()
        .await
        .expect("iterated");

    let reverse: Vec<u64> = map
        .rev_keys_from::<(&str, u64), _>(&("room", 2_u64))
        .map_ok(|(_, i)| i)
        .try_collect()
        .await
        .expect("iterated");

    assert_eq!(forward, [3, 4], "at or after the bound");
    assert_eq!(reverse, [2, 1], "at or before the bound");
}

#[tokio::test]
async fn a_stream_yields_both_halves() {
    let test = db();
    let map = &test.db["random"];

    map.put(("room", 1_u64), ("first",)).expect("written");
    map.put(("room", 2_u64), ("second",)).expect("written");

    let entries: Vec<(u64, String)> = map
        .stream::<(&str, u64), (&str,)>()
        .map_ok(|((_, i), (val,))| (i, val.to_owned()))
        .try_collect()
        .await
        .expect("iterated");

    assert_eq!(entries, [(1, "first".to_owned()), (2, "second".to_owned())]);
}

#[tokio::test]
async fn a_batch_write_is_one_flush_and_reads_back_whole() {
    let test = db();
    let map = &test.db["random"];

    let entries: Vec<_> = (1_u64..=4)
        .map(|i| {
            (
                crate::serialize_key(("room", i)).expect("serialized"),
                crate::serialize_val((i,)).expect("serialized"),
            )
        })
        .collect();

    map.insert_batch(entries.iter().map(|(k, v)| (k, v)))
        .expect("written");

    assert_eq!(map.count().await, 4);
}

#[tokio::test]
async fn a_batch_read_answers_in_order() {
    let test = db();
    let map = &test.db["random"];

    for i in 1_u64..=3 {
        map.put(("room", i), (i,)).expect("written");
    }

    let keys = futures::stream::iter([("room", 3_u64), ("room", 1), ("room", 2)]);
    let vals: Vec<u64> = map
        .qry_batch(keys)
        .map_ok(|handle| handle.deserialized::<(u64,)>().expect("deserialized").0)
        .try_collect()
        .await
        .expect("read");

    assert_eq!(vals, [3, 1, 2], "answers follow the order asked for");
}

#[tokio::test]
async fn clearing_empties_the_column_and_leaves_the_others() {
    let test = db();

    for i in 1_u64..=3 {
        test.db["random"].put(("k", i), (i,)).expect("written");
    }
    test.db["other"].put(("k",), (0_u64,)).expect("written");

    test.db["random"].clear().await;

    assert_eq!(test.db["random"].count().await, 0);
    assert_eq!(test.db["other"].count().await, 1);
}

/// What `watch_prefix` is for: a task parked on a prefix wakes when a write
/// lands under it.
#[tokio::test]
async fn a_write_wakes_a_prefix_watcher() {
    let test = db();
    let map = &test.db["random"];

    let key = crate::serialize_key(("room", 1_u64)).expect("serialized");
    let prefix = crate::serialize_key(("room", Interfix)).expect("serialized");
    let watch = map.watch_prefix(prefix.as_slice());

    map.insert(&key, b"value").expect("written");

    tokio::time::timeout(std::time::Duration::from_secs(10), watch)
        .await
        .expect("the watcher was woken");
}

#[tokio::test]
async fn a_cork_holds_the_flush_until_it_drops() {
    let test = db();
    let map = &test.db["random"];

    {
        let _cork = test.db.engine.cork_and_flush();
        assert!(test.db.engine.corked());

        for i in 1_u64..=8 {
            map.put(("room", i), (i,)).expect("written");
        }
    }

    assert!(!test.db.engine.corked(), "the cork released on drop");
    assert_eq!(map.count().await, 8, "the writes landed either way");
}

#[tokio::test]
async fn every_described_column_opens() {
    let test = db();

    assert_eq!(test.db.keys().count(), COLUMNS.len());
    assert!(test.db.get("random").is_ok());
    assert!(
        test.db.get("nonexistent").is_err(),
        "an undescribed column is not found rather than panicking"
    );
}

/// Several databases opening and closing at once must not deadlock.
///
/// The environment carrying the engine's background threads is the process's,
/// not the database's: `Env::new` hands back the one RocksDB keeps for the
/// process rather than building a new one. Shutting its thread pools down when
/// a database closes therefore takes them away from every other database still
/// open — which then waits forever for background work nothing is left to run —
/// and two closes doing it at once join the same threads twice.
///
/// This is what the suite does implicitly, since it runs in parallel and every
/// test opens its own database; here it is on purpose, so the failure names
/// itself rather than surfacing as three unrelated tests that never finish.
#[tokio::test]
async fn concurrent_databases_close_without_deadlocking() {
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                let test = db();
                let map = &test.db["random"];

                for i in 1_u64..=64 {
                    map.put(("room", i), (i,)).expect("written");
                }

                test.db.engine.sort().expect("flushed");
            });
        }
    });
}

/// Why `iter_prefix` bounds the cursor instead of just seeking to the prefix.
///
/// Seeking backwards lands on the last key at or before the target, and every
/// key inside a prefix's range sorts *after* the prefix — so seeking backwards
/// to the prefix itself lands before the range and the first step leaves it
/// again. This is the naive formulation, kept to show that it finds nothing.
#[tokio::test]
async fn seeking_backwards_to_a_prefix_lands_before_its_range() {
    let test = db();
    let map = &test.db["random"];

    for i in 1_u64..=3 {
        map.put(("room", i), (i,)).expect("written");
    }

    let prefix = crate::serialize_key(("room", Interfix)).expect("serialized");

    let found: Vec<_> = map
        .rev_raw_keys_from(&prefix)
        .try_take_while(|k| futures::future::ok(k.starts_with(&prefix)))
        .try_collect::<Vec<_>>()
        .await
        .expect("iterated");

    assert!(
        found.is_empty(),
        "the reference formulation yields nothing; got {} entries",
        found.len()
    );
}

#[tokio::test]
async fn a_prefix_is_deleted_whole() {
    let test = db();
    let map = &test.db["random"];

    for i in 0_u64..4 {
        map.put(("room", i), ("v",)).expect("written");
    }
    map.put(("other_room", 0_u64), ("v",)).expect("written");

    map.del_prefix(&("room", Interfix)).await;

    for i in 0_u64..4 {
        assert!(!map.contains(&("room", i)).await, "{i} survived");
    }
    assert!(
        map.contains(&("other_room", 0_u64)).await,
        "a sibling prefix was taken with it"
    );
}

/// The prefix ends at the record separator, so it must not reach a key whose
/// first component merely begins with the same bytes.
#[tokio::test]
async fn deleting_a_prefix_stops_at_the_separator() {
    let test = db();
    let map = &test.db["random"];

    map.put(("ab", 1_u64), ("v",)).expect("written");
    map.put(("abc", 1_u64), ("v",)).expect("written");

    map.del_prefix(&("ab", Interfix)).await;

    assert!(!map.contains(&("ab", 1_u64)).await);
    assert!(map.contains(&("abc", 1_u64)).await, "abc is not under ab");
}

#[tokio::test]
async fn deleting_an_absent_prefix_is_not_an_error() {
    let test = db();

    test.db["random"].del_prefix(&("nothing", Interfix)).await;
}

#[tokio::test]
async fn a_transaction_writes_across_columns() {
    let test = db();

    let mut txn = Txn::new(&test.db.engine);
    txn.put(&test.db["random"], ("k", 1_u64), ("one",))
        .expect("queued");
    txn.put(&test.db["other"], ("k", 2_u64), ("two",))
        .expect("queued");

    assert_eq!(txn.len(), 2);
    txn.execute().expect("committed");

    let one: (String,) = test.db["random"]
        .qry(&("k", 1_u64))
        .await
        .expect("found")
        .deserialized()
        .expect("deserialized");
    assert_eq!(one, ("one".to_owned(),));

    let two: (String,) = test.db["other"]
        .qry(&("k", 2_u64))
        .await
        .expect("found")
        .deserialized()
        .expect("deserialized");
    assert_eq!(two, ("two".to_owned(),));
}

/// The pattern the transaction exists for: a value moved between columns,
/// where landing halfway would leave it in both or in neither.
#[tokio::test]
async fn a_transaction_moves_a_value_between_columns() {
    let test = db();
    let from = &test.db["random"];
    let to = &test.db["other"];

    from.put(("k", 1_u64), ("v",)).expect("written");

    let mut txn = Txn::new(&test.db.engine);
    txn.put(to, ("k", 1_u64), ("v",)).expect("queued");
    txn.del(from, ("k", 1_u64)).expect("queued");
    txn.execute().expect("committed");

    assert!(!from.contains(&("k", 1_u64)).await, "not removed");
    assert!(to.contains(&("k", 1_u64)).await, "not written");
}

/// Building one and dropping it leaves the database as it was, which is what
/// makes bailing out part-way through safe.
#[tokio::test]
async fn a_dropped_transaction_writes_nothing() {
    let test = db();
    let map = &test.db["random"];

    let mut txn = Txn::new(&test.db.engine);
    txn.put(map, ("k", 1_u64), ("v",)).expect("queued");
    drop(txn);

    assert!(!map.contains(&("k", 1_u64)).await);
}

#[tokio::test]
async fn an_empty_transaction_is_a_no_op() {
    let test = db();

    let txn = Txn::new(&test.db.engine);

    assert!(txn.is_empty());
    txn.execute().expect("committed");
}

/// A watcher is woken by a transaction the same as by a direct write, which is
/// what keeps a long-poll from sleeping through one.
#[tokio::test]
async fn a_transaction_wakes_the_watchers() {
    let test = db();
    let map = &test.db["random"];

    let watch = map.watch_prefix(&serialize_key(("room", Interfix)).expect("serialized"));

    let mut txn = Txn::new(&test.db.engine);
    txn.put(map, ("room", 1_u64), ("v",)).expect("queued");
    txn.execute().expect("committed");

    tokio::time::timeout(std::time::Duration::from_secs(5), watch)
        .await
        .expect("the watcher was woken");
}
