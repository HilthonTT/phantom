use ruma::RoomVersionId;

use super::{Error, Result};

#[derive(Debug)]
#[allow(clippy::exhaustive_enums)]
pub enum RoomDisposition {
    Stable,

    Unstable,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum EventFormatVersion {
    V1,

    V2,

    V3,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum StateResolutionVersion {
    V1,

    V2,
}

#[non_exhaustive]
#[allow(clippy::struct_excessive_bools)]
pub struct RoomVersion {
    pub disposition: RoomDisposition,

    pub event_format: EventFormatVersion,

    pub state_res: StateResolutionVersion,
    pub enforce_key_validity: bool,

    pub special_case_aliases_auth: bool,

    pub strict_canonicaljson: bool,

    pub limit_notifications_power_levels: bool,

    pub extra_redaction_checks: bool,

    pub allow_knocking: bool,

    pub restricted_join_rules: bool,

    pub knock_restricted_join_rule: bool,

    pub integer_power_levels: bool,

    pub use_room_create_sender: bool,

    /// Room creators hold an infinite power level (MSC4289).
    pub explicitly_privilege_room_creators: bool,

    /// `additional_creators` in the create event's content names more creators.
    pub additional_room_creators: bool,

    /// The room ID is the create event's ID with a `!` sigil, the create event
    /// carries no `room_id`, and no event lists it in its `auth_events` (MSC4291).
    pub room_create_event_id_as_room_id: bool,

    /// State resolution v2.1: start the iterative auth checks from an empty
    /// state map and add the conflicted state subgraph to the full conflicted
    /// set (MSC4297).
    pub state_res_v2_1: bool,
}

impl RoomVersion {
    pub const V1: Self = Self {
        disposition: RoomDisposition::Stable,
        event_format: EventFormatVersion::V1,
        state_res: StateResolutionVersion::V1,
        enforce_key_validity: false,
        special_case_aliases_auth: true,
        strict_canonicaljson: false,
        limit_notifications_power_levels: false,
        extra_redaction_checks: true,
        allow_knocking: false,
        restricted_join_rules: false,
        knock_restricted_join_rule: false,
        integer_power_levels: false,
        use_room_create_sender: false,
        explicitly_privilege_room_creators: false,
        additional_room_creators: false,
        room_create_event_id_as_room_id: false,
        state_res_v2_1: false,
    };
    pub const V10: Self = Self {
        knock_restricted_join_rule: true,
        integer_power_levels: true,
        ..Self::V9
    };
    pub const V11: Self = Self {
        use_room_create_sender: true,
        ..Self::V10
    };
    pub const V12: Self = Self {
        explicitly_privilege_room_creators: true,
        additional_room_creators: true,
        room_create_event_id_as_room_id: true,
        state_res_v2_1: true,
        ..Self::V11
    };
    pub const V2: Self = Self {
        state_res: StateResolutionVersion::V2,
        ..Self::V1
    };
    pub const V3: Self = Self {
        event_format: EventFormatVersion::V2,
        extra_redaction_checks: false,
        ..Self::V2
    };
    pub const V4: Self = Self {
        event_format: EventFormatVersion::V3,
        ..Self::V3
    };
    pub const V5: Self = Self {
        enforce_key_validity: true,
        ..Self::V4
    };
    pub const V6: Self = Self {
        special_case_aliases_auth: false,
        strict_canonicaljson: true,
        limit_notifications_power_levels: true,
        ..Self::V5
    };
    pub const V7: Self = Self {
        allow_knocking: true,
        ..Self::V6
    };
    pub const V8: Self = Self {
        restricted_join_rules: true,
        ..Self::V7
    };
    pub const V9: Self = Self::V8;

    pub fn new(version: &RoomVersionId) -> Result<Self> {
        Ok(match version {
            RoomVersionId::V1 => Self::V1,
            RoomVersionId::V2 => Self::V2,
            RoomVersionId::V3 => Self::V3,
            RoomVersionId::V4 => Self::V4,
            RoomVersionId::V5 => Self::V5,
            RoomVersionId::V6 => Self::V6,
            RoomVersionId::V7 => Self::V7,
            RoomVersionId::V8 => Self::V8,
            RoomVersionId::V9 => Self::V9,
            RoomVersionId::V10 => Self::V10,
            RoomVersionId::V11 => Self::V11,
            RoomVersionId::V12 => Self::V12,
            ver => return Err(Error::Unsupported(format!("found version `{ver}`"))),
        })
    }

    pub fn supported() -> impl Iterator<Item = RoomVersionId> {
        [
            RoomVersionId::V1,
            RoomVersionId::V2,
            RoomVersionId::V3,
            RoomVersionId::V4,
            RoomVersionId::V5,
            RoomVersionId::V6,
            RoomVersionId::V7,
            RoomVersionId::V8,
            RoomVersionId::V9,
            RoomVersionId::V10,
            RoomVersionId::V11,
            RoomVersionId::V12,
        ]
        .into_iter()
    }

    #[must_use]
    pub fn is_supported(version: &RoomVersionId) -> bool {
        Self::new(version).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use ruma::RoomVersionId;

    use super::RoomVersion;

    #[test]
    fn every_listed_version_is_constructible() {
        assert!(RoomVersion::supported().all(|version| RoomVersion::is_supported(&version)));
    }

    #[test]
    fn version_12_is_supported() {
        assert!(RoomVersion::is_supported(&RoomVersionId::V12));
        assert!(RoomVersion::supported().any(|version| version == RoomVersionId::V12));
    }

    #[test]
    fn an_unlisted_version_is_not_supported() {
        let unstable = RoomVersionId::try_from("org.example.unstable").unwrap();
        assert!(!RoomVersion::is_supported(&unstable));
    }

    #[test]
    fn version_12_flags_match_ruma_rules() {
        let ours = RoomVersion::V12;
        let rules = RoomVersionId::V12.rules().unwrap();
        assert_eq!(
            ours.explicitly_privilege_room_creators,
            rules.authorization.explicitly_privilege_room_creators
        );
        assert_eq!(
            ours.additional_room_creators,
            rules.authorization.additional_room_creators
        );
        assert_eq!(
            ours.room_create_event_id_as_room_id,
            rules.authorization.room_create_event_id_as_room_id
        );
        assert!(ours.state_res_v2_1);
        assert!(!RoomVersion::V11.state_res_v2_1);
    }
}
