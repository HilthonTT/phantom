//! The smaller picture a client asks for in place of the original.
//!
//! A thumbnail is stored as media in its own right — the same media id, at a
//! size other than [`Dimensions::ORIGINAL`] — so serving one that has already
//! been generated is an ordinary media read, and purging a piece of media
//! purges every thumbnail of it along with the file itself.
//!
//! **A requested size is rounded up into one of a handful of buckets** before
//! anything is stored or looked up. Clients ask for whatever their layout
//! happens to want, and honouring each request literally would store a
//! picture per client per screen width; a bucket is a size many requests
//! share, so one generated thumbnail answers all of them. Rounding up rather
//! than down is what keeps the result from being scaled back up for display.
//!
//! **Generation is gated on `media_thumbnail`,** which is what carries the
//! image decoder. Without it every entry point here still answers — with the
//! original, which the media repository specification permits a server to
//! send in place of a thumbnail — so a client is shown a picture either way,
//! and nothing here claims to have produced a thumbnail it has not.
//!
//! Videos are handled the same way, one step removed: an operator-configured
//! program extracts a still frame, and that frame is then thumbnailed as an
//! ordinary picture. See the `video` module beside this one.

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

/// The type every thumbnail this server generates is served as.
///
/// One format for all of them, rather than the original's: a client asking
/// for a thumbnail is displaying it, and PNG is what every client can show.
#[cfg(feature = "media_thumbnail")]
const PNG: &str = "image/png";

/// Bytes the decoder is budgeted per pixel of the picture it is asked for.
#[cfg(feature = "media_thumbnail")]
const BYTES_PER_PIXEL: u64 = 4;

/// The filename a generated thumbnail is served under.
///
/// The media repository specification asks for this rather than the name of
/// the file it was generated from, which is just as well: the original's name
/// describes a file the client is not being sent.
#[cfg(feature = "media_thumbnail")]
const THUMBNAIL_NAME: &str = "thumbnail.png";

/// A thumbnail as a client asked for it: a size, and how to reach it.
///
/// Distinct from [`Dimensions`], which is where a picture is *stored*. A
/// request is rounded to a bucket ([`Dim::normalized`]) before it names
/// storage, and the method is part of how the picture is produced rather than
/// part of its address — a 96×96 crop and a 96×96 scale are the same entry.
#[derive(Clone, Debug)]
pub struct Dim {
    pub width: u32,
    pub height: u32,
    pub method: Method,
}

impl Dim {
    /// The original, as opposed to any thumbnail of it.
    ///
    /// Also what [`Dim::normalized`] answers for a request too large for any
    /// bucket, which is how such a request comes to be served the original.
    #[inline]
    #[must_use]
    pub fn original() -> Self {
        Self::new(0, 0, None)
    }

    /// A dimension pair as a client sent it, in ruma's integers.
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

    /// Where a thumbnail of this size is stored.
    #[inline]
    #[must_use]
    pub fn dimensions(&self) -> Dimensions {
        Dimensions {
            width: self.width,
            height: self.height,
        }
    }

    /// Whether the picture is cropped to the request rather than fitted
    /// inside it.
    #[inline]
    #[must_use]
    pub fn crop(&self) -> bool {
        self.method == Method::Crop
    }

    /// The bucket this request is served out of.
    ///
    /// Sizes at or below 96 pixels are cropped rather than scaled: at that
    /// size an avatar fitted inside the box is mostly box, and a client
    /// asking for one wants the square filled.
    ///
    /// A request past the largest bucket is answered with the original, which
    /// is what [`Dim::original`] addresses. Generating something larger than
    /// anything a client displays inline would be storing a second copy of
    /// the file to save nothing.
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

