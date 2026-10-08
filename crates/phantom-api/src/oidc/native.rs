mod credentials;
mod pages;

use std::net::IpAddr;

use axum::{
    extract::{Form, Request, State},
    response::{Redirect, Response},
};
use http::StatusCode;
use phantom_core::{Err, Error, Result, err, rand};
use phantom_service::Services;
use serde::Deserialize;
use smallstr::SmallString;
use url::Url;

use self::{
    credentials::{authenticate_local, verify_credentials},
    pages::render_page,
};
use super::{
    account::{account_error_response, account_html_response, account_redirect_response},
    authorization_sso_url, query_error,
};
use crate::router::ClientIp;

type AccountAction = SmallString<[u8; 32]>;
type DeviceId = SmallString<[u8; 24]>;
type IdpId = SmallString<[u8; 32]>;

const LOGIN_TOKEN_LENGTH: usize = 32;

#[derive(Debug, Default, Deserialize)]
struct NativeQuery {
    oidc_req_id: Option<String>,
    idp_id: Option<IdpId>,
    user_code: Option<String>,
    action: Option<AccountAction>,
    device_id: Option<DeviceId>,
    view: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NativeSubmit {
    #[serde(default)]
    oidc_req_id: Option<String>,
    #[serde(default)]
    user_code: Option<String>,
    #[serde(default)]
    action: Option<AccountAction>,
    #[serde(default)]
    device_id: Option<DeviceId>,
    #[serde(default)]
    mode: Option<String>,
    username: String,
    password: String,
    #[serde(default)]
    registration_token: Option<String>,
}

#[derive(Clone, Copy)]
enum Flow<'a> {
    Account { action: &'a str, device_id: &'a str },
    Authorization(&'a str),
    Device(&'a str),
}

/// Renders the native login or registration page for a pending authorization,
/// device, or account flow.
///
/// A provider chosen on that page arrives here as `idp_id`, and the route
/// redirects to it instead. When the request is already bound to a provider,
/// the page offers only that provider.
pub(crate) async fn native_get_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    request: Request,
) -> Response {
    if let Err(e) = require_native(&services) {
        return account_error_response(&e);
    }

    let params: NativeQuery =
        match serde_html_form::from_str(request.uri().query().unwrap_or_default()) {
            Ok(params) => params,
            Err(e) => return account_error_response(&query_error(&e)),
        };

    let context = match parse_flow(
        params.oidc_req_id.as_deref(),
        params.user_code.as_deref(),
        params.action.as_deref(),
        params.device_id.as_deref(),
    ) {
        Ok(context) => context,
        Err(e) => return account_error_response(&e),
    };

    if let Some(idp_id) = params.idp_id.as_deref() {
        return provider_redirect(&services, client, context, idp_id)
            .await
            .map_or_else(|e| account_error_response(&e), account_redirect_response);
    }

    let view = params.view.as_deref().unwrap_or("login");

    render_page(&services, view, context, None)
        .await
        .map(|html| account_html_response(StatusCode::OK, html))
        .unwrap_or_else(|e| account_error_response(&e))
}

fn parse_flow<'a>(
    oidc_req_id: Option<&'a str>,
    user_code: Option<&'a str>,
    action: Option<&'a str>,
    device_id: Option<&'a str>,
) -> Result<Flow<'a>> {
    match (
        oidc_req_id.filter(|value| !value.is_empty()),
        user_code.filter(|value| !value.is_empty()),
        action.filter(|value| !value.is_empty()),
    ) {
        (Some(req_id), None, None) => Ok(Flow::Authorization(req_id)),
        (None, Some(user_code), None) => Ok(Flow::Device(user_code)),
        (None, None, Some(action)) => Ok(Flow::Account {
            action,
            device_id: device_id.unwrap_or_default(),
        }),
        _ => Err!(Request(InvalidParam(
            "Exactly one OIDC request ID, user code, or account action is required."
        ))),
    }
}

/// Send the browser to the provider chosen on the login page.
///
/// The pending request is bound to that provider only once its URL is built, and
/// the binding is final, so the request cannot also complete with a local
/// password or another provider.
async fn provider_redirect(
    services: &Services,
    client: IpAddr,
    context: Flow<'_>,
    idp_id: &str,
) -> Result<Redirect> {
    let Flow::Authorization(req_id) = context else {
        return Err!(Request(InvalidParam(
            "Provider selection requires an authorization request"
        )));
    };

    services.oauth.check_rate_limit(client)?;

    let oidc = services.oauth.get_server()?;
    let provider_id = services
        .oauth
        .providers
        .find_config(idp_id)
        .map_err(|_| err!(Request(InvalidParam("Unrecognized identity provider"))))?
        .id();

    let sso_url = authorization_sso_url(&oidc.issuer_url()?, provider_id, req_id)?;

    oidc.bind_auth_request_to_provider(req_id, provider_id)
        .await?;

    Ok(Redirect::temporary(sso_url.as_str()))
}

/// Authenticates submitted credentials and sends the login token to the
/// authorization completion, device-consent, or account-management callback.
pub(crate) async fn native_submit_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    Form(body): Form<NativeSubmit>,
) -> Response {
    match native_submit(&services, client, &body).await {
        Ok(response) => response,
        Err(e) => render_submit_error(&services, &body, e).await,
    }
}

