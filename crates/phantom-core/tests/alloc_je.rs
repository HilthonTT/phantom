//! Runtime probes for `alloc::je`. Only meaningful when jemalloc is the active
//! allocator, so the whole file is gated on the feature.
#![cfg(all(not(target_env = "msvc"), feature = "jemalloc"))]

use phantom_core::alloc::je::this_thread;

/// `allocated()` and `deallocated()` must report distinct counters. They are
/// both `thread.*p` pointer reads, so a shared mib key silently aliases them.
#[test]
fn allocated_and_deallocated_are_distinct_counters() {
    const SIZE: usize = 4 * 1024 * 1024;

    // Prime both thread-local mib caches before sampling. Priming allocates, so
    // a first read of `deallocated` can otherwise land after `allocated` was
    // already sampled and appear to exceed it. Touching `allocated` first also
    // means a shared key would be initialized from *its* name, which is the
    // aliasing this test is here to catch.
    let _ = this_thread::allocated();
    let _ = this_thread::deallocated();

    let live_before = this_thread::allocated().saturating_sub(this_thread::deallocated());

    // Hold the allocation live across the measurement. Live bytes must grow by
    // at least the size of the Vec; if both accessors resolve to the same
    // jemalloc counter the difference is pinned to zero and cannot move.
    let v: Vec<u8> = vec![7; SIZE];
    std::hint::black_box(&v);

    let allocated = this_thread::allocated();
    let deallocated = this_thread::deallocated();
    drop(v);

    // Unrelated churn on this thread moves both counters by a few bytes either
    // way, so this asserts the order of magnitude rather than an exact total.
    // Aliased accessors pin the difference to exactly zero, which is the case
    // being ruled out.
    let growth = allocated
        .saturating_sub(deallocated)
        .saturating_sub(live_before);

    assert!(
        growth >= (SIZE / 2) as u64,
        "live bytes grew by {growth} while a {SIZE}-byte Vec was held \
         (allocated={allocated}, deallocated={deallocated}) — the two accessors \
         are reading the same jemalloc mib"
    );
}

#[test]
fn epoch_and_arena_queries_round_trip() {
    assert!(phantom_core::alloc::je::acq_epoch().is_ok());
    assert!(phantom_core::alloc::je::arenas().unwrap() > 0);
    assert!(this_thread::arena_id().is_ok());
}

#[test]
fn memory_stats_is_returned_and_bounded() {
    let stats = phantom_core::alloc::je::memory_stats("").expect("stats");
    assert!(stats.contains("jemalloc"), "unexpected stats payload");
    assert!(stats.len() <= 1_048_576);
}
