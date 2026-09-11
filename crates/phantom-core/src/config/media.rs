//! Uploaded files, thumbnails, and URL previews.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Media {
    /// Path to the directory holding uploaded and cached media files.
    ///
    /// Media is kept as files rather than in the database because it is large,
    /// written once and read whole — everything a filesystem is better at than
    /// a key-value store. What the database holds is the metadata pointing at
    /// them, so the two must be backed up and restored together.
    ///
    /// Leave this unset to use a `media` directory under `database_path`.
    ///
    /// example: "/var/lib/phantom/media"
    pub media_path: Option<PathBuf>,

    /// Check at startup that every file the media metadata names is present.
    ///
    /// Answers a request for media that is not there with "gone" rather than
    /// with an error, which is what a client can act on. The check reads one
    /// directory entry per stored file, so an installation with a very large
    /// media store may prefer to turn it off and accept the worse error.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub media_startup_check: bool,

    /// Largest picture, in pixels, the thumbnailer will decode.
    ///
    /// Dimensions cost memory whatever the encoded file weighs — a few
    /// kilobytes of PNG can declare a picture that costs gigabytes to hold —
    /// so a picture declaring more than this is served without a thumbnail
    /// rather than decoded. A frame extracted from a video inherits the
    /// resolution of the video it came from and is bounded here too.
    ///
    /// 50 megapixels is roughly four 8K frames, and more than any ordinary
    /// camera produces. Each pixel is budgeted four bytes, so the default
    /// admits a decode of about 200 MiB. The budget is per decode in flight,
    /// which is what to size it against: thumbnail requests are not otherwise
    /// limited in number.
    ///
    /// default: 50000000
    #[serde(default = "default_media_thumbnail_max_pixels")]
    pub media_thumbnail_max_pixels: u64,

    /// Program that extracts a still frame from a video, so that a video can
    /// be given a thumbnail. Phantom decodes no video itself; the frame the
    /// program writes is thumbnailed as an ordinary picture and cached as an
    /// ordinary thumbnail.
    ///
    /// The list is an argument vector whose first entry is the program and
    /// whose remaining entries are its arguments. It is executed directly and
    /// never through a shell, so no quoting or expansion applies. Every
    /// argument has these tokens substituted before each call:
    ///
    /// - `{input}` the path of a temporary file holding the source video.
    /// - `{width}` and `{height}` the requested thumbnail dimensions.
    ///
    /// The program writes one frame to standard output in any format the
    /// thumbnailer decodes: PNG, JPEG, WebP or GIF. While the list is empty,
    /// videos are served without a thumbnail.
    ///
    /// example: [
    /// "ffmpeg", "-loglevel", "error", "-i", "{input}", "-vf", "thumbnail",
    /// "-frames:v", "1", "-f", "image2pipe", "-c:v", "mjpeg", "pipe:1",
    /// ]
    ///
    /// default: []
    #[serde(default)]
    pub media_video_thumbnail_command: Vec<String>,

    /// List of storage providers to use for media. Providers can be configured
    /// below in respective sections designated by
    /// `global.storage_provider.<NAME>.<brand>` where `NAME` can be listed
    /// here.
    ///
    /// For advanced features and future extensions involving multiple providers
    /// the list may contain multiple entries. You MUST take note of other
    /// configuration options when listing multiple providers or resource
    /// duplication costs and poor performance can result.
    ///
    /// The list defaults to `["media"]` which is an implicit storage provider
    /// representing the media directory on the local filesystem. It can be
    /// altered by configuring `global.storage_provider.media.local` explicitly
    /// or disabled by omitting it from this list entirely. Users with existing
    /// deployments are advised to continue listing "media" as a fallback along
    /// with their new provider.
    ///
    /// reloadable: yes
    /// default: ["media"]
    #[serde(default = "default_media_storage_providers")]
    pub media_storage_providers: BTreeSet<String>,

    /// Seconds a video thumbnail request may spend extracting its frame.
    ///
    /// One deadline spans the wait for a free slot, staging the video and the
    /// program itself, so a queue cannot compound it into a multiple of what
    /// is configured here. On expiry the program and anything it spawned are
    /// killed and the video is served without a thumbnail.
    ///
    /// default: 30
    #[serde(default = "default_media_video_thumbnail_timeout")]
    pub media_video_thumbnail_timeout: u64,

    /// Frame extractions permitted to run at once.
    ///
    /// Decoding video costs far more than scaling a picture, so requests past
    /// this limit wait for a slot rather than piling load onto the host. A
    /// slot is held from staging the video through to the program exiting, so
    /// this also bounds how many staged videos occupy the staging directory at
    /// once. Raise it where cores are spare.
    ///
    /// default: 1
    #[serde(default = "default_media_video_thumbnail_concurrency")]
    pub media_video_thumbnail_concurrency: usize,

    /// Largest video, in bytes, staged for the thumbnail program, and largest
    /// frame read back from it.
    ///
    /// A video past this is served without a thumbnail rather than written
    /// out, and a frame past it is refused rather than decoded from what would
    /// be a truncation.
    ///
    /// default: 134217728
    #[serde(default = "default_media_video_thumbnail_max_size")]
    pub media_video_thumbnail_max_size: usize,

    /// Directory a video is staged in for the thumbnail program to read.
    ///
    /// One file per running program, removed as soon as it exits, and any left
    /// behind by a killed server are reclaimed at startup. Leave this unset to
    /// use a `tmp` directory under `database_path`, which keeps videos off the
    /// memory-backed `/tmp` a service manager commonly provides.
    ///
    /// example: "/var/tmp/phantom"
    pub media_video_thumbnail_path: Option<PathBuf>,

    /// Media IDs one user may hold reserved but unfilled at a time.
    ///
    /// A client may ask for a media ID before it has the file, so that it can
    /// send the message naming it first. Each reservation is a promise to
    /// serve something at that URI, so a user cannot hold an unbounded number
    /// of them.
    ///
    /// default: 5
    #[serde(default = "default_max_pending_media_uploads")]
    pub max_pending_media_uploads: usize,

    /// Seconds a reserved media ID stays fillable before it expires.
    ///
    /// Past this the reservation is refused rather than filled, and the ID
    /// stays permanently unresolvable: a client that reserves one and loses
    /// the file must reserve another.
    ///
    /// default: 86400
    #[serde(default = "default_media_create_unused_expiration_time")]
    pub media_create_unused_expiration_time: u64,

    /// Media IDs a user may reserve per second, sustained.
    ///
    /// Reserving one costs nothing but the record, which is what makes it
    /// worth rate limiting separately from the upload it precedes. Zero
    /// disables the limit.
    ///
    /// default: 10
    #[serde(default = "default_media_rc_create_per_second")]
    pub media_rc_create_per_second: u32,

    /// Media ID reservations a user may make in a burst.
    ///
    /// The allowance refills at `media_rc_create_per_second`, so this is what
    /// a client may spend at once after a quiet period. Zero disables the
    /// limit.
    ///
    /// default: 50
    #[serde(default = "default_media_rc_create_burst_count")]
    pub media_rc_create_burst_count: u32,

    /// Domains phantom may fetch URL previews from, matched as a substring of
    /// the URL's host.
    ///
    /// "google.com" matches `https://google.com` and also
    /// `http://notgoogle.com.example`, so prefer
    /// `url_preview_domain_explicit_allowlist` where you can. "*" allows
    /// every domain, which lets any user aim this server at any host on its
    /// network.
    ///
    /// default: []
    #[serde(default)]
    pub url_preview_domain_contains_allowlist: Vec<String>,

    /// Domains phantom may fetch URL previews from, matched exactly.
    ///
    /// "google.com" matches `https://google.com` but not
    /// `https://notgoogle.com.example`. See
    /// `url_preview_check_root_domain` for matching subdomains too.
    ///
    /// default: []
    #[serde(default)]
    pub url_preview_domain_explicit_allowlist: Vec<String>,

    /// Domains phantom may never fetch URL previews from, matched exactly.
    /// Checked before either allowlist, so it always wins.
    ///
    /// default: []
    #[serde(default)]
    pub url_preview_domain_explicit_denylist: Vec<String>,

    /// URLs phantom may fetch previews from, matched as a substring of the
    /// whole URL rather than of its host.
    ///
    /// This matches anywhere in the URL, so "google.com" also matches
    /// `https://example.invalid/google.com`. "*" allows every URL.
    ///
    /// default: []
    #[serde(default)]
    pub url_preview_url_contains_allowlist: Vec<String>,

    /// Bytes of a page phantom reads before giving up on finding its preview
    /// metadata.
    ///
    /// default: 256000
    #[serde(default = "default_url_preview_max_spider_size")]
    pub url_preview_max_spider_size: usize,

    /// Bytes of a single media item phantom will fetch or relay for a URL
    /// preview: the `og:image` it measures, and the file behind each
    /// `mxc://` a preview hands out.
    ///
    /// A file whose advertised length is over this is not registered at all,
    /// so a client is never given a URI the relay is bound to refuse.
    ///
    /// default: 52428800
    #[serde(default = "default_url_preview_max_media_size")]
    pub url_preview_max_media_size: usize,

    /// Seconds a fetched URL preview stays cached, an empty one included.
    ///
    /// Raising it spares the origins named in a room whose history is read
    /// often, at the cost of serving metadata a page has since changed. Zero
    /// fetches on every request.
    ///
    /// default: 86400
    #[serde(default = "default_url_preview_cache_ttl")]
    pub url_preview_cache_ttl: u64,

    /// `User-Agent` phantom sends when fetching a page to read its OpenGraph
    /// tags. Unset sends the ordinary server agent.
    ///
    /// Some origins serve those tags only to an agent they recognise as a
    /// link-preview crawler, and serve everyone else a page whose tags sit
    /// past `url_preview_max_spider_size`.
    ///
    /// default:
    #[serde(default)]
    pub url_preview_user_agent: Option<String>,

    /// `User-Agent` phantom sends when fetching the media a preview names —
    /// `og:image`, `og:video`, `og:audio`, and a URL that is itself a file —
    /// as opposed to the page they appear on. Unset falls back to
    /// `url_preview_user_agent`.
    ///
    /// Setting this is also what makes a page-agent rejection non-final: a
    /// URL the page client is refused is retried with this one, since an
    /// origin that gates its pages often does not gate its files.
    ///
    /// default:
    #[serde(default)]
    pub url_preview_media_user_agent: Option<String>,

    /// Apply the domain allowlists to a URL's root domain rather than to the
    /// host it names, so that allowing "wikipedia.org" also allows
    /// "en.m.wikipedia.org". Does not affect
    /// `url_preview_url_contains_allowlist`.
    #[serde(default)]
    pub url_preview_check_root_domain: bool,

    /// Address, or the name of a network interface, that URL preview requests
    /// are sent from. Unset lets the operating system pick.
    ///
    /// Interface names work on Linux, Android and Fuchsia; elsewhere only an
    /// address is accepted, and a name is rejected at startup.
    ///
    /// example: "eth0" or "1.2.3.4"
    ///
    /// default:
    #[serde(default, with = "either::serde_untagged_optional")]
    pub url_preview_bound_interface: Option<Either<IpAddr, String>>,
}
