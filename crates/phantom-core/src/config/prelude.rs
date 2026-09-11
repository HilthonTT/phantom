//! What the config modules share.
//!
//! Each module here is a slice of one flat TOML table, so they all reach for
//! the same handful of types and the same `serde` default functions. Naming
//! them once keeps thirteen near-identical import blocks from drifting.

pub(super) use std::{collections::BTreeSet, net::IpAddr, path::PathBuf};

pub(super) use bytesize::ByteSize;
pub(super) use either::Either;
pub(super) use phantom_macros::config_example_generator;
pub(super) use regex::RegexSet;
pub(super) use ruma::OwnedServerName;
pub(super) use serde::Deserialize;
pub(super) use url::Url;

pub(super) use super::{defaults::*, listen::IpLookupStrategy, proxy::ProxyConfig};
