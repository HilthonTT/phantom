//! The columns phantom keeps, and what each one's data does.
//!
//! This is the schema. A column is named for the shape of its entries —
//! `roomid_shortroomid` maps a room id to a short room id — and picks an
//! archetype from [`descriptor`] rather than spelling out three dozen engine
//! options. The archetype says two things: whether the column is large or
//! small, and whether writes land across the whole keyspace or at the end of
//! it. Everything else the engine needs follows from those.
//!
//! Adding a column here is all that is required to create it; the engine opens
//! whatever is listed and leaves anything it finds but does not recognise
//! alone. Removing one from this list does **not** delete it — see
//! [`descriptor::DROPPED`].

use std::{collections::BTreeMap, sync::Arc};

use phantom_core::Result;

use crate::{
    Engine,
    engine::descriptor::{self, CacheDisp, Descriptor},
    map::Map,
};

pub(crate) type Maps = BTreeMap<MapsKey, MapsVal>;
pub(crate) type MapsKey = &'static str;
pub(crate) type MapsVal = Arc<Map>;

/// Opens a handle on each of `maps`, keyed by name.
#[tracing::instrument(name = "maps", level = "debug", skip_all)]
pub(crate) fn open_list(db: &Arc<Engine>, maps: &[Descriptor]) -> Result<Maps> {
    maps.iter()
        .map(|desc| Ok((desc.name, Map::open(db, desc.name)?)))
        .collect()
}

