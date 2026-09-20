//! Mapping a caller's location onto the provider's absolute path.
//!
//! Every operation crosses this boundary, so the base-path prefix is applied in
//! exactly one place and stripped again on the way out.

use object_store::path::Path;
use phantom_core::{Result, err, implement, trace};

use super::Provider;

#[implement(Provider)]
pub(super) fn to_abs_path(&self, location: &str) -> Result<Path> {
    let location = Path::parse(location)
        .map_err(|e| err!("Failed to parse location into canonical PathPart: {e}"))?;

    let path = self.prepend_base_path(location);

    trace!(
        provider = ?self.name,
        base_path = ?self.base_path,
        ?path,
        "Computed absolute path for object on provider.",
    );

    Ok(path)
}

#[implement(Provider)]
pub(super) fn prepend_base_path(&self, location: Path) -> Path {
    match self.base_path.as_ref() {
        Some(base_path) if !location.prefix_matches(base_path) => {
            base_path.parts().chain(location.parts()).collect()
        }

        _ => location,
    }
}

#[implement(Provider)]
pub(super) fn strip_base_path(&self, location: Path) -> Path {
    self.base_path
        .as_ref()
        .and_then(|base_path| location.prefix_match(base_path))
        .map(Iterator::collect)
        .unwrap_or(location)
}
