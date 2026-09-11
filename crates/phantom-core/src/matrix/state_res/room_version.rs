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
            ver => return Err(Error::Unsupported(format!("found version `{ver}`"))),
        })
    }
}
