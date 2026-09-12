use std::time::{Duration, SystemTime};

use phantom_core::{Result, time::timepoint_from_now};
use serde::{Deserialize, Serialize};
use smallstr::SmallString;

type MediaType = SmallString<[u8; 32]>;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct UrlPreviewData {
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "og:title")]
    pub title: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "og:description"
    )]
    pub description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none", rename = "og:image")]
    pub image: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "matrix:image:size"
    )]
    pub image_size: Option<usize>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "og:image:width"
    )]
    pub image_width: Option<u32>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "og:image:height"
    )]
    pub image_height: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none", rename = "og:video")]
    pub video: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "og:video:type"
    )]
    pub video_type: Option<MediaType>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "matrix:video:size"
    )]
    pub video_size: Option<usize>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "og:video:width"
    )]
    pub video_width: Option<u32>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "og:video:height"
    )]
    pub video_height: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none", rename = "og:audio")]
    pub audio: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "matrix:audio:size"
    )]
    pub audio_size: Option<usize>,

    #[serde(default, skip_serializing_if = "Option::is_none", rename = "og:type")]
    pub og_type: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none", rename = "og:url")]
    pub og_url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(in crate::media) struct CachedPreview {
    pub(in crate::media) preview: UrlPreviewData,
    pub(in crate::media) expires: SystemTime,
}

impl CachedPreview {
    pub(super) fn new(ttl: Duration, preview: UrlPreviewData) -> Result<Self> {
        timepoint_from_now(ttl).map(|expires| Self { preview, expires })
    }

    #[inline]
    #[must_use]
    pub(in crate::media) fn is_expired(&self) -> bool {
        self.expires <= SystemTime::now()
    }

    #[inline]
    #[must_use]
    pub(in crate::media) fn is_valid(&self) -> bool {
        !self.is_expired()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use phantom_core::time::timepoint_ago;

    use super::{CachedPreview, UrlPreviewData};

    const TTL: Duration = Duration::from_secs(60 * 60 * 24);

    fn sample() -> UrlPreviewData {
        UrlPreviewData {
            title: Some("Title".to_owned()),
            description: Some("Description".to_owned()),
            image: Some("mxc://example.org/image".to_owned()),
            image_size: Some(0xFF01),
            image_width: Some(640),
            image_height: Some(0xFF),
            video: Some("mxc://example.org/video".to_owned()),
            video_type: Some("video/mp4".into()),
            video_size: Some(123_456),
            video_width: Some(1920),
            video_height: Some(1080),
            audio: Some("mxc://example.org/audio".to_owned()),
            audio_size: Some(4096),
            og_type: Some("website".to_owned()),
            og_url: Some("https://example.org/".to_owned()),
        }
    }

    #[test]
    fn preview_wire_keys_unchanged() {
        let value = serde_json::to_value(sample()).expect("json");
        let object = value.as_object().expect("object");

        assert!(object.contains_key("og:title"));
        assert!(object.contains_key("matrix:image:size"));
        assert!(object.contains_key("og:video:width"));
        assert!(object.contains_key("og:video:type"));
        assert!(object.contains_key("og:url"));
        assert!(!object.contains_key("title"));

        let empty = serde_json::to_value(UrlPreviewData::default()).expect("json");

        assert!(
            empty.as_object().expect("object").is_empty(),
            "a preview that found nothing carries no keys at all"
        );
    }

    #[test]
    fn cached_preview_expiry() {
        let cached = CachedPreview::new(TTL, UrlPreviewData::default()).expect("representable");

        assert!(cached.is_valid());
        assert!(!cached.is_expired());

        let expired = CachedPreview {
            preview: UrlPreviewData::default(),
            expires: timepoint_ago(Duration::from_secs(1)).expect("representable"),
        };

        assert!(expired.is_expired());
        assert!(!expired.is_valid());
    }

    #[test]
    fn cached_preview_honors_the_configured_lifetime() {
        let day = CachedPreview::new(TTL, UrlPreviewData::default()).expect("representable");
        let month = CachedPreview::new(TTL * 30, UrlPreviewData::default()).expect("representable");

        assert!(month.expires > day.expires);
    }

    #[test]
    fn cached_preview_unrepresentable_lifetime_refused() {
        let refused = CachedPreview::new(Duration::from_secs(u64::MAX), UrlPreviewData::default());

        assert!(
            refused.is_err(),
            "a lifetime past representable time errors rather than panics"
        );
    }
}
