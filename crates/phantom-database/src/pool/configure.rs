//! Sizing the pool after the storage device rather than the CPU.
//!
//! A pool worker spends its life waiting on the device, so the useful number
//! of them is set by how many requests the device will accept at once, not by
//! how many cores are free. Linux publishes that: a block device has one or
//! more hardware queues, each with a tag count — its depth — and a list of the
//! CPUs whose requests it carries. That is what is read here, and where it
//! cannot be read the configured fallback stands in.

use std::sync::Arc;

use phantom_core::{
    debug, debug_info, is_equal_to,
    math::usize_from_f64,
    result::LogDebugErr,
    server::Server,
    stream::{self, AMPLIFICATION_LIMIT, WIDTH_LIMIT},
    sys::{
        compute::is_core_available,
        storage::{self, Parallelism},
    },
};

use super::{QUEUE_LIMIT, WORKER_LIMIT};

/// The number of workers to spawn, the depth of each queue, and the core-to-
/// queue map the workers take their affinity from.
pub(super) fn configure(server: &Arc<Server>) -> (usize, Vec<usize>, Vec<usize>) {
    let config = &server.config;
    let path = config.database_path.as_path();

    let device_name = storage::name_from_path(path).log_debug_err().ok();
    let device = storage::parallelism(path);

    // Only consulted where the device told us nothing, in which case it is
    // also the entire worker count.
    let fallback = device.mq.is_empty().then_some(config.db_pool_workers);

    let worker_counts = worker_counts(&device, config.db_pool_workers_limit)
        .chain(fallback)
        .collect::<Vec<_>>();

    // The software queue between a tokio worker and a pool worker. Sized off
    // the workers draining it, since a queue deeper than its workers can serve
    // only converts a bounded wait into an unbounded one.
    let queue_sizes: Vec<_> = worker_counts
        .iter()
        .map(|workers| {
            workers
                .saturating_mul(config.db_pool_queue_mult)
                .clamp(QUEUE_LIMIT.0, QUEUE_LIMIT.1)
        })
        .collect();

    let topology = topology(&device);

    // The device's total tag count is the real ceiling: workers past it would
    // queue inside the kernel instead of here, where the backpressure is.
    let max_workers = device
        .mq
        .iter()
        .filter_map(|mq| mq.nr_tags)
        .chain(fallback)
        .fold(0_usize, usize::saturating_add)
        .clamp(WORKER_LIMIT.0, WORKER_LIMIT.1);

    let total_workers = worker_counts
        .iter()
        .copied()
        .fold(0_usize, usize::saturating_add)
        .clamp(WORKER_LIMIT.0, max_workers);

    // Now that the shape of the pool is known, the stream combinators can be
    // told how much concurrency it will actually absorb.
    if config.stream_width_scale > 0.0 {
        update_stream_width(server, queue_sizes.len().max(1), total_workers);
    }

    debug_info!(
        device_name = device_name.as_deref().unwrap_or("unknown"),
        ?worker_counts,
        ?queue_sizes,
        ?total_workers,
        stream_width = stream::automatic_width(),
        "Database frontend topology",
    );

    assert!(total_workers > 0, "some workers are expected");
    assert!(!queue_sizes.is_empty(), "some queues are expected");
    assert!(
        !queue_sizes.iter().copied().any(is_equal_to!(0)),
        "queue sizes are expected to be positive"
    );

    (total_workers, queue_sizes, topology)
}

/// Workers to give each of the device's hardware queues.
///
/// A queue's depth is the ceiling, but a queue whose cores are all masked off
/// from this process cannot be reached at all, and one reachable through a
/// single core does not want its full depth in threads.
fn worker_counts(device: &Parallelism, per_core_limit: usize) -> impl Iterator<Item = usize> + '_ {
    device
        .mq
        .iter()
        .filter(|mq| mq.cpu_list.iter().copied().any(is_core_available))
        .map(move |mq| {
            let cores = mq
                .cpu_list
                .iter()
                .copied()
                .filter(|&id| is_core_available(id))
                .count()
                .max(1);

            let limit = per_core_limit.saturating_mul(cores);
            let limit = device.nr_requests.map_or(limit, |nr| nr.min(limit));

            mq.nr_tags.unwrap_or(WORKER_LIMIT.0).min(limit)
        })
}

/// Maps each core to the queue that serves it.
///
/// Cores unavailable to this process keep the default of queue zero: nothing
/// will ever submit from them, and a hole would only need handling at every
/// lookup.
fn topology(device: &Parallelism) -> Vec<usize> {
    /// Long enough for any core id we expect to be scheduled on. Reads past
    /// it fall back to the first queue rather than growing this.
    const CORES: usize = 128;

    device.mq.iter().fold(vec![0; CORES], |mut topology, mq| {
        mq.cpu_list
            .iter()
            .copied()
            .filter(|&id| is_core_available(id))
            .filter(|&id| id < CORES)
            .for_each(|id| topology[id] = mq.id);

        topology
    })
}

