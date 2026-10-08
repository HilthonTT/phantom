use std::{fmt::Write, sync::LazyLock};

use phantom_core::{Result, html::escape as html_escape, runtime::config::IdentityProvider};
use phantom_service::Services;

use super::{
    super::{account::ACCOUNT_HEAD, url_encode},
    Flow,
};

type ProviderChoice<'a> = (&'a str, &'a str);

/// Render the page for a flow, reading an authorization request's binding.
///
/// A request bound to a provider offers only that provider, whatever view was
/// asked for. An unknown or expired request is an error rather than a page.
pub(super) async fn render_page(
    services: &Services,
    view: &str,
    context: Flow<'_>,
    error: Option<&str>,
) -> Result<String> {
    let registration_enabled = services.config.auth.allow_registration;
    let Flow::Authorization(req_id) = context else {
        return Ok(render_login(context, error, registration_enabled, ""));
    };

    let bound = services
        .oauth
        .get_server()?
        .peek_auth_request(req_id)
        .await?
        .idp_id;

    let page = match bound.as_deref() {
        None if view == "register" && registration_enabled => {
            render_register(services, req_id, error).await
        }

        Some(idp_id) => {
            let provider = services.oauth.providers.find_config(idp_id)?;

            render_bound(req_id, provider_choice(provider), error)
        }

        None => {
            let sso_options = render_sso_options("Or sign in with", req_id, sso_choices(services));

            render_login(context, error, registration_enabled, &sso_options)
        }
    };

    Ok(page)
}

fn render_login(
    context: Flow<'_>,
    error: Option<&str>,
    show_register: bool,
    sso_options: &str,
) -> String {
    let (context_fields, register_link) = match context {
        Flow::Device(user_code) => {
            let context_fields = format!(
                r#"<input type="hidden" name="user_code" value="{}">"#,
                html_escape(user_code),
            );

            (context_fields, String::new())
        }
        Flow::Account { action, device_id } => {
            let context_fields = format!(
                concat!(
                    r#"<input type="hidden" name="action" value="{}">"#,
                    "\n\t\t\t",
                    r#"<input type="hidden" name="device_id" value="{}">"#,
                ),
                html_escape(action),
                html_escape(device_id),
            );

            (context_fields, String::new())
        }
        Flow::Authorization(req_id) => {
            let context_fields = format!(
                r#"<input type="hidden" name="oidc_req_id" value="{}">"#,
                html_escape(req_id),
            );

            let register_link = if show_register {
                format!(
                    r#"<p class="auth-nav">New to this server? <a href="/_phantom/oidc/native?oidc_req_id={}&amp;view=register">Create an account</a></p>"#,
                    url_encode(req_id),
                )
            } else {
                String::new()
            };

            (context_fields, register_link)
        }
    };

    LOGIN_HTML
        .replace("{register_link}", &register_link)
        .replace("{sso_options}", sso_options)
        .replace("{error}", &error_block(error))
        // Fill caller-supplied fields last so they cannot smuggle a placeholder.
        .replace("{context_fields}", &context_fields)
}

async fn render_register(services: &Services, req_id: &str, error: Option<&str>) -> String {
    let token_field = if services.registration_tokens.is_enabled().await {
        TOKEN_FIELD
    } else {
        ""
    };

    REGISTER_HTML
        .replace("{token_field}", token_field)
        .replace("{req_id_enc}", &url_encode(req_id))
        .replace("{error}", &error_block(error))
        // Fill the caller-supplied {req_id} last so it cannot smuggle a placeholder.
        .replace("{req_id}", &html_escape(req_id))
}

fn provider_choice(provider: &IdentityProvider) -> ProviderChoice<'_> {
    (provider.id(), provider.display_name())
}

/// Offer only the provider a pending request is bound to.
///
/// A user who leaves that provider before finishing returns here, and the
/// request can complete only through it.
fn render_bound(req_id: &str, provider: ProviderChoice<'_>, error: Option<&str>) -> String {
    let sso_options = render_sso_options("Continue with", req_id, [provider]);

    BOUND_HTML
        .replace("{sso_options}", &sso_options)
        .replace("{error}", &error_block(error))
}

/// Every configured provider, as the login page offers them.
fn sso_choices(services: &Services) -> impl Iterator<Item = ProviderChoice<'_>> {
    services
        .config
        .identity_provider
        .values()
        .map(provider_choice)
}

