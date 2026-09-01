use figment::providers::Toml;

use super::*;

fn config(toml: &str) -> Result<Config> {
    Config::new(&Figment::new().merge(Toml::string(toml).nested()))
}

#[test]
fn defaults_apply_and_bind_addrs_cross_product() {
    let config = config(
        r#"
        [global]
        server_name = "phantom.chat"
        database_path = "/var/lib/phantom"
        port = [8008, 8448]
        "#,
    )
    .expect("config is valid");

    assert_eq!(
        config.get_bind_addrs().len(),
        4,
        "2 default addrs x 2 ports"
    );
    assert!(!config.allow_metrics, "serde default");
}

#[test]
fn unknown_keys_land_in_catchall() {
    let config = config(
        r#"
        [global]
        server_name = "phantom.chat"
        database_path = "/var/lib/phantom"
        not_a_real_option = 5
        "#,
    )
    .expect("config is valid");

    assert!(config.catchall.contains_key("not_a_real_option"));
}

#[test]
fn display_masks_sensitive_and_lists_fields() {
    let config = config(
        r#"
        [global]
        server_name = "phantom.chat"
        database_path = "/var/lib/phantom"
        registration_token = "hunter2"
        turn_secret = "swordfish"
        "#,
    )
    .expect("config is valid");

    let rendered = config.to_string();
    assert!(rendered.contains("| server_name | \"phantom.chat\" |"));
    assert!(rendered.contains("| registration_token | *********** |"));
    assert!(rendered.contains("| turn_secret | *********** |"));
    assert!(!rendered.contains("hunter2"), "secret must not be rendered");
    assert!(
        !rendered.contains("swordfish"),
        "secret must not be rendered"
    );
    assert!(
        !rendered.contains("catchall"),
        "ignored field is not a config option"
    );
}

#[test]
fn regex_options_are_compiled_while_the_config_loads() {
    let config = config(
        r#"
        [global]
        server_name = "phantom.chat"
        database_path = "/var/lib/phantom"
        forbidden_usernames = ["b[a4]dusernam[3e]", "badphrase"]
        "#,
    )
    .expect("config is valid");

    assert!(config.forbidden_usernames.is_match("b4dusername"));
    assert!(!config.forbidden_usernames.is_match("goodusername"));
    assert!(
        config.forbidden_alias_names.is_empty(),
        "an unset regex option is an empty set, not a set matching everything"
    );
}

#[test]
fn a_malformed_regex_is_an_error() {
    assert!(
        config(
            r#"
            [global]
            server_name = "phantom.chat"
            database_path = "/var/lib/phantom"
            forbidden_usernames = ["b[adusername"]
            "#,
        )
        .is_err()
    );
}

#[test]
fn an_empty_registration_token_is_an_error() {
    assert!(
        config(
            r#"
            [global]
            server_name = "phantom.chat"
            database_path = "/var/lib/phantom"
            registration_token = ""
            "#,
        )
        .is_err(),
        "an empty token is a half-written config, not a token"
    );
}

#[test]
fn an_unreadable_registration_token_file_is_an_error() {
    assert!(
        config(
            r#"
            [global]
            server_name = "phantom.chat"
            database_path = "/var/lib/phantom"
            registration_token_file = "/nonexistent/phantom/.reg_token"
            "#,
        )
        .is_err(),
        "the service would silently fall back to no token at all"
    );
}

#[test]
fn missing_required_option_is_an_error() {
    assert!(config("[global]\nserver_name = \"phantom.chat\"\n").is_err());
}
