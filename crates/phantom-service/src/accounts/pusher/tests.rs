//! The notified-event list against a real database under a temporary
//! directory: paging, the highlight filter, and the sweep.

use std::sync::Arc;

use phantom_core::{Error, err, http::StatusCode};
use phantom_database::{Database, Map, table};
use ruma::{
    UserId,
    api::error::ErrorKind,
    push::{Action, HighlightTweakValue, HttpPusherData, Tweak},
    user_id,
};
use tempfile::TempDir;

use crate::runtime::testing;

use super::{
    Notified, badge_count_disabled, is_permanent_failure,
    notified::{page, prune, put},
};

struct TestDb {
    db: Arc<Database>,
    _dir: TempDir,
}

impl TestDb {
    fn open() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");

        Self {
            db: Database::open(&testing::server(&dir, "")).expect("the database opened"),
            _dir: dir,
        }
    }

    fn map(&self) -> &Arc<Map> {
        &self.db[table::USERIDCOUNT_NOTIFICATION]
    }
}

fn notified(ts: u64, highlight: bool) -> Notified {
    let mut actions = vec![Action::Notify];
    if highlight {
        actions.push(Action::SetTweak(Tweak::Highlight(HighlightTweakValue::Yes)));
    }

    Notified {
        ts,
        sroomid: 7,
        tag: None,
        actions,
    }
}

fn fill(map: &Arc<Map>, user: &UserId, counts: impl IntoIterator<Item = u64>) {
    for count in counts {
        put(map, user, count, &notified(count, count % 3 == 0)).expect("written");
    }
}

fn counts(rows: &[(u64, Notified)]) -> Vec<u64> {
    rows.iter().map(|(count, _)| *count).collect()
}

#[tokio::test]
async fn a_row_reads_back_as_written() {
    let test = TestDb::open();
    let alice = user_id!("@alice:phantom.test");

    let row = notified(1234, true);
    put(test.map(), alice, 10, &row).expect("written");

    let (rows, next) = page(test.map(), alice, None, 10, false).await;

    assert_eq!(next, None);
    assert_eq!(rows.len(), 1);

    let (count, read) = &rows[0];
    assert_eq!(*count, 10);
    assert_eq!(read.ts, row.ts);
    assert_eq!(read.sroomid, row.sroomid);
    assert_eq!(read.tag, row.tag);
    assert_eq!(
        serde_json::to_value(&read.actions).expect("serialized"),
        serde_json::to_value(&row.actions).expect("serialized"),
    );
    assert!(read.is_highlight());
}

#[tokio::test]
async fn pages_run_newest_first_and_end_without_a_token() {
    let test = TestDb::open();
    let alice = user_id!("@alice:phantom.test");
    fill(test.map(), alice, 1..=5);

    let (first, next) = page(test.map(), alice, None, 2, false).await;
    assert_eq!(counts(&first), [5, 4]);
    assert_eq!(next, Some(4));

    let (second, next) = page(test.map(), alice, next, 2, false).await;
    assert_eq!(counts(&second), [3, 2]);
    assert_eq!(next, Some(2));

    let (last, next) = page(test.map(), alice, next, 2, false).await;
    assert_eq!(counts(&last), [1]);
    assert_eq!(next, None, "the last page carries no token");

    let (exact, next) = page(test.map(), alice, Some(3), 2, false).await;
    assert_eq!(counts(&exact), [2, 1]);
    assert_eq!(
        next, None,
        "a page that ends on the oldest row carries no token"
    );

    let (empty, next) = page(test.map(), alice, Some(0), 2, false).await;
    assert!(empty.is_empty());
    assert_eq!(next, None);
}

#[tokio::test]
async fn users_do_not_see_each_other() {
    let test = TestDb::open();
    let alice = user_id!("@alice:phantom.test");
    // Shares alice's bytes as a prefix, so a scan that overran her rows would
    // reach these.
    let alicia = user_id!("@alice:phantom.testing");
    let bob = user_id!("@bob:phantom.test");

    fill(test.map(), alice, [1, 4]);
    fill(test.map(), alicia, [2, 5]);
    fill(test.map(), bob, [3, 6]);

    let (rows, _) = page(test.map(), alice, None, 10, false).await;
    assert_eq!(counts(&rows), [4, 1]);

    let (rows, _) = page(test.map(), alicia, None, 10, false).await;
    assert_eq!(counts(&rows), [5, 2]);

    let (rows, _) = page(test.map(), bob, Some(6), 10, false).await;
    assert_eq!(counts(&rows), [3]);
}