/// List each provider as a link that binds the pending request to it.
///
/// Names are HTML-escaped with braces encoded too, since the result is filled in
/// before the error and context placeholders.
fn render_sso_options<'a, I>(heading: &str, req_id: &str, providers: I) -> String
where
    I: IntoIterator<Item = ProviderChoice<'a>>,
{
    let req_id = url_encode(req_id);
    let options = providers
        .into_iter()
        .map(|(id, name)| {
            let name = html_escape(name)
                .replace('{', "&#123;")
                .replace('}', "&#125;");

            (url_encode(id), name)
        })
        .fold(String::new(), |mut out, (id, name)| {
            write!(
                out,
                r#"<li><a href="/_phantom/oidc/native?oidc_req_id={req_id}&amp;idp_id={id}">{name}</a></li>"#,
            )
            .ok();

            out
        });

    if options.is_empty() {
        return String::new();
    }

    format!(r#"<section class="sso-options"><h2>{heading}</h2><ul>{options}</ul></section>"#)
}

fn error_block(error: Option<&str>) -> String {
    error
        .map(|msg| format!(r#"<p class="err">{}</p>"#, html_escape(msg)))
        .unwrap_or_default()
}

static LOGIN_HTML: LazyLock<String> = LazyLock::new(|| {
    format!(
        r#"
<!DOCTYPE html>
<html lang="en">
    <head>
        {ACCOUNT_HEAD}
        <title>Sign in · Phantom</title>
    </head>
    <body class="auth-page">
        <main class="auth-card" aria-labelledby="auth-title">
            <h1 id="auth-title">Sign in</h1>
            <p class="auth-description">Sign in to your Phantom account.</p>
            {{error}}
            <form class="auth-form" method="POST" action="/_phantom/oidc/native">
                {{context_fields}}
                <input type="hidden" name="mode" value="login">
                <label for="auth-username">Username</label>
                <input id="auth-username" type="text" name="username" autocomplete="username" autofocus required>
                <label for="auth-password">Password</label>
                <input id="auth-password" type="password" name="password" autocomplete="current-password" required>
                <button type="submit">Sign in</button>
            </form>
            {{sso_options}}
            {{register_link}}
        </main>
    </body>
</html>"#
    )
});

static REGISTER_HTML: LazyLock<String> = LazyLock::new(|| {
    format!(
        r#"
<!DOCTYPE html>
<html lang="en">
    <head>
        {ACCOUNT_HEAD}
        <title>Create account · Phantom</title>
    </head>
    <body class="auth-page">
        <main class="auth-card" aria-labelledby="auth-title">
            <h1 id="auth-title">Create account</h1>
            <p class="auth-description">Set up your account on this homeserver.</p>
            {{error}}
            <form class="auth-form" method="POST" action="/_phantom/oidc/native">
                <input type="hidden" name="oidc_req_id" value="{{req_id}}">
                <input type="hidden" name="mode" value="register">
                <label for="auth-username">Username</label>
                <input id="auth-username" type="text" name="username" autocomplete="username" autofocus required>
                <label for="auth-password">Password</label>
                <input id="auth-password" type="password" name="password" autocomplete="new-password" required>
                {{token_field}}
                <button type="submit">Create account</button>
            </form>
            <p class="auth-nav">Already have an account? <a href="/_phantom/oidc/native?oidc_req_id={{req_id_enc}}&amp;view=login">Sign in</a></p>
        </main>
    </body>
</html>"#
    )
});

static BOUND_HTML: LazyLock<String> = LazyLock::new(|| {
    format!(
        r#"
<!DOCTYPE html>
<html lang="en">
    <head>
        {ACCOUNT_HEAD}
        <title>Continue signing in · Phantom</title>
    </head>
    <body class="auth-page">
        <main class="auth-card" aria-labelledby="auth-title">
            <h1 id="auth-title">Continue signing in</h1>
            <p class="auth-description">Finish signing in with the provider you chose.</p>
            {{error}}
            {{sso_options}}
        </main>
    </body>
</html>"#
    )
});

static TOKEN_FIELD: &str = r#"<label for="auth-token">Registration token</label>
                <input id="auth-token" type="text" name="registration_token" autocomplete="off" placeholder="Enter your token" required>"#;

#[cfg(test)]
mod tests {
    use super::{Flow, error_block, render_bound, render_login, render_sso_options};

    #[test]
    fn login_page_has_form_and_hidden_req_id() {
        let html = render_login(Flow::Authorization("REQ123"), None, false, "");

        assert!(html.contains(r#"action="/_phantom/oidc/native""#));
        assert!(html.contains(r#"name="oidc_req_id" value="REQ123""#));
        assert!(html.contains(r#"name="username""#));
        assert!(html.contains(r#"name="password""#));
        assert!(!html.contains("view=register"));
    }

    #[test]
    fn login_page_links_to_register_when_enabled() {
        let html = render_login(Flow::Authorization("REQ123"), None, true, "");

        assert!(html.contains("oidc_req_id=REQ123&amp;view=register"));
    }

    #[test]
    fn login_page_offers_each_provider_with_a_bound_request() {
        let providers = [
            ("first/provider", "First provider"),
            ("second", "Second {error} <provider>"),
        ];

        let options = render_sso_options("Or sign in with", "REQ123", providers);
        let html = render_login(Flow::Authorization("REQ123"), None, false, &options);

        assert!(html.contains("oidc_req_id=REQ123&amp;idp_id=first%2Fprovider"));
        assert!(html.contains("oidc_req_id=REQ123&amp;idp_id=second"));
        assert!(html.contains("Second &#123;error&#125; &lt;provider&gt;"));
        assert!(!html.contains("<provider>"));
        assert!(html.contains(r#"name="password""#));
    }

    #[test]
    fn bound_page_offers_only_its_provider() {
        let html = render_bound(
            "REQ123",
            ("first", "First <provider>"),
            Some("Already chosen"),
        );

        assert!(html.contains("oidc_req_id=REQ123&amp;idp_id=first"));
        assert!(html.contains("First &lt;provider&gt;"));
        assert!(html.contains("Already chosen"));
        assert!(!html.contains(r#"name="password""#));
        assert!(!html.contains("{sso_options}"));
    }

    #[test]
    fn login_page_escapes_error_and_req_id() {
        let html = render_login(
            Flow::Authorization("a<b>c"),
            Some("<script>alert(1)</script>"),
            false,
            "",
        );

        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("a<b>c"));
        assert!(html.contains("a&lt;b&gt;c"));
    }

    #[test]
    fn login_page_does_not_expand_smuggled_placeholder() {
        // A req_id of "{error}" must not be re-expanded by the later error fill.
        let html = render_login(Flow::Authorization("{error}"), Some("BOOM"), false, "");

        assert_eq!(html.matches("BOOM").count(), 1);
        assert!(html.contains(r#"value="{error}""#));
    }

    #[test]
    fn device_login_page_has_only_hidden_user_code() {
        let html = render_login(Flow::Device("BCDF-GHJK"), None, true, "");

        assert!(html.contains(r#"name="user_code" value="BCDF-GHJK""#));
        assert!(!html.contains(r#"name="oidc_req_id""#));
        assert!(!html.contains("view=register"));
    }

    #[test]
    fn device_login_page_escapes_and_does_not_expand_context() {
        let html = render_login(Flow::Device("a<{error}>"), Some("BOOM"), true, "");

        assert_eq!(html.matches("BOOM").count(), 1);
        assert!(!html.contains("a<{error}>"));
        assert!(html.contains(r#"value="a&lt;{error}&gt;""#));
    }

    #[test]
    fn account_login_page_has_hidden_action_and_device_id() {
        let context = Flow::Account {
            action: "org.matrix.sessions_list",
            device_id: "",
        };

        let html = render_login(context, None, true, "");

        assert!(html.contains(r#"name="action" value="org.matrix.sessions_list""#));
        assert!(html.contains(r#"name="device_id" value="""#));
        assert!(!html.contains(r#"name="oidc_req_id""#));
        assert!(!html.contains(r#"name="user_code""#));
        assert!(!html.contains("view=register"));
    }

    #[test]
    fn account_login_page_escapes_and_does_not_expand_context() {
        let context = Flow::Account {
            action: "a<{error}>",
            device_id: "b<{error}>",
        };

        let html = render_login(context, Some("BOOM"), true, "");

        assert_eq!(html.matches("BOOM").count(), 1);
        assert!(!html.contains("a<{error}>"));
        assert!(!html.contains("b<{error}>"));
        assert!(html.contains(r#"name="action" value="a&lt;{error}&gt;""#));
        assert!(html.contains(r#"name="device_id" value="b&lt;{error}&gt;""#));
    }

    #[test]
    fn error_block_renders_only_when_present() {
        let block = error_block(None);

        assert!(block.is_empty(), "{block:?}");
        assert!(error_block(Some("oops")).contains(r#"class="err""#));
    }
}
