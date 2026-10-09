use axum::{Json, extract::State, response::IntoResponse};
use phantom_core::Result;
use serde::Serialize;

use crate::router::{AdminAuth, State as RouterState};

#[derive(Serialize)]
pub(super) struct Setting {
    key: String,
    value: String,
}

/// # `GET /_phantom/admin/v1/settings`
///
/// The running configuration, as the config's own summary prints it: fields
/// marked sensitive are masked, hidden ones and whole tables of secrets
/// (identity and storage providers, SMTP) are left out.
pub(super) async fn settings(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let config: &phantom_core::Config = &services.config;

    Ok(Json(parse(&config.to_string())))
}

/// Reads the summary's `| name | value |` rows; a value may itself hold a `|`,
/// so only the first separator splits.
fn parse(summary: &str) -> Vec<Setting> {
    summary
        .lines()
        .skip(2)
        .filter_map(|line| {
            let row = line.strip_prefix("| ")?.strip_suffix(" |")?;
            let (key, value) = row.split_once(" | ")?;

            Some(Setting {
                key: key.to_owned(),
                value: value.to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn rows_are_read_past_the_header() {
        let summary =
            "| name | value |\n| :--- | :---  |\n| port | 8008 |\n| filter | \"a | b\" |\n";

        let settings = parse(summary);
        let pairs: Vec<_> = settings
            .iter()
            .map(|s| (s.key.as_str(), s.value.as_str()))
            .collect();

        assert_eq!(pairs, [("port", "8008"), ("filter", "\"a | b\"")]);
    }
}
