//! Column descriptions.
//!
//! A [`Descriptor`] is the shape of a column: how large its values tend to be,
//! how its data is written, and therefore how the engine should compact, cache
//! and compress it. Columns pick one of the archetypes below rather than
//! spelling out three dozen options each, so that a column's declaration says
//! what its data does, not how the engine should be tuned for it.

use rocksdb::{DBCompactionStyle as CompactionStyle, DBCompressionType as CompressionType};

use super::cf_opts::SENTINEL_COMPRESSION_LEVEL;

/// Column descriptor.
#[derive(Debug, Clone, Copy)]
pub struct Descriptor {
    pub name: &'static str,
    pub dropped: bool,
    pub cache_disp: CacheDisp,
    pub block_size: usize,
    pub index_size: usize,
    pub write_size: usize,
    pub cache_size: usize,
    pub level_size: u64,
    pub level_shape: [i32; 7],
    pub file_size: u64,
    pub file_shape: i32,
    pub level0_width: i32,
    pub merge_width: (i32, i32),
    pub limit_size: u64,
    pub ttl: u64,
    pub compaction: CompactionStyle,
    pub compaction_pri: CompactionPri,
    pub compression: CompressionType,
    pub compression_shape: [i32; 7],
    pub compression_level: i32,
    pub bottommost_level: Option<i32>,
    pub block_index_hashing: Option<bool>,
    pub cache_shards: u32,
}

/// Which file a compaction picks next.
///
/// Mirrors the engine's own enumeration, which derives nothing and so cannot
/// sit in a `Copy` descriptor; [`Self::into_rocksdb`] converts back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionPri {
    /// Prefer larger files, discounted by how many deletions they hold.
    ByCompensatedSize,
    /// Prefer the file whose data was updated longest ago. Suits a keyspace
    /// where updates cluster in a few hot ranges.
    OldestLargestSeqFirst,
    /// Prefer the file whose range has gone longest without being compacted
    /// down a level. Suits updates spread across the whole keyspace.
    OldestSmallestSeqFirst,
    /// Prefer the file that overlaps least with the level below, which is
    /// usually the cheapest compaction available.
    MinOverlappingRatio,
    /// Walk each level's files in turn, so every file is compacted eventually
    /// regardless of what it holds.
    RoundRobin,
}

impl CompactionPri {
    pub(super) fn into_rocksdb(self) -> rocksdb::CompactionPri {
        match self {
            Self::ByCompensatedSize => rocksdb::CompactionPri::ByCompensatedSize,
            Self::OldestLargestSeqFirst => rocksdb::CompactionPri::OldestLargestSeqFirst,
            Self::OldestSmallestSeqFirst => rocksdb::CompactionPri::OldestSmallestSeqFirst,
            Self::MinOverlappingRatio => rocksdb::CompactionPri::MinOverlappingRatio,
            Self::RoundRobin => rocksdb::CompactionPri::RoundRobin,
        }
    }
}

