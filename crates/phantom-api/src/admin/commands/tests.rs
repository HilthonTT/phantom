use clap::Parser;
use ruma::user_id;

use super::{Cli, Command, RoomCommand, UserCommand, complete, render, words};

fn parse(line: &str) -> Result<Cli, clap::Error> {
    let words = words::split(line).expect("balanced quotes");
    Cli::try_parse_from(words::argv(words, user_id!("@phantom:example.com")))
}

#[test]
fn quotes_keep_spaces_inside_a_word() {
    assert_eq!(
        words::split(r#"!admin users create bob "two words" 'a b'"#).unwrap(),
        ["!admin", "users", "create", "bob", "two words", "a b"]
    );
}

#[test]
fn a_backslash_escapes_the_next_character() {
    assert_eq!(words::split(r"a\ b \!admin").unwrap(), ["a b", "!admin"]);
}

#[test]
fn an_unclosed_quote_is_an_error() {
    assert!(words::split(r#"!admin users create "bob"#).is_err());
}

#[test]
fn the_invocation_becomes_the_program_name() {
    let server_user = user_id!("@phantom:example.com");
    for first in [
        "!admin",
        r"\!admin",
        "@phantom:example.com",
        "@phantom:example.com:",
    ] {
        let words = vec![first.to_owned(), "users".to_owned(), "list".to_owned()];
        assert_eq!(
            words::argv(words, server_user),
            ["!admin", "users", "list"],
            "{first}"
        );
    }
}

#[test]
fn subcommands_and_flags_parse() {
    let cli = parse("!admin users deactivate bob --erase").unwrap();
    assert!(matches!(
        cli.command,
        Command::Users(UserCommand::Deactivate { ref user, erase: true }) if user == "bob"
    ));

    let cli = parse("!admin rooms delete #lobby:example.com --force").unwrap();
    assert!(matches!(
        cli.command,
        Command::Rooms(RoomCommand::Delete { force: true, .. })
    ));
}

#[test]
fn reactivate_takes_an_optional_password() {
    let cli = parse("!admin users reactivate bob hunter2").unwrap();
    assert!(matches!(
        cli.command,
        Command::Users(UserCommand::Reactivate { ref user, password: Some(ref password) })
            if user == "bob" && password == "hunter2"
    ));

    let cli = parse("!admin users reactivate bob").unwrap();
    assert!(matches!(
        cli.command,
        Command::Users(UserCommand::Reactivate { password: None, .. })
    ));
}

#[test]
fn an_unknown_command_is_an_error_with_help() {
    let error = parse("!admin users frobnicate").unwrap_err();
    assert!(error.render().to_string().contains("frobnicate"));
}

#[test]
fn help_lists_the_command_groups() {
    let help = parse("!admin --help").unwrap_err().render().to_string();
    for group in [
        "users",
        "rooms",
        "tokens",
        "media",
        "federation",
        "reports",
        "server",
    ] {
        assert!(help.contains(group), "{group} missing from:\n{help}");
    }
}

#[test]
fn completion_finishes_a_unique_subcommand() {
    assert_eq!(complete("!admin us"), "!admin users ");
    assert_eq!(complete("!admin users deac"), "!admin users deactivate ");
    assert_eq!(
        complete("!admin r"),
        "!admin r",
        "rooms and reports are both possible"
    );
}

#[test]
fn a_table_aligns_its_columns() {
    let rows = serde_json::json!([
        { "user_id": "@a:x", "admin": true },
        { "user_id": "@longer:x", "admin": false },
    ]);
    let output = render::table(
        &rows,
        &[
            render::Column::new("user", "user_id"),
            render::Column::new("admin", "admin"),
        ],
    );

    assert_eq!(
        output.body(),
        "user       admin\n@a:x       true\n@longer:x  false\n\n2 rows"
    );
}

#[test]
fn an_empty_table_says_so() {
    let output = render::table(&serde_json::json!([]), &[render::Column::new("x", "x")]);
    assert_eq!(output.body(), "Nothing to show.");
}
