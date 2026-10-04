use std::collections::BTreeMap;

use futures::{StreamExt, future::ready};
use phantom_core::{Result, extract_variant};
use phantom_service::accounts::account_data::AnyRawAccountDataEvent;
use ruma::{
    OwnedRoomId, api::client::sync::sync_events::v5::response::AccountData,
    events::AnyRoomAccountDataEvent, serde::Raw,
};

use super::{Connection, Results, SyncInfo, Window, selector};
use crate::client::account_data::is_empty_account_data_event;

#[tracing::instrument(name = "account_data", level = "trace", skip_all)]
pub(super) async fn collect(
    SyncInfo {
        services,
        sender_user,
        ..
    }: SyncInfo<'_>,
    conn: &Connection,
) -> Result<AccountData> {
    let globalsince = conn.globalsince;
    let mut account_data = AccountData::default();
    account_data.global = services
        .account_data
        .changes_since(None, sender_user, globalsince, Some(conn.next_batch))
        .filter_map(|event| ready(extract_variant!(event, AnyRawAccountDataEvent::Global)))
        .filter(move |event| ready(globalsince != 0 || !is_empty_account_data_event(event)))
        .collect()
        .await;

    Ok(account_data)
}

pub(super) fn collect_ranges(
    conn: &Connection,
    window: &Window,
    ranges: &mut Results,
) -> BTreeMap<OwnedRoomId, Vec<Raw<AnyRoomAccountDataEvent>>> {
    let implicit = conn
        .extensions
        .account_data
        .lists
        .as_deref()
        .map(<[_]>::iter);

    let explicit = conn
        .extensions
        .account_data
        .rooms
        .as_deref()
        .map(<[_]>::iter);

    selector(conn, window, implicit, explicit)
        .filter_map(|room_id| {
            ranges
                .take_account_data(room_id)
                .map(|events| (room_id.to_owned(), events))
        })
        .collect()
}
