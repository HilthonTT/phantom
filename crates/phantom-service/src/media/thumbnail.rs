#[cfg(feature = "media_thumbnail")]
use std::io::Cursor;
use std::{cmp::min, num::Saturating as Sat};

#[cfg(feature = "media_thumbnail")]
use image::{DynamicImage, ImageFormat, ImageReader, Limits, imageops::FilterType};
#[cfg(feature = "media_thumbnail")]
use phantom_core::{Err, debug, debug_warn};
use phantom_core::{Result, checked, err, implement};
#[cfg(feature = "media_thumbnail")]
use ruma::http_headers::ContentDispositionType;
use ruma::{MxcUri, UInt, http_headers::ContentDisposition, media::Method};

use super::{Dimensions, FileMeta, Service};

#[cfg(feature = "media_thumbnail")]
const PNG: &str = "image/png";

#[cfg(feature = "media_thumbnail")]
const BYTES_PER_PIXEL: u64 = 4;

#[cfg(feature = "media_thumbnail")]
const THUMBNAIL_NAME: &str = "thumbnail.png";

#[derive(Clone, Debug)]
pub struct Dim {
    pub width: u32,
    pub height: u32,
    pub method: Method,
}

impl Dim {
    #[inline]
    #[must_use]
    pub fn original() -> Self {
        Self::new(0, 0, None)
    }

    pub fn from_ruma(width: UInt, height: UInt, method: Option<Method>) -> Result<Self> {
        let width = width
            .try_into()
            .map_err(|e| err!(Request(InvalidParam("Width is invalid: {e:?}"))))?;

        let height = height
            .try_into()
            .map_err(|e| err!(Request(InvalidParam("Height is invalid: {e:?}"))))?;

        Ok(Self::new(width, height, method))
    }

    #[inline]
    #[must_use]
    pub fn new(width: u32, height: u32, method: Option<Method>) -> Self {
        Self {
            width,
            height,
            method: method.unwrap_or(Method::Scale),
        }
    }

    #[inline]
    #[must_use]
    pub fn dimensions(&self) -> Dimensions {
        Dimensions {
            width: self.width,
            height: self.height,
        }
    }

    #[inline]
    #[must_use]
    pub fn crop(&self) -> bool {
        self.method == Method::Crop
    }

    #[must_use]
    pub fn normalized(&self) -> Self {
        match (self.width, self.height) {
            (0..=32, 0..=32) => Self::new(32, 32, Some(Method::Crop)),
            (0..=96, 0..=96) => Self::new(96, 96, Some(Method::Crop)),
            (0..=320, 0..=240) => Self::new(320, 240, Some(Method::Scale)),
            (0..=640, 0..=480) => Self::new(640, 480, Some(Method::Scale)),
            (0..=800, 0..=600) => Self::new(800, 600, Some(Method::Scale)),
            _ => Self::original(),
        }
    }

    pub fn scaled(&self, source: &Self) -> Result<Self> {
        let source_width = source.width;
        let source_height = source.height;

        let width = min(self.width, source_width);
        let height = min(self.height, source_height);

        let use_width = Sat(width) * Sat(source_height) <= Sat(height) * Sat(source_width);

        let (x, y) = if use_width {
            let dividend = (Sat(width) * Sat(source_height)).0;

            (width, checked!(dividend / source_width)?)
        } else {
            let dividend = (Sat(height) * Sat(source_width)).0;

            (checked!(dividend / source_height)?, height)
        };

        Ok(Self {
            width: x,
            height: y,
            method: Method::Scale,
        })
    }

    pub fn is_passthrough(&self, source: &Self) -> Result<bool> {
        if self.width > source.width || self.height > source.height {
            return Ok(true);
        }

        let (width, height) = if self.crop() {
            (self.width, self.height)
        } else {
            let scaled = self.scaled(source)?;

            (scaled.width, scaled.height)
        };

        Ok(width == source.width && height == source.height)
    }
}

impl Default for Dim {
    #[inline]
    fn default() -> Self {
        Self::original()
    }
}

#[implement(Service)]
pub async fn upload_thumbnail(
    &self,
    mxc: &MxcUri,
    content_disposition: Option<&ContentDisposition>,
    content_type: Option<&str>,
    dim: &Dim,
    file: &[u8],
) -> Result {
    self.create_at(
        mxc,
        dim.dimensions(),
        None,
        content_disposition,
        content_type,
        file,
    )
    .await
}

#[implement(Service)]
#[tracing::instrument(name = "thumbnail", level = "debug", skip(self))]
pub async fn get_thumbnail(&self, mxc: &MxcUri, dim: &Dim) -> Result<(FileMeta, Vec<u8>)> {
    let dim = dim.normalized();

    if let Ok(found) = self.get(mxc, dim.dimensions()).await {
        return Ok(found);
    }

    let (meta, content) = self.get(mxc, Dimensions::ORIGINAL).await?;

    self.generate(mxc, &dim, meta, content).await
}

