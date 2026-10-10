mod report_event;
mod report_room;
mod report_user;

pub(crate) use self::{
    report_event::report_event_route, report_room::report_room_route,
    report_user::report_user_route,
};

use phantom_service::Services;
use ruma::{EventId, RoomId, UserId};

pub(super) const REASON_MAX_LEN: usize = 2000;

/// What a report is about: an event in a room, a room, or a user.
struct Reported<'a> {
    room_id: Option<&'a RoomId>,
    event_id: Option<&'a EventId>,
    user_id: Option<&'a UserId>,
}

/// Hands a user's abuse report to the server's administrators: it is stored
/// for the admin API and posted into the admin room as notice. A report that
/// cannot be stored is logged instead, so it is never lost silently.
async fn send_report(
    services: &Services,
    reporter: &UserId,
    reported: Reported<'_>,
    reason: &str,
    notice: &str,
) {
    let filed = services
        .reports
        .file(
            reporter.to_owned(),
            reported.room_id.map(ToOwned::to_owned),
            reported.event_id.map(ToOwned::to_owned),
            reported.user_id.map(ToOwned::to_owned),
            reason.to_owned(),
            notice,
        )
        .await;

    if let Err(e) = filed {
        phantom_core::warn!(target: "report", "Could not store a report ({e}): {notice}");
    }
}