/// Where a column's block cache comes from: its own, the cache shared by most
/// columns, or the one belonging to a named sibling.
#[derive(Debug, Clone, Copy)]
pub enum CacheDisp {
    Unique,
    Shared,
    SharedWith(&'static str),
}

/// Base descriptor supplying common defaults to all derived descriptors.
static BASE: Descriptor = Descriptor {
    name: "",
    dropped: false,
    cache_disp: CacheDisp::Shared,
    block_size: 1024 * 4,
    index_size: 1024 * 4,
    write_size: 1024 * 1024 * 2,
    cache_size: 1024 * 1024 * 4,
    level_size: 1024 * 1024 * 8,
    level_shape: [1, 1, 1, 3, 7, 15, 31],
    file_size: 1024 * 1024,
    file_shape: 2,
    level0_width: 2,
    merge_width: (2, 16),
    limit_size: 0,
    ttl: 60 * 60 * 24 * 21,
    compaction: CompactionStyle::Level,
    compaction_pri: CompactionPri::MinOverlappingRatio,
    compression: CompressionType::Zstd,
    compression_shape: [0, 0, 0, 1, 1, 1, 1],
    compression_level: SENTINEL_COMPRESSION_LEVEL,
    bottommost_level: Some(SENTINEL_COMPRESSION_LEVEL),
    block_index_hashing: None,
    cache_shards: 64,
};

/// Tombstone descriptor for columns which have been or will be deleted.
pub static DROPPED: Descriptor = Descriptor {
    dropped: true,
    ..BASE
};

/// Descriptor for large datasets with random updates across the keyspace.
pub static RANDOM: Descriptor = Descriptor {
    compaction_pri: CompactionPri::OldestSmallestSeqFirst,
    write_size: 1024 * 1024 * 32,
    cache_shards: 128,
    compression_level: -3,
    bottommost_level: Some(2),
    ..BASE
};

/// Descriptor for large datasets with updates to the end of the keyspace.
pub static SEQUENTIAL: Descriptor = Descriptor {
    compaction_pri: CompactionPri::OldestLargestSeqFirst,
    write_size: 1024 * 1024 * 64,
    level_size: 1024 * 1024 * 32,
    file_size: 1024 * 1024 * 2,
    cache_shards: 128,
    compression_level: -2,
    bottommost_level: Some(2),
    compression_shape: [0, 0, 1, 1, 1, 1, 1],
    ..BASE
};

/// Descriptor for small datasets with random updates across the keyspace.
pub static RANDOM_SMALL: Descriptor = Descriptor {
    compaction: CompactionStyle::Universal,
    write_size: 1024 * 1024 * 16,
    level_size: 1024 * 512,
    file_size: 1024 * 128,
    file_shape: 3,
    index_size: 512,
    block_size: 512,
    cache_shards: 64,
    compression_level: -4,
    bottommost_level: Some(-1),
    compression_shape: [0, 0, 0, 0, 0, 1, 1],
    ..RANDOM
};

/// Descriptor for small datasets with updates to the end of the keyspace.
pub static SEQUENTIAL_SMALL: Descriptor = Descriptor {
    compaction: CompactionStyle::Universal,
    write_size: 1024 * 1024 * 16,
    level_size: 1024 * 1024,
    file_size: 1024 * 512,
    file_shape: 3,
    block_size: 512,
    cache_shards: 64,
    block_index_hashing: Some(false),
    compression_level: -4,
    bottommost_level: Some(-2),
    compression_shape: [0, 0, 0, 0, 1, 1, 1],
    ..SEQUENTIAL
};

/// Descriptor for small persistent caches with random updates. The oldest
/// entries are dropped once `limit_size` is reached.
pub static RANDOM_SMALL_CACHE: Descriptor = Descriptor {
    compaction: CompactionStyle::Fifo,
    cache_disp: CacheDisp::Unique,
    limit_size: 1024 * 1024 * 64,
    ttl: 60 * 60 * 24 * 14,
    file_shape: 2,
    ..RANDOM_SMALL
};

#[cfg(test)]
mod tests {
    use super::*;

    /// `cf_opts` derives the cache's shard count from the base-2 logarithm of
    /// this figure, and the engine rejects more than 2^10 shards.
    #[test]
    fn cache_shards_are_powers_of_two_within_range() {
        for desc in [
            &BASE,
            &DROPPED,
            &RANDOM,
            &SEQUENTIAL,
            &RANDOM_SMALL,
            &SEQUENTIAL_SMALL,
            &RANDOM_SMALL_CACHE,
        ] {
            assert!(desc.cache_shards.is_power_of_two(), "{}", desc.name);
            assert!(desc.cache_shards.ilog2() <= 10, "{}", desc.name);
        }
    }

    /// A shape entry per level, since they are handed to the engine as the
    /// per-level arrays it expects.
    #[test]
    fn shapes_cover_every_level() {
        for desc in [&RANDOM, &SEQUENTIAL, &RANDOM_SMALL, &SEQUENTIAL_SMALL] {
            assert_eq!(desc.level_shape.len(), desc.compression_shape.len());
        }
    }

    /// The first levels hold the newest and hottest data, where compression
    /// would cost more in CPU than it saves in storage.
    #[test]
    fn compression_starts_below_the_top_levels() {
        for desc in [
            &BASE,
            &RANDOM,
            &SEQUENTIAL,
            &RANDOM_SMALL,
            &SEQUENTIAL_SMALL,
        ] {
            assert_eq!(desc.compression_shape[0], 0, "{}", desc.name);
            assert_eq!(
                desc.compression_shape[6], 1,
                "the bottom level is always compressed"
            );
        }
    }
}
