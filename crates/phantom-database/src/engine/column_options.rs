//! Per-column options, derived from a [`Descriptor`].

use phantom_core::{Config, Result, err};
use rocksdb::{
    BlockBasedIndexType, BlockBasedOptions, BlockBasedTablePinningTier, Cache,
    DBCompressionType as CompressionType, DataBlockIndexType, FifoCompactOptions, LruCacheOptions,
    Options, UniversalCompactOptions, UniversalCompactionStopStyle,
};

use super::{
    Context,
    descriptor::{CacheDisp, Descriptor},
};

/// The level the engine reads as "whatever this algorithm calls its default",
/// since the valid range differs per algorithm. phantom substitutes a level of
/// its own for any column whose descriptor names one while the config still
/// holds this.
pub(super) const SENTINEL_COMPRESSION_LEVEL: i32 = 32767;

/// Options for one column. Takes the result of
/// [`db_options`](super::database_options::db_options) and narrows it to the column
/// described by `desc`; the result is what the column is opened with.
pub(crate) fn cf_options(ctx: &Context, opts: Options, desc: &Descriptor) -> Result<Options> {
    let cache = get_cache(ctx, desc);
    let config = &ctx.server.config;

    descriptor_cf_options(opts, *desc, config, cache.as_ref())
}

fn descriptor_cf_options(
    mut opts: Options,
    mut desc: Descriptor,
    config: &Config,
    cache: Option<&Cache>,
) -> Result<Options> {
    set_compression(&mut desc, config);
    set_table_options(&mut opts, &desc, cache);

    opts.set_min_write_buffer_number(1);
    opts.set_max_write_buffer_number(2);
    opts.set_write_buffer_size(desc.write_size);

    opts.set_target_file_size_base(desc.file_size);
    opts.set_target_file_size_multiplier(desc.file_shape);

    opts.set_level_zero_file_num_compaction_trigger(desc.level0_width);
    opts.set_level_compaction_dynamic_level_bytes(false);
    opts.set_ttl(desc.ttl);

    opts.set_max_bytes_for_level_base(desc.level_size);
    opts.set_max_bytes_for_level_multiplier(1.0);
    opts.set_max_bytes_for_level_multiplier_additional(&desc.level_shape);

    opts.set_compaction_style(desc.compaction);
    opts.set_compaction_pri(desc.compaction_pri.into_rocksdb());
    opts.set_universal_compaction_options(&uc_options(&desc));
    opts.set_fifo_compaction_options(&fifo_options(&desc));

    let compression_shape: Vec<_> = desc
        .compression_shape
        .into_iter()
        .map(|val| (val > 0).then_some(desc.compression))
        .map(|val| val.unwrap_or(CompressionType::None))
        .collect();

    opts.set_compression_type(desc.compression);
    opts.set_compression_per_level(compression_shape.as_slice());
    opts.set_compression_options(-14, desc.compression_level, 0, 0); // -14 w_bits is read by zlib.
    if let Some(&bottommost_level) = desc.bottommost_level.as_ref() {
        opts.set_bottommost_compression_type(desc.compression);
        opts.set_bottommost_zstd_max_train_bytes(0, true);
        opts.set_bottommost_compression_options(-14, bottommost_level, 0, 0, true);
    }

    // Larger than the engine's own default, so that a memtable serving values
    // of the sizes phantom writes allocates in fewer, larger blocks.
    opts.set_arena_block_size(1024 * 1024 * 2);

    // Debug builds pay for the consistency checks; release builds honour the
    // operator's `rocksdb_paranoid_file_checks` instead.
    #[cfg(debug_assertions)]
    opts.set_paranoid_checks(true);

    Ok(opts)
}

fn set_table_options(opts: &mut Options, desc: &Descriptor, cache: Option<&Cache>) {
    let mut table = table_options(desc, cache.is_some());

    if let Some(cache) = cache {
        table.set_block_cache(cache);
    } else {
        table.disable_cache();
    }

    opts.set_block_based_table_factory(&table);
}

fn set_compression(desc: &mut Descriptor, config: &Config) {
    desc.compression = match config.rocksdb_compression_algo.as_ref() {
        "snappy" => CompressionType::Snappy,
        "zlib" => CompressionType::Zlib,
        "bz2" => CompressionType::Bz2,
        "lz4" => CompressionType::Lz4,
        "lz4hc" => CompressionType::Lz4hc,
        "none" => CompressionType::None,
        // `config::check` rejects anything else, so this is zstd and the
        // spellings of it rather than a silent fallback.
        _ => CompressionType::Zstd,
    };

    // The per-column levels below are tuned for zstd. An operator who names a
    // level, or an algorithm whose scale differs, takes precedence over them.
    let can_override_level = config.rocksdb_compression_level == SENTINEL_COMPRESSION_LEVEL
        && desc.compression == CompressionType::Zstd;

    if !can_override_level {
        desc.compression_level = config.rocksdb_compression_level;
    }

    let can_override_bottom = config.rocksdb_bottommost_compression_level
        == SENTINEL_COMPRESSION_LEVEL
        && desc.compression == CompressionType::Zstd;

    if !can_override_bottom {
        desc.bottommost_level = Some(config.rocksdb_bottommost_compression_level);
    }

    if !config.rocksdb_bottommost_compression {
        desc.bottommost_level = None;
    }
}

fn fifo_options(desc: &Descriptor) -> FifoCompactOptions {
    let mut opts = FifoCompactOptions::default();
    opts.set_max_table_files_size(desc.limit_size);

    opts
}

