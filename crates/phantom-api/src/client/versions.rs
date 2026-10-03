use std::iter::once;

use axum::extract::State;
use phantom_core::{Result, diagnostics::info};
use ruma::api::client::discovery::get_supported_versions::{self, Server};

use crate::router::Ruma;

/// # `GET /_matrix/client/versions`
///
/// Get the versions of the specification and unstable features supported by
/// this server.
///
/// - Versions take the form MAJOR.MINOR.PATCH
/// - Only the latest PATCH release will be reported for each MAJOR.MINOR value
/// - Unstable features are namespaced and may include version information in
///   their name
pub(crate) async fn get_supported_versions_route(
    State(services): State<crate::router::State>,
    _body: Ruma<get_supported_versions::Request>,
) -> Result<get_supported_versions::Response> {
    // MSC4383: client-side parity with /_matrix/federation/v1/version.
    let server = Server::new(info::name().into(), info::version().into());

    let mut response =
        get_supported_versions::Response::new(VERSIONS.into_iter().map(Into::into).collect());

    response.unstable_features = UNSTABLE_FEATURES
        .into_iter()
        .chain(
            services
                .config
                .rendezvous
                .rendezvous_enabled
                .then_some("org.matrix.msc4108"),
        )
        .map(Into::into)
        .zip(once(true).cycle())
        .collect();

    response.server = Some(server);

    Ok(response)
}

static VERSIONS: [&str; 26] = [
    "r0.0.1", /* Historical */
    "r0.1.0", /* Historical */
    "r0.2.0", /* Historical */
    "r0.3.0", /* Historical */
    "r0.4.0", /* Historical */
    "r0.5.0", /* Historical */
    "r0.6.0", /* Historical */
    "r0.6.1", /* Historical */
    "v1.1",   /* Stable */
    "v1.2",   /* Stable */
    "v1.3",   /* Stable */
    "v1.4",   /* private read receipts, threads */
    "v1.5",   /* Stable */
    "v1.6",   /* jump to date */
    "v1.7",   /* intentional mentions */
    "v1.8",   /* no action */
    "v1.9",   /* no action */
    "v1.10",  /* relations recursion */
    "v1.11",  /* authenticated media */
    "v1.12",  /* no action */
    "v1.13",  /* no action */
    "v1.14",  /* no action */
    "v1.15",  /* OIDC auth metadata */
    "v1.16",  /* extended profiles (MSC4133) */
    "v1.17",  /* no action */
    "v1.18",  /* policy servers (MSC4284) */
];

static UNSTABLE_FEATURES: [&str; 24] = [
    "org.matrix.e2e_cross_signing",
    // private read receipts (https://github.com/matrix-org/matrix-spec-proposals/pull/2285)
    "org.matrix.msc2285.stable",
    // appservice ping (https://github.com/matrix-org/matrix-spec-proposals/pull/2659)
    "fi.mau.msc2659.stable",
    // threading/threads (https://github.com/matrix-org/matrix-spec-proposals/pull/2836)
    "org.matrix.msc2836",
    // jump to date (https://github.com/matrix-org/matrix-spec-proposals/pull/3030)
    "org.matrix.msc3030",
    // spaces/hierarchy summaries (https://github.com/matrix-org/matrix-spec-proposals/pull/2946)
    "org.matrix.msc2946",
    // sliding sync (https://github.com/matrix-org/matrix-spec-proposals/pull/3575/files#r1588877046)
    "org.matrix.msc3575",
    // filtering of /publicRooms by room type (https://github.com/matrix-org/matrix-spec-proposals/pull/3827)
    "org.matrix.msc3827",
    "org.matrix.msc3827.stable",
    // authenticated media (https://github.com/matrix-org/matrix-spec-proposals/pull/3916)
    "org.matrix.msc3916.stable",
    // intentional mentions (https://github.com/matrix-org/matrix-spec-proposals/pull/3952)
    "org.matrix.msc3952_intentional_mentions",
    // MatrixRTC transport discovery (https://github.com/matrix-org/matrix-spec-proposals/pull/4143)
    "org.matrix.msc4143",
    // stable flag for 3916 (https://github.com/matrix-org/matrix-spec-proposals/pull/4180)
    "org.matrix.msc4180",
    // Simplified Sliding sync (https://github.com/matrix-org/matrix-spec-proposals/pull/4186)
    "org.matrix.simplified_msc3575",
    // Sliding sync profiles extension (https://github.com/matrix-org/matrix-spec-proposals/pull/4262)
    "org.matrix.msc4262",
    // OIDC-native auth umbrella (https://github.com/matrix-org/matrix-spec-proposals/pull/3861)
    "org.matrix.msc3861",
    // OIDC-native auth: authorization code grant (https://github.com/matrix-org/matrix-spec-proposals/pull/2964)
    "org.matrix.msc2964",
    // OIDC-native auth: auth issuer discovery (https://github.com/matrix-org/matrix-spec-proposals/pull/2965)
    "org.matrix.msc2965",
    // OIDC-native auth: dynamic client registration (https://github.com/matrix-org/matrix-spec-proposals/pull/2966)
    "org.matrix.msc2966",
    // OIDC-native auth: API scopes (https://github.com/matrix-org/matrix-spec-proposals/pull/2967)
    "org.matrix.msc2967",
    // Backwards-compatible redaction sending via /send (https://github.com/matrix-org/matrix-spec-proposals/pull/4169)
    "com.beeper.msc4169",
    // Client-server discovery of server version (https://github.com/matrix-org/matrix-spec-proposals/pull/4383)
    "net.zemos.msc4383",
    // Read receipts for threads (https://github.com/matrix-org/matrix-spec-proposals/pull/3771)
    "org.matrix.msc3771",
    // Threading via m.thread relations, stable since Matrix 1.4 (https://github.com/matrix-org/matrix-spec-proposals/pull/3440)
    "org.matrix.msc3440.stable",
];
