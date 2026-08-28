//! One column of the database, and the typed surface over it.
//!
//! The surface is split across the submodules by what it does — reads in
//! [`get`], writes in [`insert`], iteration in [`keys`] and [`stream`] over
//! the one path in [`iter`] — and lands on [`Map`] as one flat set of methods.

mod clear;
pub mod compact;
mod contains;
mod count;
mod get;
mod insert;
mod iter;
mod keys;
mod open;
mod qry;
mod qry_batch;
mod stream;

pub use self::qry_batch::Qry;

use std::{
    ffi::CStr,
    fmt::{self, Debug, Display},
    future::Future,
    sync::Arc,
};

use phantom_core::Result;
use rocksdb::{AsColumnFamilyRef, ColumnFamily, ReadOptions, ReadTier, WriteOptions};

use crate::{Engine, watchers::Watchers};

/// One column of the database: an ordered mapping of byte keys to byte
/// values, with the reads, writes and iterations phantom performs on it.
///
/// The engine underneath holds every column at once; this is the handle to
/// one of them, together with the read and write options it was opened with.
/// Nearly everything above this crate does its work here.
///
/// # Raw and serialized
///
/// Every operation comes in two forms, and the name says which:
///
/// - The plain form takes a [`Serialize`](serde::Serialize) key and returns a
///   [`Deserialize`](serde::Deserialize) result. This is what callers want:
///   the key is built from typed components and the result arrives as a type.
/// - The `raw_` form takes and returns bytes, for a key that was already
///   serialized or a value that is going somewhere as bytes anyway.
///
/// Where a caller needs one of each — a typed key with a raw result, say —
/// they serialize the key themselves with
/// [`serialize_key`](crate::keyval::serialize_key) and call the `raw_` form,
/// or map the raw result through
/// [`result_deserialize`](crate::keyval::result_deserialize). Naming all four
/// combinations is what makes this kind of surface hard to read, and the two
/// left out compose out of the two that are named.
///
/// # Order
///
/// Keys are ordered by their bytes, which is what makes a range of them
/// askable for. [`Interfix`](crate::Interfix) is what ends a key that is going
/// to be used as a prefix: it leaves the trailing record separator on, without
/// which the prefix would also match a longer component that happens to start
/// the same way.
///
/// # Cached and blocking
///
/// A read the block cache can answer never leaves the calling thread. One it
/// cannot is submitted to a pool of threads that exist to block, because a
/// tokio worker must not be the thing that waits on storage. Each read path
/// therefore tries the cache first and falls back; the `_blocking` forms skip
/// that and always block, and exist for callers already on a thread where
/// that is allowed.
pub struct Map {
    name: &'static str,
    watchers: Watchers,
    cf: Arc<ColumnFamily>,
    db: Arc<Engine>,

    /// Options for a read that may go to disk.
    read_options: ReadOptions,

    /// Options for a read that must not, which the engine answers with an
    /// `Incomplete` status rather than reading a block.
    cache_read_options: ReadOptions,

    write_options: WriteOptions,
}

impl Map {
    pub(crate) fn open(db: &Arc<Engine>, name: &'static str) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            name,
            watchers: Watchers::default(),
            cf: open::open(db, name),
            db: db.clone(),
            read_options: read_options_default(db),
            cache_read_options: cache_read_options_default(db),
            write_options: write_options_default(db),
        }))
    }

    /// A future that completes once a key beginning with `prefix` is written
    /// to this column.
    ///
    /// "Once" is narrow: the future resolves on the first matching write
    /// after it was created, and says nothing about what was written. A
    /// caller that still cares after being woken has to watch again.
    #[inline]
    pub fn watch_prefix<K>(&self, prefix: &K) -> impl Future<Output = ()> + Send + use<K>
    where
        K: AsRef<[u8]> + ?Sized,
    {
        self.watchers.watch(prefix.as_ref())
    }

    /// Queries one of this column's properties by null-terminated name, for
    /// those with an integer representation.
    #[inline]
    pub fn property_integer(&self, name: &CStr) -> Result<u64> {
        self.db.property_integer(&self.cf(), name)
    }

    /// Queries one of this column's properties by name, as a string.
    #[inline]
    pub fn property(&self, name: &str) -> Result<String> {
        self.db.property(&self.cf(), name)
    }

    #[inline]
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    #[inline]
    pub(crate) fn db(&self) -> &Arc<Engine> {
        &self.db
    }

    #[inline]
    pub(crate) fn cf(&self) -> impl AsColumnFamilyRef + '_ {
        &*self.cf
    }
}

/// Options for a read that is allowed to go to disk.
#[inline]
pub(crate) fn read_options_default(db: &Arc<Engine>) -> ReadOptions {
    let mut options = ReadOptions::default();

    // The columns are not opened with a prefix extractor, so a seek that is
    // not told to order across the whole keyspace could skip entries outside
    // whichever prefix it landed in.
    options.set_total_order_seek(true);

    if !db.checksums() {
        options.set_verify_checksums(false);
    }

    options
}

/// Options for a read that must be answered from the block cache or not at
/// all, which is how the read paths test whether they need the pool.
#[inline]
pub(crate) fn cache_read_options_default(db: &Arc<Engine>) -> ReadOptions {
    let mut options = read_options_default(db);
    options.set_read_tier(ReadTier::BlockCache);

    // A miss is going to be re-read through the pool, which will populate the
    // cache then; doing it here would evict a block to hold one we are about
    // to fetch again anyway.
    options.fill_cache(false);

    options
}

/// Options for an iteration that is allowed to go to disk.
#[inline]
pub(crate) fn iter_options_default(db: &Arc<Engine>) -> ReadOptions {
    let mut options = read_options_default(db);

    // An iterator holds pinned blocks and, where the column is being
    // compacted, obsolete files. Releasing those on a background thread keeps
    // the cost off whichever thread happened to drop the stream.
    options.set_background_purge_on_iterator_cleanup(true);

    options
}

/// Options for an iteration that must be answered from the block cache. See
/// [`cache_read_options_default`].
#[inline]
pub(crate) fn cache_iter_options_default(db: &Arc<Engine>) -> ReadOptions {
    let mut options = iter_options_default(db);
    options.set_read_tier(ReadTier::BlockCache);
    options.fill_cache(false);

    options
}

/// The engine's defaults are what phantom wants: writes go through the
/// write-ahead log, and the flush after them is decided per write by whether
/// a [`Cork`](crate::Cork) is held.
#[inline]
pub(crate) fn write_options_default(_db: &Arc<Engine>) -> WriteOptions {
    WriteOptions::default()
}

impl Debug for Map {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("Map").field("name", &self.name).finish()
    }
}

/// A column is identified by its name everywhere an operator would see it, so
/// that is what the tracing spans record.
impl Display for Map {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(self.name)
    }
}
