//! Which remote servers this operator refuses, and how far the refusal goes.
//!
//! The refusal has two sources: the `forbidden_remote_*` patterns of the
//! config file, and the block list an admin edits at runtime, kept in the
//! database. Both are regular expressions matched against a server's host, so
//! a pattern means the same wherever it was written. The runtime list is held
//! compiled in memory and recompiled only when it changes, because `forbids`
//! sits on every federation request in and out.

use std::{
    fmt::Display,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use futures::StreamExt;
use phantom_core::{
    Err, Result, err, implement, info, runtime::server::Server, stream::TryIgnore, time::now_millis,
};
use phantom_database::{Json, Map, table};
use regex::RegexSet;
use ruma::{OwnedUserId, ServerName, UserId};
use serde::{Deserialize, Serialize};

use crate::{Dep, ops::server_state};

pub struct Service {
    services: Services,
    servername_blocked: Arc<Map>,
    blocks: RwLock<Blocks>,
}

struct Services {
    server: Arc<Server>,
    server_state: Dep<server_state::Service>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Restriction {
    Federation,

    RoomDirectory,

    Media,
}

/// One pattern of the runtime block list, as stored.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Block {
    pub pattern: String,
    pub reason: Option<String>,
    pub by: Option<OwnedUserId>,
    pub at_ms: u64,
}

/// The runtime block list, and its patterns compiled into one set.
#[derive(Default)]
struct Blocks {
    entries: Vec<Block>,
    set: RegexSet,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                server: args.server.clone(),
                server_state: args.depend::<server_state::Service>(),
            },
            servername_blocked: args.db[table::SERVERNAME_BLOCKED].clone(),
            blocks: RwLock::new(Blocks::default()),
        }))
    }

    async fn worker(self: Arc<Self>) -> Result<()> {
        self.reload_blocks().await
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
#[must_use]
pub fn forbids(&self, server: &ServerName, restriction: Restriction) -> bool {
    if self.services.server_state.server_is_ours(server) {
        return false;
    }

    let config = &self.services.server.config;
    let host = server.host();

    config
        .federation
        .forbidden_remote_server_names
        .is_match(host)
        || self.is_blocked(host)
        || match restriction {
            Restriction::Federation => false,
            Restriction::RoomDirectory => config
                .federation
                .forbidden_remote_room_directory_server_names
                .is_match(host),
            Restriction::Media => config
                .federation
                .forbidden_remote_media_server_names
                .is_match(host),
        }
}

#[implement(Service)]
#[must_use]
pub fn why_forbidden(&self, server: &ServerName, restriction: Restriction) -> Vec<String> {
    let config = &self.services.server.config;
    let host = server.host();

    let narrow = match restriction {
        Restriction::Federation => None,
        Restriction::RoomDirectory => Some(
            &config
                .federation
                .forbidden_remote_room_directory_server_names,
        ),
        Restriction::Media => Some(&config.federation.forbidden_remote_media_server_names),
    };

    let blocks = self.blocks.read().expect("locked");

    matched(&config.federation.forbidden_remote_server_names, host)
        .chain(matched(&blocks.set, host))
        .chain(narrow.into_iter().flat_map(|set| matched(set, host)))
        .collect()
}

/// Whether the runtime block list matches the host.
#[implement(Service)]
fn is_blocked(&self, host: &str) -> bool {
    self.blocks.read().expect("locked").set.is_match(host)
}

/// The runtime block list, oldest entry first.
#[implement(Service)]
#[must_use]
pub fn blocks(&self) -> Vec<Block> {
    self.blocks.read().expect("locked").entries.clone()
}

/// The config file's `forbidden_remote_server_names`, which refuse a server
/// as the runtime list does but can only be changed in the file.
#[implement(Service)]
#[must_use]
pub fn config_blocks(&self) -> Vec<String> {
    self.services
        .server
        .config
        .federation
        .forbidden_remote_server_names
        .patterns()
        .to_vec()
}

/// Adds a pattern to the runtime block list. Answers false when the list
/// already had it.
#[implement(Service)]
pub async fn block(&self, pattern: &str, reason: Option<String>, by: &UserId) -> Result<bool> {
    let compiled = regex::Regex::new(pattern).map_err(|e| {
        err!(Request(InvalidParam(
            "{pattern:?} is not a regular expression: {e}"
        )))
    })?;

    let ours = self.services.server_state.server_name();
    if compiled.is_match(ours.host()) {
        return Err!(Request(InvalidParam(
            "{pattern:?} matches this server's own name; it would block nothing, so it is refused."
        )));
    }

    if self.servername_blocked.exists(pattern).await.is_ok() {
        return Ok(false);
    }

    // Each pattern compiles alone, but the list is matched as one set, which
    // has a size limit of its own: refuse the pattern that would break it
    // rather than store a list that no longer compiles.
    let mut patterns: Vec<String> = self
        .blocks()
        .into_iter()
        .map(|block| block.pattern)
        .collect();
    patterns.push(pattern.to_owned());
    if let Err(e) = RegexSet::new(&patterns) {
        return Err!(Request(InvalidParam(
            "Adding {pattern:?} would make the block list too large to match: {e}"
        )));
    }

    let block = Block {
        pattern: pattern.to_owned(),
        reason,
        by: Some(by.to_owned()),
        at_ms: now_millis(),
    };

    self.servername_blocked.raw_put(pattern, Json(&block))?;
    self.reload_blocks().await?;

    info!(%pattern, %by, "Blocked servers");

    Ok(true)
}

/// Removes one pattern from the runtime block list, as it was written.
#[implement(Service)]
pub async fn unblock_pattern(&self, pattern: &str) -> Result {
    if self.servername_blocked.exists(pattern).await.is_err() {
        return Err!(Request(NotFound(
            "{pattern:?} is not on the runtime block list."
        )));
    }

    self.servername_blocked.remove(pattern)?;
    self.reload_blocks().await?;

    info!(%pattern, "Unblocked servers");

    Ok(())
}

/// Removes every runtime pattern matching the server, answering with the
/// patterns removed. The config file's patterns are left alone; `forbids`
/// tells whether the server is still refused because of one.
#[implement(Service)]
pub async fn unblock_server(&self, server: &ServerName) -> Result<Vec<String>> {
    let host = server.host();
    let matching: Vec<String> = self
        .blocks()
        .into_iter()
        .map(|block| block.pattern)
        .filter(|pattern| regex::Regex::new(pattern).is_ok_and(|re| re.is_match(host)))
        .collect();

    for pattern in &matching {
        self.servername_blocked.remove(pattern.as_str())?;
    }

    self.reload_blocks().await?;

    if !matching.is_empty() {
        info!(%server, ?matching, "Unblocked a server");
    }

    Ok(matching)
}

/// Reads the runtime block list back from the database and recompiles it.
/// A stored pattern that no longer compiles is skipped and logged rather
/// than failing the whole list, which would unblock everything. If the list
/// as a whole does not compile, the one in force is kept.
#[implement(Service)]
async fn reload_blocks(&self) -> Result {
    let entries: Vec<Block> = self
        .servername_blocked
        .raw_stream()
        .ignore_err()
        .filter_map(async |(_, value)| serde_json::from_slice(value).ok())
        .collect()
        .await;

    let blocks = compile(entries).inspect_err(|e| {
        phantom_core::error!(
            "The runtime block list does not compile; keeping the one in force: {e}"
        );
    })?;
    *self.blocks.write().expect("locked") = blocks;

    Ok(())
}

fn compile(mut entries: Vec<Block>) -> Result<Blocks> {
    entries.sort_by_key(|block| block.at_ms);
    entries.retain(|block| match regex::Regex::new(&block.pattern) {
        Ok(_) => true,
        Err(e) => {
            phantom_core::warn!(pattern = %block.pattern, "Ignoring a stored block pattern: {e}");
            false
        }
    });

    // Never an empty set on failure: that would unblock every server while
    // the list still reads as blocking them.
    let set = RegexSet::new(entries.iter().map(|block| block.pattern.as_str()))
        .map_err(|e| err!("The block patterns do not compile together: {e}"))?;

    Ok(Blocks { entries, set })
}

/// The pattern that blocks exactly one server: its host, escaped and
/// anchored. The config file's patterns are unanchored, so `evil.example`
/// written there would also refuse `notevil.example`; a block made by name
/// should not.
#[must_use]
pub fn exact_pattern(server: &ServerName) -> String {
    format!("^{}$", regex::escape(server.host()))
}

fn matched<'a>(set: &'a RegexSet, host: &'a str) -> impl Iterator<Item = String> + 'a {
    set.matches(host)
        .into_iter()
        .filter_map(|index| set.patterns().get(index))
        .cloned()
}

impl Display for Restriction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Federation => "federation",
            Self::RoomDirectory => "room directory access",
            Self::Media => "media downloads",
        };

        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use ruma::server_name;

    use super::{Block, compile, exact_pattern};

    fn block(pattern: &str, at_ms: u64) -> Block {
        Block {
            pattern: pattern.to_owned(),
            reason: None,
            by: None,
            at_ms,
        }
    }

    #[test]
    fn an_exact_block_matches_only_that_host() {
        let pattern = exact_pattern(server_name!("evil.example:8448"));
        let blocks = compile(vec![block(&pattern, 1)]).expect("the patterns compile");

        assert_eq!(pattern, r"^evil\.example$");
        assert!(blocks.set.is_match("evil.example"));
        assert!(!blocks.set.is_match("notevil.example"));
        assert!(!blocks.set.is_match("evilXexample"));
        assert!(!blocks.set.is_match("sub.evil.example"));
    }

    #[test]
    fn patterns_match_as_the_config_list_does() {
        let blocks = compile(vec![block(r"\.spam\.example$", 1), block("^bad", 2)])
            .expect("the patterns compile");

        assert!(blocks.set.is_match("a.spam.example"));
        assert!(blocks.set.is_match("badserver.org"));
        assert!(!blocks.set.is_match("good.example"));
    }

    #[test]
    fn a_broken_pattern_is_skipped_not_fatal() {
        let blocks =
            compile(vec![block("(unclosed", 1), block("^bad", 2)]).expect("the patterns compile");

        assert_eq!(blocks.entries.len(), 1);
        assert!(blocks.set.is_match("bad.example"));
    }

    #[test]
    fn entries_are_oldest_first() {
        let blocks = compile(vec![block("^b", 2), block("^a", 1)]).expect("the patterns compile");
        let order: Vec<&str> = blocks
            .entries
            .iter()
            .map(|block| block.pattern.as_str())
            .collect();

        assert_eq!(order, ["^a", "^b"]);
    }

    #[test]
    fn an_empty_list_matches_nothing() {
        let blocks = compile(Vec::new()).expect("the patterns compile");

        assert!(!blocks.set.is_match("anything.example"));
    }
}
