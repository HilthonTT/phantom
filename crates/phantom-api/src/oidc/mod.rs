use std::fmt::Write;

pub mod account;

pub(crate) fn url_encode(s: &str) -> String {
    s.bytes()
        .fold(String::with_capacity(s.len()), |mut out, b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                out.push(b.into());
            } else {
                write!(&mut out, "%{b:02X}").ok();
            }

            out
        })
}
