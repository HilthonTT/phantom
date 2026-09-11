use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Media {
    pub media_path: Option<PathBuf>,

    #[serde(default = "true_fn")]
    pub media_startup_check: bool,

    #[serde(default = "default_media_thumbnail_max_pixels")]
    pub media_thumbnail_max_pixels: u64,

    #[serde(default)]
    pub media_video_thumbnail_command: Vec<String>,

    #[serde(default = "default_media_storage_providers")]
    pub media_storage_providers: BTreeSet<String>,

    #[serde(default = "default_media_video_thumbnail_timeout")]
    pub media_video_thumbnail_timeout: u64,

    #[serde(default = "default_media_video_thumbnail_concurrency")]
    pub media_video_thumbnail_concurrency: usize,

    #[serde(default = "default_media_video_thumbnail_max_size")]
    pub media_video_thumbnail_max_size: usize,

    pub media_video_thumbnail_path: Option<PathBuf>,

    #[serde(default = "default_max_pending_media_uploads")]
    pub max_pending_media_uploads: usize,

    #[serde(default = "default_media_create_unused_expiration_time")]
    pub media_create_unused_expiration_time: u64,

    #[serde(default = "default_media_rc_create_per_second")]
    pub media_rc_create_per_second: u32,

    #[serde(default = "default_media_rc_create_burst_count")]
    pub media_rc_create_burst_count: u32,

    #[serde(default)]
    pub url_preview_domain_contains_allowlist: Vec<String>,

    #[serde(default)]
    pub url_preview_domain_explicit_allowlist: Vec<String>,

    #[serde(default)]
    pub url_preview_domain_explicit_denylist: Vec<String>,

    #[serde(default)]
    pub url_preview_url_contains_allowlist: Vec<String>,

    #[serde(default = "default_url_preview_max_spider_size")]
    pub url_preview_max_spider_size: usize,

    #[serde(default = "default_url_preview_max_media_size")]
    pub url_preview_max_media_size: usize,

    #[serde(default = "default_url_preview_cache_ttl")]
    pub url_preview_cache_ttl: u64,

    #[serde(default)]
    pub url_preview_user_agent: Option<String>,

    #[serde(default)]
    pub url_preview_media_user_agent: Option<String>,

    #[serde(default)]
    pub url_preview_check_root_domain: bool,

    #[serde(default, with = "either::serde_untagged_optional")]
    pub url_preview_bound_interface: Option<Either<IpAddr, String>>,
}
