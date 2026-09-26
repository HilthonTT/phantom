use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
pub enum IpSource {
    /// TCP peer address. Safe default; no proxy required.
    #[default]
    ConnectInfo,

    /// Rightmost value of `X-Forwarded-For`.
    RightmostXForwardedFor,

    /// Rightmost value of RFC 7239 `Forwarded`.
    RightmostForwarded,

    /// `X-Real-IP` header (nginx).
    XRealIp,

    /// `CF-Connecting-IP` (Cloudflare / cloudflared).
    CfConnectingIp,

    /// `True-Client-IP` (Akamai, Cloudflare Enterprise).
    TrueClientIp,

    /// `Fly-Client-IP` (Fly.io).
    FlyClientIp,

    /// `CloudFront-Viewer-Address` (AWS CloudFront).
    #[serde(rename = "cloudfront_viewer_address")]
    CloudFrontViewerAddress,
}