fn uc_options(desc: &Descriptor) -> UniversalCompactOptions {
    let mut opts = UniversalCompactOptions::default();
    opts.set_stop_style(UniversalCompactionStopStyle::Total);
    opts.set_min_merge_width(desc.merge_width.0);
    opts.set_max_merge_width(desc.merge_width.1);
    opts.set_max_size_amplification_percent(10000);
    opts.set_compression_size_percent(-1);
    opts.set_size_ratio(1);

    opts
}

fn table_options(desc: &Descriptor, has_cache: bool) -> BlockBasedOptions {
    let mut opts = BlockBasedOptions::default();

    opts.set_block_size(desc.block_size);
    opts.set_metadata_block_size(desc.index_size);

    opts.set_cache_index_and_filter_blocks(has_cache);
    opts.set_pin_top_level_index_and_filter(false);
    opts.set_pin_l0_filter_and_index_blocks_in_cache(false);
    opts.set_partition_pinning_tier(BlockBasedTablePinningTier::None);
    opts.set_unpartitioned_pinning_tier(BlockBasedTablePinningTier::None);
    opts.set_top_level_index_pinning_tier(BlockBasedTablePinningTier::None);

    opts.set_partition_filters(true);
    opts.set_index_type(BlockBasedIndexType::TwoLevelIndexSearch);

    opts.set_data_block_index_type(match desc.block_index_hashing {
        None if desc.index_size > 512 => DataBlockIndexType::BinaryAndHash,
        Some(enable) if enable => DataBlockIndexType::BinaryAndHash,
        Some(_) | None => DataBlockIndexType::BinarySearch,
    });

    opts
}

/// The block cache a column reads through, which is either its own, one it
/// shares with a named sibling, or the cache shared by every column that asks
/// for nothing in particular.
fn get_cache(ctx: &Context, desc: &Descriptor) -> Option<Cache> {
    if desc.dropped {
        return None;
    }

    let shard_bits: i32 = desc
        .cache_shards
        .ilog2()
        .try_into()
        .expect("cache_shards fits in i32 once reduced to its logarithm");

    debug_assert!(shard_bits <= 10, "cache shards probably too large");

    let mut cache_opts = LruCacheOptions::default();
    cache_opts.set_num_shard_bits(shard_bits);
    cache_opts.set_capacity(desc.cache_size);

    let mut caches = ctx.col_cache.lock().expect("locked");
    match desc.cache_disp {
        CacheDisp::Unique if desc.cache_size == 0 => None,
        CacheDisp::Unique => {
            let cache = Cache::new_lru_cache_opts(&cache_opts);
            caches.insert(desc.name.into(), cache.clone());
            Some(cache)
        }

        CacheDisp::SharedWith(other) if !caches.contains_key(other) => {
            let cache = Cache::new_lru_cache_opts(&cache_opts);
            caches.insert(desc.name.into(), cache.clone());
            Some(cache)
        }

        CacheDisp::SharedWith(other) => Some(
            caches
                .get(other)
                .cloned()
                .expect("caches.contains_key(other) must be true"),
        ),

        CacheDisp::Shared => Some(
            caches
                .get(Context::SHARED_CACHE)
                .cloned()
                .expect("shared cache must already exist"),
        ),
    }
}

/// Scales a capacity given in entities by the operator's
/// `cache_capacity_modifier` and the size of one entity.
pub(super) fn cache_size_f64(config: &Config, base_size: f64, entity_size: usize) -> Result<usize> {
    let ents = phantom_core::math::usize_from_f64(base_size * config.cache_capacity_modifier)
        .map_err(|e| err!(Config("cache_capacity_modifier", "{e}")))?;

    ents.checked_mul(entity_size)
        .ok_or_else(|| err!(Config("cache_capacity_modifier", "cache size is too large")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(toml: &str) -> Config {
        use figment::{
            Figment,
            providers::{Format, Toml},
        };

        let toml = format!(
            "[global]\nserver_name = \"phantom.chat\"\ndatabase_path = \"/var/lib/phantom\"\n{toml}"
        );

        Config::new(&Figment::new().merge(Toml::string(&toml).nested())).expect("config is valid")
    }

    #[test]
    fn cache_size_scales_by_the_modifier() {
        let config = config("cache_capacity_modifier = 0.5\n");

        assert_eq!(
            cache_size_f64(&config, 100.0, 1024).expect("in range"),
            50 * 1024
        );
    }

    #[test]
    fn cache_size_rejects_an_overflowing_capacity() {
        let config = config("cache_capacity_modifier = 1.0\n");

        assert!(
            cache_size_f64(&config, f64::from(u32::MAX), usize::MAX).is_err(),
            "an overflowing cache size is a config error, not a wrapped value"
        );
    }

    /// An operator who names a compression level means it, even for a column
    /// whose descriptor carries one tuned for the default algorithm.
    #[test]
    fn a_configured_compression_level_overrides_the_descriptor() {
        let config = config("rocksdb_compression_level = 9\n");
        let mut desc = super::super::descriptor::RANDOM;

        set_compression(&mut desc, &config);

        assert_eq!(desc.compression_level, 9);
    }

    #[test]
    fn the_descriptor_level_survives_the_default_config() {
        let config = config("");
        let mut desc = super::super::descriptor::RANDOM;
        let tuned = desc.compression_level;

        set_compression(&mut desc, &config);

        assert_eq!(desc.compression_level, tuned, "sentinel means 'ours'");
        assert_eq!(desc.compression, CompressionType::Zstd);
    }

    #[test]
    fn disabling_bottommost_compression_clears_the_level() {
        let config = config("rocksdb_bottommost_compression = false\n");
        let mut desc = super::super::descriptor::RANDOM;

        set_compression(&mut desc, &config);

        assert!(desc.bottommost_level.is_none());
    }
}
