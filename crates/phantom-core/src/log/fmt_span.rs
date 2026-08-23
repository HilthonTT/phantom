//! Parsing for the `log_span_events` config option.

use tracing_subscriber::fmt::format::FmtSpan;

use crate::{Err, Result};

/// Every name [`from_str`] accepts, in the order they are documented.
const NAMES: &[(&str, FmtSpan)] = &[
    ("none", FmtSpan::NONE),
    ("new", FmtSpan::NEW),
    ("enter", FmtSpan::ENTER),
    ("exit", FmtSpan::EXIT),
    ("close", FmtSpan::CLOSE),
    ("active", FmtSpan::ACTIVE),
    ("full", FmtSpan::FULL),
];

/// Parses the span lifecycle points to log.
///
/// Several may be combined with `,` or `|`, e.g. `"new,close"`; the reference
/// implementation took a single name and silently fell back to `none` for
/// anything it did not recognise, which turned a typo into missing logs. Here a
/// bad name is an error the caller can attribute to the config option it came
/// from.
pub fn from_str(str: &str) -> Result<FmtSpan> {
    str.split([',', '|'])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .try_fold(FmtSpan::NONE, |events, name| Ok(events | parse_one(name)?))
}

fn parse_one(name: &str) -> Result<FmtSpan> {
    NAMES
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, events)| events.clone())
        .map_or_else(
            || {
                let expected = NAMES
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ");

                Err!("unknown span event {name:?}; expected one or more of: {expected}")
            },
            Ok,
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_documented_name() {
        for (name, expected) in NAMES {
            assert_eq!(from_str(name).expect(name), *expected);
            assert_eq!(from_str(&name.to_uppercase()).expect(name), *expected);
        }
    }

    #[test]
    fn combines_names() {
        let expected = FmtSpan::NEW | FmtSpan::CLOSE;

        assert_eq!(from_str("new,close").expect("parsed"), expected);
        assert_eq!(from_str("new|close").expect("parsed"), expected);
        assert_eq!(from_str(" new , close ").expect("parsed"), expected);
        assert_eq!(
            from_str("enter,exit").expect("parsed"),
            FmtSpan::ACTIVE,
            "enter|exit is what ACTIVE means"
        );
    }

    #[test]
    fn empty_input_logs_no_span_events() {
        assert_eq!(from_str("").expect("parsed"), FmtSpan::NONE);
        assert_eq!(from_str(",").expect("parsed"), FmtSpan::NONE);
    }

    #[test]
    fn a_typo_is_an_error_naming_the_alternatives() {
        let error = from_str("cloze").expect_err("rejected");
        let message = error.message();

        assert!(message.contains("\"cloze\""), "{message}");
        assert!(message.contains("close"), "{message}");

        assert!(from_str("new,cloze").is_err(), "one bad name fails the set");
    }
}
