//! What rustc was asked to do, read back from its own command line.
//!
//! `#[config_example_generator]` writes a file as a side effect, and doing
//! that on every metadata-only pass would have an editor rewrite it on each
//! keystroke. These say whether this invocation is one that should.

/// True when rustc was invoked to actually link a binary, as opposed to the
/// metadata-only passes `cargo check`, rust-analyzer and friends run.
///
/// The example file is only written on real builds so that editors re-checking
/// the crate on every keystroke do not keep rewriting it.
pub(crate) fn is_cargo_build() -> bool {
    let mut args = std::env::args();

    while let Some(arg) = args.next() {
        let kinds = if let Some(kinds) = arg.strip_prefix("--emit=") {
            kinds.to_owned()
        } else if arg == "--emit" {
            let Some(kinds) = args.next() else {
                break;
            };

            kinds
        } else {
            continue;
        };

        if kinds
            .split(',')
            .any(|kind| kind.split_once('=').map_or(kind, |(kind, _)| kind) == "link")
        {
            return true;
        }
    }

    false
}

/// True when rustc is building a test harness rather than the library itself.
pub(crate) fn is_cargo_test() -> bool {
    std::env::args().any(|flag| flag == "--test")
}