async fn native_submit(
    services: &Services,
    client: IpAddr,
    body: &NativeSubmit,
) -> Result<Response> {
    require_native(services)?;
    // Always-on anti-brute-force floor; the oidc_rc_* throttle below is opt-in.
    services.oauth.check_device_rate_limit(client)?;
    services.oauth.check_rate_limit(client)?;

    let context = parse_flow(
        body.oidc_req_id.as_deref(),
        body.user_code.as_deref(),
        body.action.as_deref(),
        body.device_id.as_deref(),
    )?;

    let user_id = match context {
        Flow::Authorization(req_id) => authenticate_local(services, req_id, body).await?,
        _ => verify_credentials(services, &body.username, &body.password).await?,
    };

    let token = rand::string(LOGIN_TOKEN_LENGTH);
    let _expires_in = services.users.create_login_token(&user_id, &token);

    let redirect = complete_redirect(services, context, &token)?;

    Ok(account_redirect_response(redirect))
}

/// Re-render the page a failed submission came from, carrying its error.
///
/// A submission whose flow cannot be parsed, or whose request has gone, gets
/// the error page instead.
async fn render_submit_error(services: &Services, body: &NativeSubmit, error: Error) -> Response {
    let context = match parse_flow(
        body.oidc_req_id.as_deref(),
        body.user_code.as_deref(),
        body.action.as_deref(),
        body.device_id.as_deref(),
    ) {
        Ok(context) => context,
        Err(e) => return account_error_response(&e),
    };

    let view = match (context, body.mode.as_deref()) {
        (Flow::Authorization(_), Some("register")) => "register",
        _ => "login",
    };

    // Read the error out before awaiting, since `Error` is not `Sync`.
    let msg = error.sanitized_message();
    let status = error.status_code();

    render_page(services, view, context, Some(&msg))
        .await
        .map(|html| account_html_response(status, html))
        .unwrap_or_else(|e| account_error_response(&e))
}

/// Redirects with 303 so the browser cannot replay the password form into the
/// completion or callback route.
fn complete_redirect(services: &Services, flow: Flow<'_>, login_token: &str) -> Result<Redirect> {
    let issuer = services.oauth.get_server()?.issuer_url()?;
    let base = issuer.trim_end_matches('/');

    let url = match flow {
        Flow::Device(user_code) => Url::parse_with_params(
            &format!("{base}/_phantom/oidc/device_callback"),
            [("user_code", user_code), ("loginToken", login_token)],
        ),
        Flow::Authorization(req_id) => Url::parse_with_params(
            &format!("{base}/_phantom/oidc/_complete"),
            [("oidc_req_id", req_id), ("loginToken", login_token)],
        ),
        Flow::Account { action, device_id } => Url::parse_with_params(
            &format!("{base}/_phantom/oidc/account_callback"),
            [
                ("action", action),
                ("device_id", device_id),
                ("loginToken", login_token),
            ],
        ),
    }
    .map_err(|_| err!(error!("Failed to build completion URL")))?;

    Ok(Redirect::to(url.as_str()))
}

fn require_native(services: &Services) -> Result {
    services.oauth.get_server()?;

    services
        .config
        .oidc
        .oidc_native_auth
        .then_some(())
        .ok_or_else(|| err!(Request(NotFound("Native authentication is not enabled"))))
}

#[cfg(test)]
mod tests {
    use super::{Flow, parse_flow};

    #[test]
    fn flow_requires_exactly_one_nonempty_value() {
        assert!(matches!(
            parse_flow(Some("REQ123"), None, None, None),
            Ok(Flow::Authorization("REQ123"))
        ));

        assert!(matches!(
            parse_flow(None, Some("BCDF-GHJK"), None, None),
            Ok(Flow::Device("BCDF-GHJK"))
        ));

        assert!(matches!(
            parse_flow(None, None, Some("org.matrix.sessions_list"), None),
            Ok(Flow::Account {
                action: "org.matrix.sessions_list",
                device_id: "",
            })
        ));

        assert!(matches!(
            parse_flow(None, None, Some("org.matrix.session_view"), Some("DEVICE")),
            Ok(Flow::Account {
                action: "org.matrix.session_view",
                device_id: "DEVICE",
            })
        ));

        assert!(matches!(
            parse_flow(None, None, Some("org.matrix.sessions_list"), Some("")),
            Ok(Flow::Account {
                action: "org.matrix.sessions_list",
                device_id: "",
            })
        ));

        assert!(parse_flow(None, None, None, None).is_err());
        assert!(parse_flow(None, None, None, Some("DEVICE")).is_err());
        assert!(parse_flow(Some(""), None, None, None).is_err());
        assert!(parse_flow(None, Some(""), None, None).is_err());
        assert!(parse_flow(None, None, Some(""), None).is_err());
        assert!(parse_flow(None, None, Some(""), Some("DEVICE")).is_err());
        assert!(parse_flow(Some("REQ123"), Some("BCDF-GHJK"), None, None).is_err());
        assert!(parse_flow(Some("REQ123"), None, Some("org.matrix.sessions_list"), None).is_err());

        assert!(
            parse_flow(
                None,
                Some("BCDF-GHJK"),
                Some("org.matrix.sessions_list"),
                None
            )
            .is_err()
        );

        assert!(
            parse_flow(
                Some("REQ123"),
                Some("BCDF-GHJK"),
                Some("org.matrix.sessions_list"),
                None,
            )
            .is_err()
        );
    }
}
