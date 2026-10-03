mod report_event;
mod report_room;
mod report_user;

pub(crate) use self::{
    report_event::report_event_route, report_room::report_room_route,
    report_user::report_user_route,
};

pub(super) const REASON_MAX_LEN: usize = 2000;

/// Hands a user's abuse report to the server's administrators.
///
/// phantom has no admin room to post into yet, so the report is logged.
async fn send_report(report: &str) {
    phantom_core::warn!(target: "report", "{report}");
}