    /// This request fitted inside `source`, with the source's aspect ratio
    /// kept.
    ///
    /// The result fits within the request in both dimensions rather than
    /// covering it, which is what `scale` means in the media repository
    /// specification: the client is given a picture it can display whole. It
    /// is never larger than the source either, since a thumbnail larger than
    /// what it was made from is the original again with a re-encode's loss.
    pub fn scaled(&self, source: &Self) -> Result<Self> {
        let source_width = source.width;
        let source_height = source.height;

        let width = min(self.width, source_width);
        let height = min(self.height, source_height);

        // Which side binds: the one the source runs out of first. Comparing
        // the two ratios by cross-multiplication rather than dividing keeps
        // the decision exact.
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

    /// Whether generating would only reproduce the source, so that the
    /// original should be sent instead.
    ///
    /// Two ways that happens: the request is larger than the source, which
    /// would be upscaling, or it scales to exactly the source's own
    /// dimensions. Both would spend a decode and an encode to hand back the
    /// picture the server already has, and the second would store a second
    /// copy of it.
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

/// Stores a thumbnail a client supplied, rather than one this server made.
///
/// The size is taken as given rather than rounded: the client is describing a
/// picture it has, not asking for one, and a later request is what rounds.
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

/// Serves a thumbnail, generating it from a stored original if there is not
/// one already, but never contacting the server the media came from.
#[implement(Service)]
#[tracing::instrument(name = "thumbnail", level = "debug", skip(self))]
pub async fn get_thumbnail(&self, mxc: &MxcUri, dim: &Dim) -> Result<(FileMeta, Vec<u8>)> {
    let dim = dim.normalized();

    // Also the hit for a request past the largest bucket, which normalizes to
    // the original and is answered with it here.
    if let Ok(found) = self.get(mxc, dim.dimensions()).await {
        return Ok(found);
    }

    let (meta, content) = self.get(mxc, Dimensions::ORIGINAL).await?;

    self.generate(mxc, &dim, meta, content).await
}

/// Serves a thumbnail, obtaining what it needs from the server the media came
/// from where this server does not have it.
///
/// A remote thumbnail is asked of the origin rather than made here: the
/// origin has the file already, so fetching the whole original to scale it
/// down would be spending the bandwidth of the very thing a thumbnail exists
/// to avoid. Where the original happens to be cached here anyway, the local
/// path above has already answered.
#[implement(Service)]
#[tracing::instrument(name = "thumbnail", level = "debug", skip(self))]
pub async fn get_or_fetch_thumbnail(&self, mxc: &MxcUri, dim: &Dim) -> Result<(FileMeta, Vec<u8>)> {
    if let Ok(found) = self.get_thumbnail(mxc, dim).await {
        return Ok(found);
    }

    let (server_name, _) = super::parts(mxc)?;

    if self.services.server_state.server_is_ours(server_name) {
        // Ours and not stored is not necessarily unknown: a URL preview mints
        // a URI for each piece of media a page names without downloading it,
        // and this is the request that resolves one.
        self.get_or_fetch(mxc).await?;

        return self.get_thumbnail(mxc, dim).await;
    }

    let dim = dim.normalized();

    // A request past the largest bucket normalizes to the original, and it is
    // the original that has to be fetched to answer it. Asking the origin for
    // a thumbnail of no size would be asking it for nothing.
    if dim.dimensions() == Dimensions::ORIGINAL {
        return self.get_or_fetch(mxc).await;
    }

    // One fetch per size is in flight at a time, for the same reason as in
    // the lazy path: a URI handed to a room's worth of clients at once must
    // not become a room's worth of requests at the origin.
    let _lock = self
        .federation_mutex
        .lock(&format!("{mxc}#{}x{}", dim.width, dim.height))
        .await;

    if let Ok(found) = self.get(mxc, dim.dimensions()).await {
        return Ok(found);
    }

    self.fetch_thumbnail(mxc, &dim).await
}

/// Produces a thumbnail from an original, and stores it.
///
/// Failures here are not errors: a picture the decoder refuses and a video no
/// program could be run on both end the same way, with the original served in
/// the thumbnail's place. The alternative is answering a client's request for
/// a picture with no picture at all.
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
    let budget = self.services.config.media_thumbnail_max_pixels;
    let requested = dim.clone();

    // Decoding and re-encoding a picture is the only real arithmetic the
    // server does to answer a request, and a worker thread spending a second
    // on it is a worker thread answering nothing else. The original is handed
    // back out again because the paths below still need it.
    let (made, content) = tokio::task::spawn_blocking(move || {
        let source = frame.as_deref().unwrap_or(&content);
        let made = thumbnail_of(source, &requested, from_video, budget);

        drop(frame);

        (made, content)
    })
    .await?;

    let bytes = match made {
        Ok(Some(bytes)) => bytes,
        // Generating could not improve on what is stored.
        Ok(None) => return Ok((meta, content)),
        Err(e) => {
            // A frame the thumbnailer refuses is this video's verdict too.
            // Without that, the program would be run again on the next
            // request for any size, to produce a frame refused the same way.
            if from_video {
                self.remember_failure(mxc);
            }

            debug_warn!(%mxc, "Could not thumbnail the media: {e}");

            return Ok((meta, content));
        }
    };

    // Nothing below reads the original, which on the video path is the whole
    // staged file; the store must not hold it as well.
    drop(content);

    let content_disposition = ContentDisposition::new(ContentDispositionType::Inline)
        .with_filename(Some(THUMBNAIL_NAME.to_owned()));

    // Stored so the decode is spent once rather than on every request. A
    // store that fails costs the next request the same work, which is not
    // worth failing a thumbnail this one already has in hand.
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

/// The PNG a picture thumbnails to, or `None` where the original should be
/// served instead.
///
/// Everything that costs processor time is here, so that the caller has one
/// thing to hand to a blocking thread.
#[cfg(feature = "media_thumbnail")]
fn thumbnail_of(
    bytes: &[u8],
    requested: &Dim,
    from_video: bool,
    budget: u64,
) -> Result<Option<Vec<u8>>> {
    let image = decode(bytes, budget)?;

    // A video is never servable in place of its own thumbnail — a client
    // asking for a picture cannot show one — so its frame is re-encoded
    // however small it turns out to be.
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

/// Serving the original, for a build with no image decoder in it.
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

/// Decodes a picture whose header declares no more than the configured pixel
/// count.
///
/// The dimensions are read and checked before any decoder allocates, because
/// [`Limits`] enforces a byte budget only, and a decoder is free to ignore
/// it. Without that check a few kilobytes of header can ask for gigabytes.
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

/// Scales or crops one picture into another.
///
/// Kept apart from the storing and the decoding around it so that what the
/// two methods actually do to a picture can be tested on its own.
#[cfg(feature = "media_thumbnail")]
pub(super) fn generate_thumbnail(image: &DynamicImage, requested: &Dim) -> Result<DynamicImage> {
    if !requested.crop() {
        let source = Dim::new(image.width(), image.height(), None);
        let Dim { width, height, .. } = requested.scaled(&source)?;

        return Ok(image.thumbnail_exact(width, height));
    }

    // Upscaling is forbidden outright, and `resize_to_fill` enlarges a source
    // smaller than the request to meet it.
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
        // The portrait source runs out of height first, so the height binds
        // and the width comes in under what was asked for.
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
