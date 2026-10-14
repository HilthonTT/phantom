//! The admin audit log: recording every change made through the admin API or
//! an `!admin` command, and reading the record back.
//!
//! Recording happens in two places and nowhere else. The admin API's routes
//! all sit under [`layer`], which records each request that is not a read;
//! the `!admin` processor calls [`command`] for each command that is not one
//! of its listings. Handlers never record anything themselves, so a route or
//! command added later is covered without anyone remembering to.

use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{
        MatchedPath, Query, RawPathParams, Request, State, rejection::RawPathParamsRejection,
    },
    middleware::Next,
    response::{IntoResponse, Response},
};
use http::Method;
use phantom_core::{Result, warn};
use phantom_service::Services;
use ruma::UserId;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::router::{AdminAuth, AdminWitness, State as RouterState};

/// The prefix every admin route shares, left out of the recorded action.
const PREFIX: &str = "/_phantom/admin/v1";

/// How much of a failed action's answer is read for its error message.
const ERROR_BODY_LIMIT: usize = 64 * 1024;

/// The middleware every admin route runs under. A read passes straight
/// through; anything else is recorded once it has answered, provided it
/// authenticated as an admin, with the error it failed with if it did.
pub(super) async fn layer(
    params: Result<RawPathParams, RawPathParamsRejection>,
    mut request: Request,
    next: Next,
) -> Response {
    if matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        return next.run(request).await;
    }

    let route = request.extensions().get::<MatchedPath>().map_or_else(
        || request.uri().path().to_owned(),
        |path| path.as_str().to_owned(),
    );
    let action = format!(
        "{} {}",
        request.method(),
        route.strip_prefix(PREFIX).unwrap_or(&route)
    );
    let target = params
        .map(|params| {
            params
                .iter()
                .map(|(_, value)| value)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();

    let witness = AdminWitness::default();
    request.extensions_mut().insert(witness.clone());

    let response = next.run(request).await;

    let Some((services, admin)) = witness.seen() else {
        return response;
    };

    let status = response.status();
    let (response, result) = match status.is_success() {
        true => (response, status.to_string()),
        false => {
            let (parts, body) = response.into_parts();
            let bytes = to_bytes(body, ERROR_BODY_LIMIT).await.unwrap_or_default();
            let error = error_message(&bytes);

            (
                Response::from_parts(parts, Body::from(bytes)),
                format!("{status}: {error}"),
            )
        }
    };

    if let Err(e) = services.audit.record(
        admin.as_str(),
        "api",
        &action,
        &target,
        status.is_success(),
        &result,
    ) {
        warn!("Could not record {action} by {admin} in the audit log: {e}");
    }

    response
}

/// The `error` of a Matrix error body, or the body itself when it has none.
fn error_message(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|body| body["error"].as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned())
}

/// The subcommands that only read, which are left out of the log.
const READS: [&str; 8] = [
    "list", "info", "devices", "status", "services", "config", "logs", "tasks",
];

/// Records an `!admin` command, given as clap parsed it, unless it only
/// read; error is why it failed, if it did.
pub(super) fn command(services: &Services, admin: &UserId, argv: &[String], error: Option<&str>) {
    let Some((action, target)) = command_action(argv) else {
        return;
    };

    let result = error.unwrap_or("done");
    if let Err(e) = services.audit.record(
        admin.as_str(),
        "command",
        &action,
        &target,
        error.is_none(),
        result,
    ) {
        warn!("Could not record {action} by {admin} in the audit log: {e}");
    }
}

/// What a command did and to what: its subcommand, as `!admin users
/// deactivate`, and its first argument. Only the first is kept, so a password
/// given after the account it is for never reaches the log.
fn command_action(argv: &[String]) -> Option<(String, String)> {
    let mut words = argv.iter().skip(1);
    let group = words.next()?;
    let sub = words.next().map(String::as_str).unwrap_or_default();

    if READS.contains(&sub) {
        return None;
    }

    let target = words
        .find(|word| !word.starts_with('-'))
        .cloned()
        .unwrap_or_default();

    Some((
        format!("!admin {group} {sub}").trim_end().to_owned(),
        target,
    ))
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct AuditQuery {
    limit: Option<usize>,

    /// The `next` of the page before, to read on from.
    before: Option<String>,
}

/// # `GET /_phantom/admin/v1/audit?limit=&before=`
///
/// The audit log, newest first, a page at a time: `next`, when set, asks for
/// the page after this one.
pub(super) async fn audit(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Query(query): Query<AuditQuery>,
) -> Result<impl IntoResponse> {
    let limit = query.limit.unwrap_or(100).clamp(1, 1000);
    let records = services.audit.page(query.before.as_deref(), limit).await;

    let next = records
        .last()
        .filter(|_| records.len() == limit)
        .map(|record| record.id.clone());

    Ok(Json(json!({ "records": records, "next": next })))
}

#[cfg(test)]
mod tests {
    use super::{command_action, error_message};

    fn argv(line: &str) -> Vec<String> {
        line.split_whitespace().map(ToOwned::to_owned).collect()
    }

    #[test]
    fn reads_are_not_recorded() {
        for line in [
            "!admin users list",
            "!admin users info bob",
            "!admin users devices bob",
            "!admin server logs --limit 5",
            "!admin server tasks",
        ] {
            assert_eq!(command_action(&argv(line)), None, "{line}");
        }
    }

    #[test]
    fn changes_are_recorded_with_their_first_argument() {
        assert_eq!(
            command_action(&argv("!admin users deactivate bob --erase")),
            Some(("!admin users deactivate".to_owned(), "bob".to_owned()))
        );
        assert_eq!(
            command_action(&argv("!admin rooms delete --force #lobby:example.com")),
            Some((
                "!admin rooms delete".to_owned(),
                "#lobby:example.com".to_owned()
            ))
        );
        assert_eq!(
            command_action(&argv("!admin server backup")),
            Some(("!admin server backup".to_owned(), String::new()))
        );
    }

    #[test]
    fn a_password_never_reaches_the_log() {
        let (_, target) = command_action(&argv("!admin users reset-password bob hunter2")).unwrap();

        assert_eq!(target, "bob");
    }

    #[test]
    fn the_error_of_a_matrix_error_body_is_kept() {
        assert_eq!(
            error_message(br#"{"errcode":"M_FORBIDDEN","error":"No."}"#),
            "No."
        );
        assert_eq!(error_message(b"plain"), "plain");
    }
}
