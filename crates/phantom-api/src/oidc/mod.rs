/// Raw JS served at `/_phantom/oidc/account.js`.
/// Referenced via `<script src>` for CSP compatibility.
static ACCOUNT_JS: &str = include_str!("account.js");

/// Shared stylesheet served at `/_phantom/oidc/account.css`.
static ACCOUNT_CSS: &str = include_str!("account.css");

pub(super) static ACCOUNT_HEAD: &str = r#"
	<meta charset="UTF-8">
	<link rel="stylesheet" href="/_phantom/oidc/account.css">
"#;

static ACCOUNT_JS_INCLUDE: &str = r#"
	<script src="/_phantom/oidc/account.js"></script>
"#;

/// Cache-control header value.
static ACCOUNT_CACHE_CONTROL: &str = "no-store";
