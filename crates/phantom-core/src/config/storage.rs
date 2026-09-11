use super::prelude::*;

#[derive(Clone, Debug, Default, Deserialize)]
pub enum StorageProvider {
    #[expect(non_camel_case_types)]
    local(StorageProviderLocal),

    #[expect(non_camel_case_types)]
    #[serde(rename = "s3", alias = "S3")]
    s3(Box<StorageProviderS3>),

    #[default]
    None,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[config_example_generator(
    filename = "phantom-example.toml",
    section = "global.storage_provider.<ID>.local"
)]
pub struct StorageProviderLocal {
    #[serde(alias = "path")]
    pub base_path: String,

    #[serde(default)]
    pub create_if_missing: bool,

    #[serde(default = "true_fn")]
    pub delete_empty_directories: bool,

    #[serde(default = "true_fn")]
    pub startup_check: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[config_example_generator(
    filename = "phantom-example.toml",
    section = "global.storage_provider.<ID>.s3"
)]
pub struct StorageProviderS3 {
    pub url: Option<String>,

    pub bucket: Option<String>,

    pub region: Option<String>,

    #[doc = "display: sensitive"]
    pub key: Option<String>,

    #[doc = "display: sensitive"]
    #[serde(skip_serializing)]
    pub secret: Option<String>,

    #[serde(alias = "path")]
    pub base_path: Option<String>,

    pub endpoint: Option<String>,

    #[serde(default)]
    pub use_vhost_request: Option<bool>,

    #[doc = "display: sensitive"]
    #[serde(skip_serializing)]
    pub token: Option<String>,

    #[doc = "display: sensitive"]
    pub kms: Option<String>,

    pub use_bucket_key: Option<bool>,

    #[serde(default = "default_multipart_threshold")]
    pub multipart_threshold: ByteSize,

    #[serde(default = "default_multipart_part_size")]
    pub multipart_part_size: ByteSize,

    #[serde(default = "some_true_fn")]
    pub use_https: Option<bool>,

    #[serde(default = "some_true_fn")]
    pub use_signatures: Option<bool>,

    #[serde(default = "some_true_fn")]
    pub use_payload_signatures: Option<bool>,

    #[serde(default = "true_fn")]
    pub startup_check: bool,
}
