//! Where media objects are stored.
//!
//! Opens its own TOML section rather than continuing `[global]`, so it is
//! declared after every module that does continue it.

use super::prelude::*;

/// Selects the backend for a named media storage provider.
///
/// Local providers store objects beneath a filesystem path, while S3 providers
/// use a compatible object store. The default variant disables the entry.
#[derive(Clone, Debug, Default, Deserialize)]
pub enum StorageProvider {
    /// Selects a local filesystem backend.
    ///
    /// The contained settings root object paths beneath a configured directory.
    /// Startup checks can require that directory to be usable.
    #[expect(non_camel_case_types)]
    local(StorageProviderLocal),

    /// Selects an S3-compatible object storage backend.
    ///
    /// The boxed settings configure endpoint, credentials, encryption, and
    /// multipart uploads. Custom endpoints permit compatible non-AWS services.
    #[expect(non_camel_case_types)]
    #[serde(rename = "s3", alias = "S3")]
    s3(Box<StorageProviderS3>),

    /// Disables this storage provider entry.
    ///
    /// This is the default when no backend variant is selected. It carries no
    /// backend settings.
    #[default]
    None,
}

/// Configures local filesystem object storage.
///
/// `base_path` prefixes every object path belonging to this provider. Remaining
/// options control directory creation, cleanup, and startup checks.
#[derive(Clone, Debug, Default, Deserialize)]
#[config_example_generator(
    filename = "phantom-example.toml",
    section = "global.storage_provider.<ID>.local"
)]
pub struct StorageProviderLocal {
    /// Absolute path to this local filesystem storage provider. Technically the
    /// provider exists at the filesystem root, and the base_path is prefixed to
    /// all objects.
    #[serde(alias = "path")]
    pub base_path: String,

    /// Creates the directory on the local filesystem if missing. This is not
    /// recommended to prevent misconfigured environments and missing mounts
    /// from silently succeeding.
    #[serde(default)]
    pub create_if_missing: bool,

    /// Toggles the preservation of a directory after its last file contents are
    /// removed.
    #[serde(default = "true_fn")]
    pub delete_empty_directories: bool,

    /// Enables checks performed at startup determining the usability of the
    /// local directory. Failures will abort the server's startup.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub startup_check: bool,
}

/// Configures an S3-compatible object storage provider.
///
/// Bucket, endpoint, and credential fields identify the remote store.
/// Transport, encryption, multipart, and startup options tune how objects are
/// accessed.
#[derive(Clone, Debug, Default, Deserialize)]
#[config_example_generator(
    filename = "phantom-example.toml",
    section = "global.storage_provider.<ID>.s3"
)]
pub struct StorageProviderS3 {
    /// Supply an s3 URL e.g. "s3://bucket/path". These URLs may contain one
    /// or all of `bucket`, `region`, and `path` . When not supplied, such
    /// additional items can be supplied below individually.
    pub url: Option<String>,

    /// The name of the S3 bucket. e.g. "bucketname-123456789-us-west-2-an".
    pub bucket: Option<String>,

    /// The region of the S3 bucket. e.g. "us-west-2".
    ///
    /// default: "us-east-1"
    pub region: Option<String>,

    /// Your amazon IAM Key ID with access granted to this bucket.
    /// e.g. "ABCDEFG1X1ZZYYXXWWVV"
    ///
    /// display: sensitive
    pub key: Option<String>,

    /// The secret key component which is approx 40 characters of base64.
    ///
    /// default:
    /// display: sensitive
    #[serde(skip_serializing)]
    pub secret: Option<String>,

    /// Optional path prefix within the bucket where all our operations will
    /// take place.
    #[serde(alias = "path")]
    pub base_path: Option<String>,

    /// (expert use) Override the location of s3 applied after components of the
    /// parsed `url` (or when none set).
    pub endpoint: Option<String>,

    /// (expert use) Override this property useful for some self-hosted
    /// environments. By default it is derived when parsing the primary `url`.
    #[serde(default)]
    pub use_vhost_request: Option<bool>,

    /// (expert use) Alternative session-token authentication method.
    ///
    /// display: sensitive
    /// default:
    #[serde(skip_serializing)]
    pub token: Option<String>,

    /// (expert use) Associated SSE-KMS key material.
    ///
    /// display: sensitive
    pub kms: Option<String>,

    /// (expert use) When configured for the bucket it should be reflected here.
    pub use_bucket_key: Option<bool>,

    /// (expert use) Threshold size for switching to Multi-part uploads. This is
    /// a quirk of the S3 protocol which requires us to use a different approach
    /// for "large" uploads. This value determines what a "large" upload is. The
    /// default value should be sufficient for most providers. The value is a
    /// parsed string allowing SI or IEC units for convenience.
    ///
    /// default: 100 MiB
    #[serde(default = "default_multipart_threshold")]
    pub multipart_threshold: ByteSize,

    /// (expert use) Size of each individual part within a Multi-part upload.
    /// Once an upload exceeds `multipart_threshold` the payload is split into
    /// parts of this size, each sent as a separate HTTP PUT. Smaller values
    /// keep individual requests under per-request timeouts on slow uplinks at
    /// the cost of more round-trips. S3 requires every part except the last
    /// to be at least 5 MiB. The value is a parsed string allowing SI or IEC
    /// units for convenience.
    ///
    /// default: 10 MiB
    #[serde(default = "default_multipart_part_size")]
    pub multipart_part_size: ByteSize,

    /// (developer use) Allows relaxing default requirement forcing HTTPS.
    ///
    /// default: true
    #[serde(default = "some_true_fn")]
    pub use_https: Option<bool>,

    /// (developer_use) Allows skipping request header signatures (will be
    /// reejected by AWS).
    ///
    /// default: true
    #[serde(default = "some_true_fn")]
    pub use_signatures: Option<bool>,

    /// (developer_use) Allows disabling request payload signatures.
    ///
    /// default: true
    #[serde(default = "some_true_fn")]
    pub use_payload_signatures: Option<bool>,

    /// (developer use) Enables checks performed at startup such as pinging the
    /// provider. Failures are considered critical startup errors which abort
    /// startup. When set to false, faulty providers are only discovered with
    /// first use and will not be fatal errors.
    ///
    /// Only set this to false if you expect a provider to be down at startup or
    /// for development/testing purposes; checks are disabled when the server
    /// is started in '--maintenance' mode.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub startup_check: bool,
}
