pub fn trim<I: Into<Option<usize>> + Copy>(_: I) -> crate::Result {
    Ok(())
}

#[must_use]
pub fn memory_stats(_opts: &str) -> Option<String> {
    None
}

#[must_use]
pub fn memory_usage() -> Option<String> {
    None
}
