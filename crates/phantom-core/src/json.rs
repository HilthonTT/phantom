//! Canonical JSON, and the `serde` adapters for values that arrive as
//! strings.

use std::{fmt, str::FromStr};

use ruma::{CanonicalJsonError, CanonicalJsonObject, canonical_json::try_from_json_map};

use crate::Result;

/// Fallible conversion from any value that implements `Serialize` to a
/// `CanonicalJsonObject`.
///
/// `value` must serialize to an `serde_json::Value::Object`.
pub fn to_canonical_object<T: serde::Serialize>(
    value: T,
) -> Result<CanonicalJsonObject, CanonicalJsonError> {
    // ruma 0.16 dropped `CanonicalJsonError::SerDe`, so serialization failures
    // are reported through `Other` and a non-object through `InvalidType`.
    match serde_json::to_value(value)
        .map_err(|error| CanonicalJsonError::Other(error.to_string()))?
    {
        serde_json::Value::Object(map) => try_from_json_map(map),
        _ => Err(CanonicalJsonError::InvalidType(
            "a non-object value".to_owned(),
        )),
    }
}

pub fn deserialize_from_str<
    'de,
    D: serde::de::Deserializer<'de>,
    T: FromStr<Err = E>,
    E: fmt::Display,
>(
    deserializer: D,
) -> Result<T, D::Error> {
    struct Visitor<T: FromStr<Err = E>, E>(std::marker::PhantomData<T>);
    impl<T: FromStr<Err = Err>, Err: fmt::Display> serde::de::Visitor<'_> for Visitor<T, Err> {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "a parsable string")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            v.parse().map_err(serde::de::Error::custom)
        }
    }
    deserializer.deserialize_str(Visitor(std::marker::PhantomData))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize)]
    struct Event {
        sender: String,
        depth: u64,
    }

    #[test]
    fn objects_convert_to_canonical_json() {
        let object = to_canonical_object(Event {
            sender: "@alice:phantom.chat".to_owned(),
            depth: 3,
        })
        .expect("an object converts");

        assert_eq!(object.len(), 2);
        assert!(object.contains_key("sender"));
    }

    #[test]
    fn non_objects_are_rejected_as_invalid_type() {
        let error = to_canonical_object("just a string").expect_err("not an object");

        assert!(
            matches!(error, CanonicalJsonError::InvalidType(_)),
            "got {error:?}"
        );
    }

    #[test]
    fn deserialize_from_str_parses_and_reports_failures() {
        #[derive(serde::Deserialize, Debug)]
        struct Wrapper {
            #[serde(deserialize_with = "crate::json::deserialize_from_str")]
            addr: std::net::IpAddr,
        }

        let wrapper: Wrapper = toml::from_str(r#"addr = "127.0.0.1""#).expect("parsed");
        assert_eq!(wrapper.addr, std::net::IpAddr::from([127, 0, 0, 1]));

        let error = toml::from_str::<Wrapper>(r#"addr = "not-an-ip""#).expect_err("rejected");
        assert!(error.to_string().contains("invalid IP address"), "{error}");
    }
}