#[tokio::test]
async fn only_highlight_skips_the_rest() {
    let test = TestDb::open();
    let alice = user_id!("@alice:phantom.test");
    // Every third count is a highlight.
    fill(test.map(), alice, 1..=10);

    let (rows, next) = page(test.map(), alice, None, 2, true).await;
    assert_eq!(counts(&rows), [9, 6]);
    assert!(rows.iter().all(|(_, row)| row.is_highlight()));
    assert_eq!(next, Some(6));

    let (rows, next) = page(test.map(), alice, next, 2, true).await;
    assert_eq!(counts(&rows), [3]);
    assert_eq!(next, None);
}

#[tokio::test]
async fn the_sweep_keeps_each_users_newest() {
    let test = TestDb::open();
    let alice = user_id!("@alice:phantom.test");
    let bob = user_id!("@bob:phantom.test");
    fill(test.map(), alice, 1..=5);
    fill(test.map(), bob, 6..=7);

    let removed = prune(test.map(), 100, 0, 3).await;
    assert_eq!(removed, 2);

    let (rows, _) = page(test.map(), alice, None, 10, false).await;
    assert_eq!(counts(&rows), [5, 4, 3]);

    let (rows, _) = page(test.map(), bob, None, 10, false).await;
    assert_eq!(
        counts(&rows),
        [7, 6],
        "a user under the cap keeps every row"
    );
}

#[tokio::test]
async fn the_sweep_drops_rows_past_their_age() {
    let test = TestDb::open();
    let alice = user_id!("@alice:phantom.test");
    // `fill` stamps each row with its count as the time.
    fill(test.map(), alice, [10, 20, 30, 40]);

    let removed = prune(test.map(), 45, 20, 0).await;
    assert_eq!(
        removed, 2,
        "rows stamped before 25 are older than 20ms at 45"
    );

    let (rows, _) = page(test.map(), alice, None, 10, false).await;
    assert_eq!(counts(&rows), [40, 30]);
}

#[tokio::test]
async fn a_disabled_sweep_removes_nothing() {
    let test = TestDb::open();
    let alice = user_id!("@alice:phantom.test");
    fill(test.map(), alice, 1..=5);

    assert_eq!(prune(test.map(), u64::MAX, 0, 0).await, 0);

    let (rows, _) = page(test.map(), alice, None, 10, false).await;
    assert_eq!(rows.len(), 5);
}

#[test]
fn gateway_refusals_are_not_retried_but_outages_are() {
    let refused = |status| Error::Request(ErrorKind::Unknown, "refused".into(), status);

    assert!(is_permanent_failure(&refused(StatusCode::BAD_REQUEST)));
    assert!(is_permanent_failure(&refused(StatusCode::NOT_FOUND)));
    assert!(is_permanent_failure(&err!(Request(InvalidParam(
        "bad pusher URL"
    )))));

    assert!(!is_permanent_failure(&refused(
        StatusCode::TOO_MANY_REQUESTS
    )));
    assert!(!is_permanent_failure(&err!(BadServerResponse(
        "gateway returned 502"
    ))));
}

#[test]
fn only_a_true_flag_disables_the_badge() {
    let with = |data: serde_json::Value| {
        let mut http = HttpPusherData::new("https://push.example/_matrix/push/v1/notify".into());
        http.data = serde_json::from_value(data).expect("an object");
        badge_count_disabled(&http)
    };

    assert!(with(serde_json::json!({ "disable_badge_count": true })));
    assert!(with(
        serde_json::json!({ "org.matrix.msc4076.disable_badge_count": true })
    ));
    assert!(!with(serde_json::json!({ "disable_badge_count": false })));
    assert!(!with(serde_json::json!({})));
}
