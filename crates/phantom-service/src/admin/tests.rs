use ruma::{OwnedUserId, UserId};

use super::{Invocation, invocation};

fn server_user() -> OwnedUserId {
    UserId::parse("@phantom:example.com").expect("valid user id")
}

#[test]
fn bang_prefix_is_direct() {
    assert_eq!(
        invocation("!admin users list-users", &server_user()),
        Some(Invocation::Direct)
    );
}

#[test]
fn server_user_is_addressed_directly() {
    assert_eq!(
        invocation("@phantom:example.com help", &server_user()),
        Some(Invocation::Direct)
    );
}

#[test]
fn another_server_user_is_not() {
    assert_eq!(
        invocation("@phantom:other.example help", &server_user()),
        None
    );
}

#[test]
fn backslash_escapes_the_prefix() {
    assert_eq!(
        invocation("\\!admin server uptime", &server_user()),
        Some(Invocation::Escaped)
    );
}

/// A client that escapes the escape still means the escape: what matters is
/// that `!admin` follows the backslashes, not how many of them a client added.
#[test]
fn repeated_backslashes_still_escape() {
    assert_eq!(
        invocation("\\\\!admin server uptime", &server_user()),
        Some(Invocation::Escaped)
    );
}

#[test]
fn a_backslash_alone_is_not_an_invocation() {
    assert_eq!(invocation("\\so I said", &server_user()), None);
}

/// The prefix has to start the message. Naming the command mid-sentence is
/// talking about it, which is what the escape exists for.
#[test]
fn the_prefix_has_to_lead() {
    assert_eq!(invocation("try !admin help", &server_user()), None);
    assert_eq!(invocation("ask @phantom:example.com", &server_user()), None);
}

#[test]
fn ordinary_messages_are_not_commands() {
    assert_eq!(invocation("", &server_user()), None);
    assert_eq!(invocation("admin", &server_user()), None);
    assert_eq!(invocation("hello", &server_user()), None);
}
