//! Persistence layer for the phantom homeserver.
//!
//! This crate owns the on-disk state: the key-value engine, the typed maps
//! layered over it, and the (de)serialization that turns Matrix types into
//! keys and values. Everything above it — services, the API surface — talks to
//! storage only through the handles exported here, so the choice of engine
//! stays an implementation detail of this crate.
//!
//! # The layers
//!
//! - [`Engine`] is the open database: the operations that act on it as a whole
//!   — flushing, compaction, backup, the properties an operator queries.
//! - [`Map`] is one column of it, and is where nearly everything above this
//!   crate does its work. See its module docs for the shape of that surface.
//! - [`Database`] holds the engine and every column open on it, and is what a
//!   server hands around.
//! - The serializer and deserializer are what turn Rust values into keys
//!   and values — see [`serialize`] and [`Interfix`]. Iteration order is
//!   byte order, so how a key is written decides what ranges of it can be
//!   asked for.
//! - Behind all of it is a thread pool a read is offloaded to when the
//!   block cache cannot answer it without blocking.

mod codec;
mod cork;
mod cursor;
mod engine;
mod handle;
pub mod keyval;
mod map;
mod pool;
mod schema;
mod watchers;

use std::{ops::Index, sync::Arc};

use phantom_core::{Result, err, server::Server};

pub use self::{
    codec::{
        deserialize::{Ignore, IgnoreAll},
        serialize::{
            Cbor, Interfix, Json, SEP, Separator, serialize, serialize_to, serialize_to_vec,
        },
    },
    cork::Cork,
    engine::{Context, Engine, descriptor},
    handle::{Deserialized, Handle},
    keyval::{Key, KeyVal, Slice, Val, serialize_key, serialize_val},
    map::{Map, compact},
};
use self::{
    engine::descriptor::Descriptor,
    schema::{Maps, MapsKey, MapsVal},
};

/// The open database, and every column on it.
pub struct Database {
    maps: Maps,

    /// The engine the columns live in. Public because the whole-database
    /// operations an operator drives are on it rather than here.
    pub db: Arc<Engine>,

    /// Held so that the caches and the environment's background threads
    /// outlive the database that was opened against them.
    _ctx: Arc<Context>,
}

impl Database {
    /// Opens the database at the configured path, creating it and any column
    /// it is missing.
    pub fn open(server: &Arc<Server>) -> Result<Arc<Self>> {
        Self::open_list(server, schema::MAPS)
    }

    /// [`Self::open`] with the columns given rather than the schema's, for
    /// tests that want a database of two columns instead of ninety.
    pub(crate) fn open_list(server: &Arc<Server>, desc: &[Descriptor]) -> Result<Arc<Self>> {
        let ctx = Context::new(server)?;
        let db = Engine::open(ctx.clone(), desc)?;

        Ok(Arc::new(Self {
            maps: schema::open_list(&db, desc)?,
            db,
            _ctx: ctx,
        }))
    }

    /// The column of that name.
    ///
    /// Columns come from a static table, so a miss is a programming error;
    /// [`Index`] is the shorthand where the name is a literal.
    #[inline]
    pub fn get(&self, name: &str) -> Result<&Arc<Map>> {
        self.maps
            .get(name)
            .ok_or_else(|| err!(Request(NotFound("column not found"))))
    }

    /// Every column, by name.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&MapsKey, &MapsVal)> + Send + '_ {
        self.maps.iter()
    }

    /// The name of every column.
    #[inline]
    pub fn keys(&self) -> impl Iterator<Item = &MapsKey> + Send + '_ {
        self.maps.keys()
    }

    #[inline]
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.db.is_read_only()
    }

    #[inline]
    #[must_use]
    pub fn is_secondary(&self) -> bool {
        self.db.is_secondary()
    }
}

/// # Panics
///
/// If no column of that name exists. For the fallible form, see
/// [`Database::get`].
impl Index<&str> for Database {
    type Output = Arc<Map>;

    fn index(&self, name: &str) -> &Self::Output {
        self.maps
            .get(name)
            .expect("column does not exist in database")
    }
}

#[cfg(test)]
mod tests;
