mod clear;
pub mod compact;
mod contains;
mod count;
mod del_prefix;
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

pub struct Map {
    name: &'static str,
    watchers: Watchers,
    cf: Arc<ColumnFamily>,
    db: Arc<Engine>,

    read_options: ReadOptions,

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

    #[inline]
    pub fn watch_prefix<K>(&self, prefix: &K) -> impl Future<Output = ()> + Send + use<K>
    where
        K: AsRef<[u8]> + ?Sized,
    {
        self.watchers.watch(prefix.as_ref())
    }

    #[inline]
    pub fn property_integer(&self, name: &CStr) -> Result<u64> {
        self.db.property_integer(&self.cf(), name)
    }

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
    pub(crate) fn wake(&self, key: &[u8]) {
        self.watchers.wake(key);
    }

    #[inline]
    pub(crate) fn cf(&self) -> impl AsColumnFamilyRef + '_ {
        &*self.cf
    }
}

#[inline]
pub(crate) fn read_options_default(db: &Arc<Engine>) -> ReadOptions {
    let mut options = ReadOptions::default();

    options.set_total_order_seek(true);

    if !db.checksums() {
        options.set_verify_checksums(false);
    }

    options
}

#[inline]
pub(crate) fn cache_read_options_default(db: &Arc<Engine>) -> ReadOptions {
    let mut options = read_options_default(db);
    options.set_read_tier(ReadTier::BlockCache);

    options.fill_cache(false);

    options
}

#[inline]
pub(crate) fn iter_options_default(db: &Arc<Engine>) -> ReadOptions {
    let mut options = read_options_default(db);

    options.set_background_purge_on_iterator_cleanup(true);

    options
}

#[inline]
pub(crate) fn cache_iter_options_default(db: &Arc<Engine>) -> ReadOptions {
    let mut options = iter_options_default(db);
    options.set_read_tier(ReadTier::BlockCache);
    options.fill_cache(false);

    options
}

#[inline]
pub(crate) fn write_options_default(_db: &Arc<Engine>) -> WriteOptions {
    WriteOptions::default()
}

impl Debug for Map {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("Map").field("name", &self.name).finish()
    }
}

impl Display for Map {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(self.name)
    }
}
