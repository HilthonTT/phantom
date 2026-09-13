use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Rendezvous {
    #[serde(default = "true_fn")]
    pub rendezvous_enabled: bool,

    #[serde(default = "default_rendezvous_session_max_bytes")]
    pub rendezvous_session_max_bytes: usize,

    #[serde(default = "default_rendezvous_session_ttl")]
    pub rendezvous_session_ttl: u64,

    #[serde(default = "default_rendezvous_max_sessions")]
    pub rendezvous_max_sessions: usize,

    #[serde(default = "true_fn")]
    pub rendezvous_authenticated_only: bool,

    #[serde(default = "default_rendezvous_rc_per_second")]
    pub rendezvous_rc_per_second: u32,

    #[serde(default = "default_rendezvous_rc_burst_count")]
    pub rendezvous_rc_burst_count: u32,
}