#[implement(Service)]
#[tracing::instrument(name = "thumbnail", level = "debug", skip(self))]
pub async fn get_or_fetch_thumbnail(&self, mxc: &MxcUri, dim: &Dim) -> Result<(FileMeta, Vec<u8>)> {
    if let Ok(found) = self.get_thumbnail(mxc, dim).await {
        return Ok(found);
    }

    let (server_name, _) = super::parts(mxc)?;

    if self.services.server_state.server_is_ours(server_name) {
        self.get_or_fetch(mxc).await?;

        return self.get_thumbnail(mxc, dim).await;
    }

    let dim = dim.normalized();

    if dim.dimensions() == Dimensions::ORIGINAL {
        return self.get_or_fetch(mxc).await;
    }

    let _lock = self
        .federation_mutex
        .lock(&format!("{mxc}#{}x{}", dim.width, dim.height))
        .await;

    if let Ok(found) = self.get(mxc, dim.dimensions()).await {
        return Ok(found);
    }

    self.fetch_thumbnail(mxc, &dim).await
}

#[cfg(feature = "media_thumbnail")]
#[implement(Service)]
#[tracing::instrument(name = "generate", level = "debug", skip(self, meta, content))]
async fn generate(
    &self,
    mxc: &MxcUri,
    dim: &Dim,
    meta: FileMeta,
    content: Vec<u8>,
) -> Result<(FileMeta, Vec<u8>)> {
    let frame = self
        .video_frame(mxc, dim, meta.content_type.as_deref(), &content)
        .await;

    let from_video = frame.is_some();
    let budget = self.services.config.media.media_thumbnail_max_pixels;
    let requested = dim.clone();

    let (made, content) = tokio::task::spawn_blocking(move || {
        let source = frame.as_deref().unwrap_or(&content);
        let made = thumbnail_of(source, &requested, from_video, budget);

        drop(frame);

        (made, content)
    })
    .await?;

    let bytes = match made {
        Ok(Some(bytes)) => bytes,

        Ok(None) => return Ok((meta, content)),
        Err(e) => {
            if from_video {
                self.remember_failure(mxc);
            }

            debug_warn!(%mxc, "Could not thumbnail the media: {e}");

            return Ok((meta, content));
        }
    };

    drop(content);

    let content_disposition = ContentDisposition::new(ContentDispositionType::Inline)
        .with_filename(Some(THUMBNAIL_NAME.to_owned()));

    self.create_at(
        mxc,
        dim.dimensions(),
        None,
        Some(&content_disposition),
        Some(PNG),
        &bytes,
    )
    .await
    .unwrap_or_else(|e| debug_warn!(%mxc, "Could not store the generated thumbnail: {e}"));

    debug!(%mxc, width = dim.width, height = dim.height, size = bytes.len(), "Generated thumbnail");

    let meta = FileMeta {
        content_type: Some(PNG.to_owned()),
        content_disposition: Some(content_disposition.to_string()),
        size: bytes.len() as u64,
        created: super::now(),
    };

    Ok((meta, bytes))
}

#[cfg(feature = "media_thumbnail")]
fn thumbnail_of(
    bytes: &[u8],
    requested: &Dim,
    from_video: bool,
    budget: u64,
) -> Result<Option<Vec<u8>>> {
    let image = decode(bytes, budget)?;

    let source = Dim::new(image.width(), image.height(), None);

    if !from_video && requested.is_passthrough(&source)? {
        return Ok(None);
    }

    let thumbnail = generate_thumbnail(&image, requested)?;
    let mut bytes = Vec::new();

    thumbnail
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|e| err!("Could not encode the thumbnail: {e}"))?;

    Ok(Some(bytes))
}

#[cfg(not(feature = "media_thumbnail"))]
#[implement(Service)]
#[expect(clippy::unused_async)]
async fn generate(
    &self,
    _mxc: &MxcUri,
    _dim: &Dim,
    meta: FileMeta,
    content: Vec<u8>,
) -> Result<(FileMeta, Vec<u8>)> {
    Ok((meta, content))
}

#[cfg(feature = "media_thumbnail")]
#[tracing::instrument(name = "decode", level = "trace", skip_all)]
fn decode(bytes: &[u8], budget: u64) -> Result<DynamicImage> {
    let (width, height) = reader(bytes)?
        .into_dimensions()
        .map_err(|e| err!(debug_warn!("Could not read the picture's dimensions: {e}")))?;

    let pixels = u64::from(width).saturating_mul(u64::from(height));

    if pixels > budget {
        return Err!(debug_warn!(
            "Picture of {width}x{height} is past the {budget} pixel budget."
        ));
    }

    let mut limits = Limits::no_limits();
    limits.max_alloc = Some(budget.saturating_mul(BYTES_PER_PIXEL));

    let mut reader = reader(bytes)?;
    reader.limits(limits);

    reader
        .decode()
        .map_err(|e| err!(debug_warn!("Could not decode the picture: {e}")))
}

#[cfg(feature = "media_thumbnail")]
fn reader(bytes: &[u8]) -> Result<ImageReader<Cursor<&[u8]>>> {
    ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(Into::into)
}

