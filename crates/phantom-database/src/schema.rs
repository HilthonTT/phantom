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

#[tracing::instrument(name = "maps", level = "debug", skip_all)]
pub(crate) fn open_list(db: &Arc<Engine>, maps: &[Descriptor]) -> Result<Maps> {
    maps.iter()
        .map(|desc| Ok((desc.name, Map::open(db, desc.name)?)))
        .collect()
}

/// A table of the database, as named by a constant in [`table`].
///
/// The only way to index a [`Database`](crate::Database) outside this crate's
/// tests, so a mistyped table name is a compile error. tuwunel indexes by
/// string, where the same typo panics when the service using it is built.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Table(&'static str);

impl Table {
    #[inline]
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.0
    }
}

/// Declares every table once, as both its constant in [`table`] and its
/// entry in [`MAPS`], so neither can exist without the other.
macro_rules! schema {
    ($($table:ident = $name:literal { $($descriptor:tt)* })*) => {
        /// One constant per table of the database.
        pub mod table {
            use super::Table;

            $(pub const $table: Table = Table($name);)*
        }

        pub(crate) static MAPS: &[Descriptor] = &[$(Descriptor { name: $name, $($descriptor)* }),*];
    };
}

schema! {
    ALIAS_ROOMID = "alias_roomid" {
        ..descriptor::RANDOM_SMALL
    }

    ALIAS_USERID = "alias_userid" {
        ..descriptor::RANDOM_SMALL
    }

    ALIASID_ALIAS = "aliasid_alias" {
        ..descriptor::RANDOM_SMALL
    }

    BACKUPID_ALGORITHM = "backupid_algorithm" {
        ..descriptor::RANDOM_SMALL
    }

    BACKUPID_ETAG = "backupid_etag" {
        ..descriptor::RANDOM_SMALL
    }

    BACKUPKEYID_BACKUP = "backupkeyid_backup" {
        ..descriptor::RANDOM_SMALL
    }

    BANNEDROOMIDS = "bannedroomids" {
        ..descriptor::RANDOM_SMALL
    }

    DISABLEDROOMIDS = "disabledroomids" {
        ..descriptor::RANDOM_SMALL
    }

    EMAIL_USERID = "email_userid" {
        ..descriptor::RANDOM_SMALL
    }

    EVENTID_ORIGINALPDU = "eventid_originalpdu" {
        block_size: 2048,
        index_size: 512,
        ..descriptor::RANDOM
    }

    EVENTID_OUTLIERPDU = "eventid_outlierpdu" {
        cache_disp: CacheDisp::SharedWith("pduid_pdu"),
        block_size: 1024,
        index_size: 512,
        ..descriptor::RANDOM
    }

    EVENTID_PDUID = "eventid_pduid" {
        cache_disp: CacheDisp::Unique,
        block_size: 512,
        index_size: 512,
        ..descriptor::RANDOM
    }

    EVENTID_SHORTEVENTID = "eventid_shorteventid" {
        cache_disp: CacheDisp::Unique,
        block_size: 512,
        index_size: 512,
        ..descriptor::RANDOM
    }

    GLOBAL = "global" {
        ..descriptor::RANDOM_SMALL
    }

    ID_APPSERVICEREGISTRATIONS = "id_appserviceregistrations" {
        ..descriptor::RANDOM_SMALL
    }

    KEYCHANGEID_USERID = "keychangeid_userid" {
        ..descriptor::RANDOM
    }

    KEYID_KEY = "keyid_key" {
        ..descriptor::RANDOM_SMALL
    }

    LAZYLOADEDIDS = "lazyloadedids" {
        ..descriptor::RANDOM_SMALL
    }

    LOGINTOKEN_EXPIRESATUSERID = "logintoken_expiresatuserid" {
        ..descriptor::RANDOM_SMALL
    }

    MEDIAID_FILE = "mediaid_file" {
        ..descriptor::RANDOM_SMALL
    }

    MEDIAID_LAZY = "mediaid_lazy" {
        ..descriptor::RANDOM_SMALL
    }

    MEDIAID_LAZYCONTENT = "mediaid_lazycontent" {
        ..descriptor::RANDOM
    }

    MEDIAID_PENDING = "mediaid_pending" {
        ..descriptor::RANDOM_SMALL
    }

    MEDIAID_USER = "mediaid_user" {
        ..descriptor::RANDOM_SMALL
    }

    OAUTHID_SESSION = "oauthid_session" {
        ..descriptor::RANDOM_SMALL
    }

    OAUTHUNIQID_OAUTHID = "oauthuniqid_oauthid" {
        ..descriptor::RANDOM_SMALL
    }

    OIDC_SIGNINGKEY = "oidc_signingkey" {
        ..descriptor::RANDOM_SMALL
    }

    OIDCCLIENTID_REGISTRATION = "oidcclientid_registration" {
        ..descriptor::RANDOM_SMALL
    }

    OIDCCODE_AUTHSESSION = "oidccode_authsession" {
        ..descriptor::RANDOM_SMALL
    }

    OIDCDEVICE_USERDEVICEID = "oidcdevice_userdeviceid" {
        ..descriptor::RANDOM_SMALL
    }

    OIDCDEVICECODE_DEVICEGRANT = "oidcdevicecode_devicegrant" {
        ..descriptor::RANDOM_SMALL
    }

    OIDCREQID_AUTHREQUEST = "oidcreqid_authrequest" {
        ..descriptor::RANDOM_SMALL
    }

    OIDCUSERCODE_DEVICECODE = "oidcusercode_devicecode" {
        ..descriptor::RANDOM_SMALL
    }

    ONETIMEKEYID_ONETIMEKEYS = "onetimekeyid_onetimekeys" {
        ..descriptor::RANDOM_SMALL
    }

    OPENIDTOKEN_EXPIRESATUSERID = "openidtoken_expiresatuserid" {
        ..descriptor::RANDOM_SMALL
    }

    PDUID_PDU = "pduid_pdu" {
        cache_disp: CacheDisp::SharedWith("eventid_outlierpdu"),
        block_size: 2048,
        index_size: 512,
        ..descriptor::SEQUENTIAL
    }

    PRESENCEID_PRESENCE = "presenceid_presence" {
        ..descriptor::SEQUENTIAL_SMALL
    }

    PUBLICROOMIDS = "publicroomids" {
        ..descriptor::RANDOM_SMALL
    }

    PUSHKEY_DEVICEID = "pushkey_deviceid" {
        ..descriptor::RANDOM_SMALL
    }

    READRECEIPTID_READRECEIPT = "readreceiptid_readreceipt" {
        ..descriptor::RANDOM
    }

    REFERENCEDEVENTS = "referencedevents" {
        ..descriptor::RANDOM
    }

    REGISTRATIONTOKEN_INFO = "registrationtoken_info" {
        ..descriptor::RANDOM_SMALL
    }

    REPORTID_REPORT = "reportid_report" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMID_INVITEDCOUNT = "roomid_invitedcount" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMID_INVITEVIASERVERS = "roomid_inviteviaservers" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMID_JOINEDCOUNT = "roomid_joinedcount" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMID_PDULEAVES = "roomid_pduleaves" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMID_SHORTROOMID = "roomid_shortroomid" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMID_SHORTSTATEHASH = "roomid_shortstatehash" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMSERVERIDS = "roomserverids" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMSYNCTOKEN_SHORTSTATEHASH = "roomsynctoken_shortstatehash" {
        file_shape: 3,
        block_size: 512,
        compression_level: 3,
        bottommost_level: Some(6),
        ..descriptor::SEQUENTIAL
    }

    ROOMUSERDATAID_ACCOUNTDATA = "roomuserdataid_accountdata" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMUSERID_INVITECOUNT = "roomuserid_invitecount" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMUSERID_JOINED = "roomuserid_joined" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMUSERID_KNOCKEDCOUNT = "roomuserid_knockedcount" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMUSERID_LASTNOTIFICATIONREAD = "roomuserid_lastnotificationread" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMUSERID_LASTPRIVATEREADUPDATE = "roomuserid_lastprivatereadupdate" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMUSERID_LEFTCOUNT = "roomuserid_leftcount" {
        ..descriptor::RANDOM
    }

    ROOMUSERID_PRIVATEREAD = "roomuserid_privateread" {
        ..descriptor::RANDOM_SMALL
    }

    ROOMUSERONCEJOINEDIDS = "roomuseroncejoinedids" {
        ..descriptor::RANDOM
    }

    ROOMUSERTYPE_ROOMUSERDATAID = "roomusertype_roomuserdataid" {
        ..descriptor::RANDOM_SMALL
    }

    SENDERKEY_PUSHER = "senderkey_pusher" {
        ..descriptor::RANDOM_SMALL
    }

    SERVER_SIGNINGKEYS = "server_signingkeys" {
        ..descriptor::RANDOM
    }

    SERVERCURRENTEVENT_DATA = "servercurrentevent_data" {
        ..descriptor::RANDOM_SMALL
    }

    SERVERNAME_DESTINATION = "servername_destination" {
        ..descriptor::RANDOM_SMALL_CACHE
    }

    SERVERNAME_EDUCOUNT = "servername_educount" {
        ..descriptor::RANDOM_SMALL
    }

    SERVERNAME_OVERRIDE = "servername_override" {
        ..descriptor::RANDOM_SMALL_CACHE
    }

    SERVERNAME_STATUS = "servername_status" {
        ..descriptor::RANDOM_SMALL_CACHE
    }

    SERVERNAMEEVENT_DATA = "servernameevent_data" {
        cache_disp: CacheDisp::Unique,
        ..descriptor::RANDOM
    }

    SERVERROOMIDS = "serverroomids" {
        ..descriptor::RANDOM_SMALL
    }

    SHORTEVENTID_AUTHCHAIN = "shorteventid_authchain" {
        cache_disp: CacheDisp::Unique,
        ..descriptor::SEQUENTIAL
    }

    SHORTEVENTID_EVENTID = "shorteventid_eventid" {
        cache_disp: CacheDisp::Unique,
        ..descriptor::SEQUENTIAL_SMALL
    }

    SHORTEVENTID_SHORTSTATEHASH = "shorteventid_shortstatehash" {
        block_size: 512,
        index_size: 512,
        ..descriptor::SEQUENTIAL
    }

    SHORTSTATEHASH_STATEDIFF = "shortstatehash_statediff" {
        ..descriptor::SEQUENTIAL_SMALL
    }

    SHORTSTATEKEY_STATEKEY = "shortstatekey_statekey" {
        cache_disp: CacheDisp::Unique,
        ..descriptor::RANDOM_SMALL
    }

    SOFTFAILEDEVENTIDS = "softfailedeventids" {
        ..descriptor::RANDOM_SMALL
    }

    SPENTREFRESH_USERDEVICEID = "spentrefresh_userdeviceid" {
        ..descriptor::RANDOM_SMALL
    }

    STATEHASH_SHORTSTATEHASH = "statehash_shortstatehash" {
        ..descriptor::RANDOM
    }

    STATEKEY_SHORTSTATEKEY = "statekey_shortstatekey" {
        cache_disp: CacheDisp::Unique,
        ..descriptor::RANDOM
    }

    THREADID_USERIDS = "threadid_userids" {
        ..descriptor::SEQUENTIAL_SMALL
    }

    THREEPIDSID_PENDING = "threepidsid_pending" {
        ttl: 60 * 60 * 24, // pending validation session; minutes to complete
        ..descriptor::RANDOM_SMALL_CACHE
    }

    TIMEREDACTED_EVENTID = "timeredacted_eventid" {
        ..descriptor::SEQUENTIAL_SMALL
    }

    TODEVICEID_EVENTS = "todeviceid_events" {
        ..descriptor::RANDOM
    }

    TOFROM_RELATION = "tofrom_relation" {
        ..descriptor::RANDOM_SMALL
    }

    TOKEN_USERDEVICEID = "token_userdeviceid" {
        ..descriptor::RANDOM_SMALL
    }

    TOKENIDS = "tokenids" {
        block_size: 512,
        ..descriptor::RANDOM
    }

    URL_PREVIEWS = "url_previews" {
        ..descriptor::RANDOM
    }

    USERDEVICEID_METADATA = "userdeviceid_metadata" {
        ..descriptor::RANDOM_SMALL
    }

    USERDEVICEID_REFRESH = "userdeviceid_refresh" {
        ..descriptor::RANDOM_SMALL
    }

    USERDEVICEID_SPENTREFRESH = "userdeviceid_spentrefresh" {
        ..descriptor::RANDOM_SMALL
    }

    USERDEVICEID_TOKEN = "userdeviceid_token" {
        ..descriptor::RANDOM_SMALL
    }

    USERDEVICESESSIONID_THREEPID = "userdevicesessionid_threepid" {
        ttl: 60 * 60 * 24, // interactive-auth session; minutes to complete
        ..descriptor::RANDOM_SMALL_CACHE
    }

    USERDEVICESESSIONID_UIAAINFO = "userdevicesessionid_uiaainfo" {
        ..descriptor::RANDOM_SMALL
    }

    USERDEVICETXNID_RESPONSE = "userdevicetxnid_response" {
        ..descriptor::RANDOM_SMALL
    }

    USERFILTERID_FILTER = "userfilterid_filter" {
        ..descriptor::RANDOM_SMALL
    }

    USERID_AVATARURL = "userid_avatarurl" {
        ..descriptor::RANDOM_SMALL
    }

    USERID_BLURHASH = "userid_blurhash" {
        ..descriptor::RANDOM_SMALL
    }

    USERID_DEVICELISTVERSION = "userid_devicelistversion" {
        ..descriptor::RANDOM_SMALL
    }

    USERID_DISPLAYNAME = "userid_displayname" {
        ..descriptor::RANDOM_SMALL
    }

    USERID_EMAIL = "userid_email" {
        ..descriptor::RANDOM_SMALL
    }

    USERID_LASTONETIMEKEYUPDATE = "userid_lastonetimekeyupdate" {
        ..descriptor::RANDOM_SMALL
    }

    USERID_MASTERKEYID = "userid_masterkeyid" {
        ..descriptor::RANDOM_SMALL
    }

    USERID_OAUTHID = "userid_oauthid" {
        ..descriptor::RANDOM_SMALL
    }

    USERID_PASSWORD = "userid_password" {
        ..descriptor::RANDOM
    }

    USERID_PRESENCEID = "userid_presenceid" {
        ..descriptor::RANDOM_SMALL
    }

    USERID_SELFSIGNINGKEYID = "userid_selfsigningkeyid" {
        ..descriptor::RANDOM_SMALL
    }

    USERID_USERSIGNINGKEYID = "userid_usersigningkeyid" {
        ..descriptor::RANDOM_SMALL
    }

    USERIDPROFILEKEY_VALUE = "useridprofilekey_value" {
        ..descriptor::RANDOM_SMALL
    }

    USERROOMID_HIGHLIGHTCOUNT = "userroomid_highlightcount" {
        ..descriptor::RANDOM
    }

    USERROOMID_INVITESTATE = "userroomid_invitestate" {
        ..descriptor::RANDOM_SMALL
    }

    USERROOMID_JOINED = "userroomid_joined" {
        ..descriptor::RANDOM
    }

    USERROOMID_KNOCKEDSTATE = "userroomid_knockedstate" {
        ..descriptor::RANDOM_SMALL
    }

    USERROOMID_LEFTSTATE = "userroomid_leftstate" {
        ..descriptor::RANDOM
    }

    USERROOMID_NOTIFICATIONCOUNT = "userroomid_notificationcount" {
        ..descriptor::RANDOM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn no_column_takes_the_shared_cache_name() {
        assert!(
            !MAPS
                .iter()
                .any(|desc| desc.name == crate::engine::Context::SHARED_CACHE),
            "a column may not be named for the shared cache"
        );
    }

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