/// Retunes the stream combinators to the pool that was just derived.
///
/// Their defaults are guesses made before anything is known about the host;
/// the width the storage will actually absorb is workers per queue, since that
/// is how many requests can be outstanding on one queue at a time.
#[allow(clippy::as_conversions, clippy::cast_precision_loss)]
fn update_stream_width(server: &Arc<Server>, num_queues: usize, total_workers: usize) {
    let config = &server.config;
    let scale = f64::from(config.stream_width_scale.min(100.0));

    let width = total_workers
        .checked_div(num_queues)
        .expect("queue count is non-zero")
        .next_multiple_of(2);

    let width = usize_from_f64(width as f64 * scale)
        .expect("a scaled width is a positive number")
        .clamp(WIDTH_LIMIT.0, WIDTH_LIMIT.1);

    let amplification = usize_from_f64(config.stream_amplification as f64 * scale)
        .expect("a scaled amplification is a positive number")
        .clamp(AMPLIFICATION_LIMIT.0, AMPLIFICATION_LIMIT.1);

    let (old_width, new_width) = stream::set_width(width);
    let (old_amp, new_amp) = stream::set_amplification(amplification);

    debug!(
        scale = ?config.stream_width_scale,
        num_queues,
        old_width,
        new_width,
        old_amp,
        new_amp,
        "Retuned stream width"
    );
}

#[cfg(test)]
mod tests {
    use phantom_core::sys::storage::Queue;

    use super::*;

    fn queue(id: usize, nr_tags: Option<usize>, cpu_list: Vec<usize>) -> Queue {
        Queue {
            id,
            nr_tags,
            cpu_list,
        }
    }

    /// The device is what bounds the pool: more tags than the cores feeding a
    /// queue could keep busy should not turn into more threads.
    #[test]
    fn a_deep_queue_is_capped_by_the_cores_that_feed_it() {
        let device = Parallelism {
            nr_requests: None,
            mq: vec![queue(0, Some(1024), vec![0])],
        };

        assert_eq!(
            worker_counts(&device, 64).collect::<Vec<_>>(),
            [64],
            "one core at a per-core limit of 64"
        );
    }

    /// And the reverse: a shallow queue is not padded out to the limit.
    #[test]
    fn a_shallow_queue_keeps_its_own_depth() {
        let device = Parallelism {
            nr_requests: None,
            mq: vec![queue(0, Some(4), vec![0, 1, 2, 3])],
        };

        assert_eq!(worker_counts(&device, 64).collect::<Vec<_>>(), [4]);
    }

    /// `nr_requests` is the whole device's budget, so it caps a queue even
    /// where the cores would allow more.
    #[test]
    fn the_device_request_budget_caps_a_queue() {
        let device = Parallelism {
            nr_requests: Some(8),
            mq: vec![queue(0, Some(1024), vec![0, 1, 2, 3])],
        };

        assert_eq!(worker_counts(&device, 64).collect::<Vec<_>>(), [8]);
    }

    /// Cores this process cannot be scheduled on are not cores that will
    /// submit requests.
    #[test]
    fn a_queue_reachable_from_no_available_core_is_skipped() {
        let device = Parallelism {
            nr_requests: None,
            mq: vec![queue(0, Some(32), vec![usize::MAX])],
        };

        assert_eq!(
            worker_counts(&device, 64).count(),
            0,
            "an unreachable queue gets no workers"
        );
    }

    #[test]
    fn topology_maps_each_core_to_its_queue() {
        let device = Parallelism {
            nr_requests: None,
            mq: vec![queue(0, Some(32), vec![0]), queue(1, Some(32), vec![1])],
        };

        let topology = topology(&device);

        // Only the cores actually available to the test process were mapped,
        // so check the ones that were rather than asserting on both.
        for (core, &queue) in topology.iter().enumerate().take(2) {
            if is_core_available(core) {
                assert_eq!(queue, core, "core {core} should map to queue {core}");
            }
        }
    }

    /// Out-of-range core ids come from a machine wider than the table, and
    /// must not panic the server at startup.
    #[test]
    fn topology_ignores_cores_beyond_the_table() {
        let device = Parallelism {
            nr_requests: None,
            mq: vec![queue(1, Some(32), vec![4096])],
        };

        assert!(
            topology(&device).iter().all(is_equal_to!(&0)),
            "an out-of-range core leaves the table at its default"
        );
    }
}
