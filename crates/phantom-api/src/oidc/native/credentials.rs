use phantom_core::{Err, Result, err, hash::verify_password, info};
use phantom_service::Services;
use ruma::{OwnedUserId, UserId};

use super::NativeSubmit;

/// Authenticate through the local branch of an authorization request.
///
/// A request bound to a provider is refused before any credential is checked.
/// A login claims the request once the password verifies, and registration
/// claims it before creating the account.
pub(super) async fn authenticate_local(
    services: &Services,
    req_id: &str,
    body: &NativeSubmit,
) -> Result<OwnedUserId> {
    let oidc = services.oauth.get_server()?;

    oidc.check_local_auth_request(req_id).await?;

    if body.mode.as_deref() == Some("register") {
        return do_register(services, req_id, body).await;
    }

    let user_id = verify_credentials(services, &body.username, &body.password).await?;

    oidc.bind_auth_request_to_local(req_id).await?;

    Ok(user_id)
}

/// Authenticate a local account by password, mirroring the `/login` password
/// flow (`password_login`): password-origin accounts only, uniform error.
pub(super) async fn verify_credentials(
    services: &Services,
    username: &str,
    password: &str,
) -> Result<OwnedUserId> {
    let invalid = || err!(Request(Forbidden("Invalid username or password.")));
    let server_name = services.server_state.server_name();

    let user_id = UserId::parse_with_server_name(username, server_name).map_err(|_| invalid())?;

    if !services.server_state.user_is_local(&user_id) {
        return Err(invalid());
    }

    // A per-account floor, so guesses spread over many addresses are still
    // throttled.
    services.oauth.check_password_rate_limit(&user_id)?;

    // Native registration lowercases the localpart, so resolve to whichever case
    // carries the password.
    let (user_id, hash) = match services.users.password_hash(&user_id).await {
        Ok(hash) => (user_id, hash),
        Err(_) => {
            let lowercased = UserId::parse_with_server_name(username.to_lowercase(), server_name)
                .map_err(|_| invalid())?;

            let hash = services
                .users
                .password_hash(&lowercased)
                .await
                .map_err(|_| invalid())?;

            (lowercased, hash)
        }
    };

    // Deactivated accounts have no password here to check.
    if hash.is_empty() {
        return Err(invalid());
    }

    verify_password(password, &hash).map_err(|_| invalid())?;

    Ok(user_id)
}

async fn do_register(
    services: &Services,
    req_id: &str,
    body: &NativeSubmit,
) -> Result<OwnedUserId> {
    if !services.config.auth.allow_registration {
        return Err!(Request(Forbidden(
            "Registration is disabled on this server."
        )));
    }

    let username = body.username.trim().to_lowercase();
    if username.is_empty() {
        return Err!(Request(InvalidUsername("A username is required.")));
    }

    if body.password.is_empty() {
        return Err!(Request(InvalidParam("A password is required.")));
    }

    // This page cannot collect a 3PID, so refuse rather than silently bypass a
    // mandatory-email policy.
    let token_required = services.registration_tokens.is_enabled().await;
    let smtp = &services.config.smtp;
    let email_required = smtp.connection_uri.is_some()
        && (smtp.require_email_for_registration
            || (token_required && smtp.require_email_for_token_registration));

    if email_required {
        return Err!(Request(Forbidden(
            "This server requires an email to register, which this page cannot collect."
        )));
    }

    if services
        .config
        .rooms
        .forbidden_usernames
        .is_match(&username)
    {
        return Err!(Request(Forbidden("That username is not allowed.")));
    }

    let user_id = UserId::parse_with_server_name(&username, services.server_state.server_name())
        .map_err(|_| err!(Request(InvalidUsername("That username is not valid."))))?;

    user_id.validate_strict().map_err(|_| {
        err!(Request(InvalidUsername(
            "That username contains disallowed characters."
        )))
    })?;

    if services.appservice.is_exclusive_user_id(&user_id).await {
        return Err!(Request(Exclusive(
            "That username is reserved by an appservice."
        )));
    }

    if services.users.exists(&user_id).await {
        return Err!(Request(UserInUse("That username is taken.")));
    }

    let token = body.registration_token.as_deref().unwrap_or_default();

    // Validate before claiming, so a mistyped token leaves provider choice open.
    if token_required {
        services.registration_tokens.is_token_valid(token).await?;
    }

    // Claim this branch before consuming the token or creating the account.
    services
        .oauth
        .get_server()?
        .bind_auth_request_to_local(req_id)
        .await?;

    if token_required {
        services.registration_tokens.try_consume(token).await?;
    }

    register_local_user(services, &user_id, &body.password)?;

    Ok(user_id)
}

/// Create a local account with a password, named after its localpart plus
/// the configured suffix.
fn register_local_user(services: &Services, user_id: &UserId, password: &str) -> Result {
    services.users.create(user_id, Some(password))?;

    let suffix = services.config.auth.new_user_displayname_suffix.as_str();
    let displayname = match suffix {
        "" => user_id.localpart().to_owned(),
        suffix => format!("{} {suffix}", user_id.localpart()),
    };

    services.profile.set_displayname(user_id, Some(displayname));

    info!("New user {user_id} registered through native OIDC sign-up");

    Ok(())
}
