use phantom_core::html::escape as html_escape;

use super::DEVICE_HEAD;

pub(super) fn result_html(title: &str, message: &str) -> String {
    let page_html = format!(
        r#"
<!DOCTYPE html>
<html lang="en">
	<head>
		{DEVICE_HEAD}
		<title>{{title}}</title>
	</head>
	<body>
		<h1>{{title}}</h1>
		<p>{{message}}</p>
	</body>
</html>"#
    );

    page_html
        .replace("{title}", &html_escape(title))
        .replace("{message}", &html_escape(message))
}