#[cfg(feature = "media_thumbnail")]
pub(super) fn generate_thumbnail(image: &DynamicImage, requested: &Dim) -> Result<DynamicImage> {
    if !requested.crop() {
        let source = Dim::new(image.width(), image.height(), None);
        let Dim { width, height, .. } = requested.scaled(&source)?;

        return Ok(image.thumbnail_exact(width, height));
    }

    let width = min(requested.width, image.width());
    let height = min(requested.height, image.height());

    Ok(image.resize_to_fill(width, height, FilterType::CatmullRom))
}

#[cfg(test)]
mod tests {
    use ruma::media::Method;

    use super::Dim;

    fn scale(width: u32, height: u32) -> Dim {
        Dim::new(width, height, Some(Method::Scale))
    }

    fn crop(width: u32, height: u32) -> Dim {
        Dim::new(width, height, Some(Method::Crop))
    }

    fn source(width: u32, height: u32) -> Dim {
        Dim::new(width, height, None)
    }

    #[test]
    fn scaling_keeps_the_aspect_ratio() {
        let scaled = scale(320, 240).scaled(&source(1000, 500)).unwrap();

        assert_eq!((scaled.width, scaled.height), (320, 160));
    }

    #[test]
    fn scaling_never_exceeds_the_source() {
        let scaled = scale(800, 600).scaled(&source(100, 50)).unwrap();

        assert_eq!((scaled.width, scaled.height), (100, 50));
    }

    #[test]
    fn passthrough_when_the_request_matches_the_source() {
        assert!(scale(640, 480).is_passthrough(&source(640, 480)).unwrap());
        assert!(crop(96, 96).is_passthrough(&source(96, 96)).unwrap());
    }

    #[test]
    fn passthrough_when_the_request_exceeds_the_source() {
        assert!(scale(800, 600).is_passthrough(&source(640, 480)).unwrap());
        assert!(crop(96, 96).is_passthrough(&source(32, 32)).unwrap());
    }

    #[test]
    fn scaling_fits_inside_the_request() {
        let scaled = scale(320, 240).scaled(&source(500, 1000)).unwrap();

        assert_eq!((scaled.width, scaled.height), (120, 240));
    }

    #[test]
    fn generates_when_the_source_is_larger() {
        assert!(!scale(320, 240).is_passthrough(&source(1920, 1080)).unwrap());
        assert!(!crop(32, 32).is_passthrough(&source(96, 32)).unwrap());
    }

    #[test]
    fn small_requests_are_cropped_into_squares() {
        assert_eq!(scale(17, 20).normalized().dimensions().width, 32);
        assert!(scale(17, 20).normalized().crop());
        assert!(scale(64, 64).normalized().crop());
        assert!(!scale(100, 100).normalized().crop());
    }

    #[test]
    fn requests_round_up_into_buckets() {
        let sizes = |dim: Dim| {
            let normalized = dim.normalized();

            (normalized.width, normalized.height)
        };

        assert_eq!(sizes(scale(1, 1)), (32, 32));
        assert_eq!(sizes(scale(33, 33)), (96, 96));
        assert_eq!(sizes(scale(200, 100)), (320, 240));
        assert_eq!(sizes(scale(567, 400)), (640, 480));
        assert_eq!(sizes(scale(700, 600)), (800, 600));
    }

    #[cfg(feature = "media_thumbnail")]
    mod generate {
        use image::{DynamicImage, RgbaImage};

        use super::{crop, scale};
        use crate::media::thumbnail::generate_thumbnail;

        fn blank(width: u32, height: u32) -> DynamicImage {
            DynamicImage::ImageRgba8(RgbaImage::new(width, height))
        }

        #[test]
        fn cropping_fills_the_request_within_the_source() {
            let thumbnail = generate_thumbnail(&blank(200, 100), &crop(96, 96)).unwrap();

            assert_eq!((thumbnail.width(), thumbnail.height()), (96, 96));
        }

        #[test]
        fn cropping_never_upscales() {
            let thumbnail = generate_thumbnail(&blank(20, 20), &crop(96, 96)).unwrap();

            assert_eq!((thumbnail.width(), thumbnail.height()), (20, 20));
        }

        #[test]
        fn cropping_never_upscales_one_dimension() {
            let thumbnail = generate_thumbnail(&blank(200, 20), &crop(96, 96)).unwrap();

            assert_eq!((thumbnail.width(), thumbnail.height()), (96, 20));
        }

        #[test]
        fn scaling_keeps_the_aspect_ratio() {
            let thumbnail = generate_thumbnail(&blank(1000, 500), &scale(320, 240)).unwrap();

            assert_eq!((thumbnail.width(), thumbnail.height()), (320, 160));
        }

        #[test]
        fn scaling_never_upscales() {
            let thumbnail = generate_thumbnail(&blank(64, 32), &scale(800, 600)).unwrap();

            assert_eq!((thumbnail.width(), thumbnail.height()), (64, 32));
        }
    }

    #[test]
    fn requests_past_every_bucket_address_the_original() {
        let normalized = scale(4096, 4096).normalized();

        assert_eq!(normalized.dimensions(), crate::media::Dimensions::ORIGINAL);
    }
}
