use ruma::RoomVersionId;

use super::prelude::*;

/// Options for the client-server API.
#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Client {
    /// Allow standard users to set or clear their display names through the
    /// client profile API.
    ///
    /// Server admins and appservices are always allowed to change display
    /// names.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub enable_set_displayname: bool,

    /// Whether to enable login using the traditional user/password flow.
    ///
    /// Set this to false to allow logging in only through other mechanisms,
    /// such as OIDC.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub login_with_password: bool,

    /// Allow an existing session to mint a login token for another client.
    ///
    /// This requires interactive authentication, but a malicious client could
    /// use it to spawn more than one session.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub login_via_existing_session: bool,

    /// Whether the login token route accepts login tokens at all.
    ///
    /// Distinct from `login_via_existing_session`: leave this enabled while
    /// disabling that one to stop clients minting tokens without stopping
    /// the server from doing so.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub login_via_token: bool,

    /// Room version newly created rooms use when the client does not ask for
    /// one.
    ///
    /// default: "12"
    #[serde(default = "default_default_room_version")]
    pub default_room_version: RoomVersionId,

    /// Whether encrypted rooms and events are allowed.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub allow_encryption: bool,

    /// Whether locally created rooms are end-to-end encrypted by default.
    ///
    /// "all" encrypts every room, "invite" encrypts rooms created with the
    /// `private_chat` or `trusted_private_chat` presets; anything else has no
    /// effect.
    ///
    /// default: "none"
    #[serde(default)]
    pub encryption_enabled_by_default_for_room_type: Option<String>,

    /// Default power-level overrides merged into every room this server
    /// creates, before the client's own `power_level_content_override`.
    ///
    /// Top-level keys replace the computed defaults wholesale.
    ///
    /// default: unset
    #[serde(default)]
    pub default_power_level_content_override: Option<serde_json::Value>,

    /// Sets the default `m.federate` property for newly created rooms when
    /// the client does not request one. With `allow_federation` off, created
    /// rooms are never federated.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub federate_created_rooms: bool,

    /// Allow local presence updates and requests.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub allow_local_presence: bool,

    /// Minimum seconds a local client can indicate typing.
    ///
    /// default: 15
    #[serde(default = "default_typing_client_timeout_min_s")]
    pub typing_client_timeout_min_s: u64,

    /// Maximum seconds a local client can indicate typing.
    ///
    /// default: 45
    #[serde(default = "default_typing_client_timeout_max_s")]
    pub typing_client_timeout_max_s: u64,

    /// Minimum long-polling sync timeout in milliseconds; smaller requests
    /// are clamped up.
    ///
    /// default: 5000
    #[serde(default = "default_client_sync_timeout_min")]
    pub client_sync_timeout_min: u64,

    /// Long-polling sync timeout in milliseconds when the client does not
    /// request one.
    ///
    /// default: 30000
    #[serde(default = "default_client_sync_timeout_default")]
    pub client_sync_timeout_default: u64,

    /// Maximum long-polling sync timeout in milliseconds; larger requests
    /// are clamped down.
    ///
    /// default: 90000
    #[serde(default = "default_client_sync_timeout_max")]
    pub client_sync_timeout_max: u64,

    /// Whether sync computes room heroes. The spec mandates them; disable
    /// only to save resources on custom deployments.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub calculate_heroes: bool,

    /// Allow guest registrations.
    ///
    /// default: false
    #[serde(default)]
    pub allow_guest_registration: bool,

    /// Log guest registrations in the admin room.
    ///
    /// default: false
    #[serde(default)]
    pub log_guest_registrations: bool,

    /// Prevent local users other than server admins from sending redactions.
    ///
    /// default: false
    #[serde(default)]
    pub disable_local_redactions: bool,

    /// Maximum one-time keys stored per device.
    ///
    /// default: 256
    #[serde(default = "default_one_time_key_limit")]
    pub one_time_key_limit: usize,

    /// Only count encrypted rooms when deciding who receives device-list
    /// updates. Helps very large servers but may surprise clients.
    ///
    /// default: false
    #[serde(default)]
    pub device_key_update_encrypted_rooms_only: bool,

    /// Seconds a client-initiated `/keys/query` or `/keys/claim` waits on a
    /// remote server.
    ///
    /// default: 8
    #[serde(default = "default_federation_keys_timeout")]
    pub federation_keys_timeout: u64,

    /// Allow guest users to fetch TURN credentials.
    ///
    /// default: false
    #[serde(default)]
    pub turn_allow_guests: bool,

    /// Only allow admins to publish rooms to the room directory. Unpublishing
    /// stays open to everyone.
    ///
    /// default: false
    #[serde(default)]
    pub lockdown_public_room_directory: bool,

    /// Let room directory searches starting with '!' match partial room IDs.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub allow_public_room_search_by_id: bool,

    /// Let room ID searches match any joinable room rather than only rooms
    /// in the public directory.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub allow_unlisted_room_search_by_id: bool,

    /// Show all local users in the user directory, rather than only those in
    /// public rooms or sharing a room with the searcher.
    ///
    /// default: false
    #[serde(default)]
    pub show_all_local_users_in_user_directory: bool,

    /// Show appservice senders and users in exclusive appservice namespaces
    /// in the user directory.
    ///
    /// default: false
    #[serde(default)]
    pub show_appservice_users_in_user_directory: bool,

    /// Require authentication on the profile retrieval endpoints.
    ///
    /// default: false
    #[serde(default)]
    pub require_auth_for_profile_requests: bool,

    /// Restrict profile retrieval to the user themselves or users sharing a
    /// joined room. Appservices are exempt.
    ///
    /// default: false
    #[serde(default)]
    pub limit_profile_requests_to_users_who_share_rooms: bool,

    /// Maximum join attempts per client join request, each against a
    /// different resident server.
    ///
    /// default: 3
    #[serde(default = "default_max_join_attempts_per_join_request")]
    pub max_join_attempts_per_join_request: usize,

    /// Delete a room when the last local user leaves it. Experimental.
    ///
    /// default: false
    #[serde(default)]
    pub delete_rooms_after_leave: bool,

    /// Deactivate any local user who attempts to join a banned room, a
    /// forbidden alias, or a room on a forbidden server.
    ///
    /// default: false
    #[serde(default)]
    pub auto_deactivate_banned_room_attempts: bool,

    /// Allow users with the `redact` power level to request unredacted event
    /// content (MSC2815). Server admins always can.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub allow_room_admins_to_request_unredacted_events: bool,

    /// Fetch the base event of a `/context` request over federation when the
    /// server never received it, instead of returning 404.
    ///
    /// default: false
    #[serde(default)]
    pub fetch_unreceived_contexts_over_federation: bool,

    /// URL of the server's support page, served from
    /// `/.well-known/matrix/support`.
    ///
    /// default: unset
    pub well_known_support_page: Option<Url>,

    /// Role of the support contact, e.g. "m.role.admin".
    ///
    /// default: unset
    pub well_known_support_role: Option<String>,

    /// Email address of the support contact.
    ///
    /// default: unset
    pub well_known_support_email: Option<String>,

    /// Matrix ID of the support contact.
    ///
    /// default: unset
    pub well_known_support_mxid: Option<String>,
}
