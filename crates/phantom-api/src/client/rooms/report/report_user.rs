use axum::extract::State;
use phantom_core::{Err, Result, info};
use ruma::api::client::reporting::report_user;

use super::{REASON_MAX_LEN, Reported, send_report};
use crate::router::{ClientIp, Ruma};

/// # `POST /_matrix/client/v3/users/{userId}/report`
///
/// Reports an inappropriate user to homeserver admins (MSC4260).
#[tracing::instrument(skip_all, fields(%client), name = "report_user")]
pub(crate) async fn report_user_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<report_user::v3::Request>,
) -> Result<report_user::v3::Response> {
    let sender_user = body.sender_user();
    let target_user = &body.user_id;
    let reason = &body.reason;

    if reason.len() > REASON_MAX_LEN {
        return Err!(Request(InvalidParam(
            "Reason too long, should be {REASON_MAX_LEN} characters or fewer"
        )));
    }

    info!("Received user report by {sender_user} for {target_user} with reason: \"{reason}\"");

    // Succeed regardless of user existence to deter enumeration (MSC4277).
    if services.users.is_active_local(target_user).await {
        send_report(
            &services,
            sender_user,
            Reported {
                room_id: None,
                event_id: None,
                user_id: Some(target_user),
            },
            reason,
            &format!(
                "@room User report received from {sender_user}\nReport Reason: \
                 {reason}\n\nReported User ID: {target_user}",
            ),
        )
        .await;
    }

    Ok(report_user::v3::Response::new())
}
