use phantom_core::html::escape as html_escape;

use super::DEVICE_HEAD;

pub(super) fn error_html(message: &str) -> String {
    let page_html = format!(
        r#"
<!DOCTYPE html>
<html lang="en">
	<head>
		{DEVICE_HEAD}
		<title>Error</title>
	</head>
	<body>
		<h1 class="err">Error</h1>
		<p>{{msg}}</p>
		<div class="nav">
			<a href="/_phantom/oidc/device">Try again</a>
		</div>
	</body>
</html>"#
    );

    page_html.replace("{msg}", &html_escape(message))
}