/// Every column the server opens.
///
/// Kept in alphabetical order so that a name can be found by eye and a
/// diff adding one reads as an addition.
pub(crate) static MAPS: &[Descriptor] = &[
    Descriptor {
        name: "alias_roomid",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "alias_userid",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "aliasid_alias",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "backupid_algorithm",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "backupid_etag",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "backupkeyid_backup",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "bannedroomids",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "disabledroomids",
        ..descriptor::RANDOM_SMALL
    },
    // Events that arrived without their place in a room's history. Held with
    // the room's own events, which is what they are compared against.
    Descriptor {
        name: "eventid_outlierpdu",
        cache_disp: CacheDisp::SharedWith("pduid_pdu"),
        block_size: 1024,
        index_size: 512,
        ..descriptor::RANDOM
    },
    // Read on the way into nearly every request that names an event, so it
    // gets a cache of its own rather than competing for the shared one.
    Descriptor {
        name: "eventid_pduid",
        cache_disp: CacheDisp::Unique,
        block_size: 512,
        index_size: 512,
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "eventid_shorteventid",
        cache_disp: CacheDisp::Unique,
        block_size: 512,
        index_size: 512,
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "global",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "id_appserviceregistrations",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "keychangeid_userid",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "keyid_key",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "lazyloadedids",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "logintoken_expiresatuserid",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "mediaid_file",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "mediaid_user",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "onetimekeyid_onetimekeys",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "openidtoken_expiresatuserid",
        ..descriptor::RANDOM_SMALL
    },
    // The events themselves, and the largest column by far. Keys are ordered
    // by room and then by position in it, so writes land at the end of each
    // room's range rather than across the keyspace.
    Descriptor {
        name: "pduid_pdu",
        cache_disp: CacheDisp::SharedWith("eventid_outlierpdu"),
        block_size: 2048,
        index_size: 512,
        ..descriptor::SEQUENTIAL
    },
    Descriptor {
        name: "presenceid_presence",
        ..descriptor::SEQUENTIAL_SMALL
    },
    Descriptor {
        name: "publicroomids",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "pushkey_deviceid",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "readreceiptid_readreceipt",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "referencedevents",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "roomid_invitedcount",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "roomid_inviteviaservers",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "roomid_joinedcount",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "roomid_pduleaves",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "roomid_shortroomid",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "roomid_shortstatehash",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "roomserverids",
        ..descriptor::RANDOM_SMALL
    },
    // Written once per sync token per room and never updated, so it compacts
    // hard: the data is cold the moment the next token is issued.
    Descriptor {
        name: "roomsynctoken_shortstatehash",
        file_shape: 3,
        block_size: 512,
        compression_level: 3,
        bottommost_level: Some(6),
        ..descriptor::SEQUENTIAL
    },
    Descriptor {
        name: "roomuserdataid_accountdata",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "roomuserid_invitecount",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "roomuserid_joined",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "roomuserid_knockedcount",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "roomuserid_lastprivatereadupdate",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "roomuserid_leftcount",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "roomuserid_privateread",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "roomuseroncejoinedids",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "roomusertype_roomuserdataid",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "senderkey_pusher",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "server_signingkeys",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "servercurrentevent_data",
        ..descriptor::RANDOM_SMALL
    },
    // Resolution results for remote servers, which go stale on their own; the
    // cache archetype drops the oldest entries once the column fills.
    Descriptor {
        name: "servername_destination",
        ..descriptor::RANDOM_SMALL_CACHE
    },
    Descriptor {
        name: "servername_educount",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "servername_override",
        ..descriptor::RANDOM_SMALL_CACHE
    },
    Descriptor {
        name: "servernameevent_data",
        cache_disp: CacheDisp::Unique,
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "serverroomids",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "shorteventid_authchain",
        cache_disp: CacheDisp::Unique,
        ..descriptor::SEQUENTIAL
    },
    Descriptor {
        name: "shorteventid_eventid",
        cache_disp: CacheDisp::Unique,
        ..descriptor::SEQUENTIAL_SMALL
    },
    Descriptor {
        name: "shorteventid_shortstatehash",
        block_size: 512,
        index_size: 512,
        ..descriptor::SEQUENTIAL
    },
    Descriptor {
        name: "shortstatehash_statediff",
        ..descriptor::SEQUENTIAL_SMALL
    },
    Descriptor {
        name: "shortstatekey_statekey",
        cache_disp: CacheDisp::Unique,
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "softfailedeventids",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "statehash_shortstatehash",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "statekey_shortstatekey",
        cache_disp: CacheDisp::Unique,
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "threadid_userids",
        ..descriptor::SEQUENTIAL_SMALL
    },
    Descriptor {
        name: "todeviceid_events",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "tofrom_relation",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "token_userdeviceid",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "tokenids",
        block_size: 512,
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "url_previews",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "userdeviceid_metadata",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userdeviceid_token",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userdevicesessionid_uiaainfo",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userdevicetxnid_response",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userfilterid_filter",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userid_avatarurl",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userid_blurhash",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userid_devicelistversion",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userid_displayname",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userid_lastonetimekeyupdate",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userid_masterkeyid",
        ..descriptor::RANDOM_SMALL
    },
    // Password hashes are read on every login and nowhere else, so this is
    // sized for reads that miss rather than for a working set.
    Descriptor {
        name: "userid_password",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "userid_presenceid",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userid_selfsigningkeyid",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userid_usersigningkeyid",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "useridprofilekey_value",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userroomid_highlightcount",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "userroomid_invitestate",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userroomid_joined",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "userroomid_knockedstate",
        ..descriptor::RANDOM_SMALL
    },
    Descriptor {
        name: "userroomid_leftstate",
        ..descriptor::RANDOM
    },
    Descriptor {
        name: "userroomid_notificationcount",
        ..descriptor::RANDOM
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Columns are addressed by name and opened into a map keyed by it, so a
    /// duplicate would silently shadow rather than fail.
    #[test]
    fn column_names_are_unique() {
        let mut names: Vec<_> = MAPS.iter().map(|desc| desc.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();

        assert_eq!(names.len(), count, "a column name is repeated");
    }

    #[test]
    fn column_names_are_in_order() {
        let names: Vec<_> = MAPS.iter().map(|desc| desc.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();

        assert_eq!(names, sorted, "keep the column list alphabetical");
    }

    /// A column sharing another's cache has to name one that exists, or it
    /// would silently fall back to a cache of its own.
    #[test]
    fn shared_caches_name_a_real_column() {
        for desc in MAPS {
            let CacheDisp::SharedWith(name) = desc.cache_disp else {
                continue;
            };

            assert!(
                MAPS.iter().any(|other| other.name == name),
                "{} shares the cache of {name:?}, which is not a column",
                desc.name
            );
        }
    }

    /// The name the shared cache is held under is not a column name, and a
    /// column taking it would collide with it.
    #[test]
    fn no_column_takes_the_shared_cache_name() {
        assert!(
            !MAPS
                .iter()
                .any(|desc| desc.name == crate::engine::Context::SHARED_CACHE),
            "a column may not be named for the shared cache"
        );
    }

    /// Nothing in the live list should be carrying the tombstone, which is
    /// only for columns the engine finds but no longer describes.
    #[test]
    fn no_live_column_is_dropped() {
        for desc in MAPS {
            assert!(
                !desc.dropped,
                "{} is described but marked dropped",
                desc.name
            );
        }
    }
}
