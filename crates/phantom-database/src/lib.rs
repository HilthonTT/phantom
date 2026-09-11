mod codec;
mod cork;
mod cursor;
mod engine;
mod handle;
pub mod keyval;
mod map;
mod pool;
mod schema;
mod txn;
mod watchers;

use std::{ops::Index, sync::Arc};

use phantom_core::{Result, err, server::Server};

pub use self::{
    codec::{
        deserialize::{Ignore, IgnoreAll, from_slice as deserialize},
        serialize::{
            Cbor, Interfix, Json, SEP, Separator, serialize, serialize_to, serialize_to_vec,
        },
    },
    cork::Cork,
    engine::{Context, Engine, descriptor},
    handle::{Deserialized, Handle},
    keyval::{Key, KeyVal, Slice, Val, serialize_key, serialize_val},
    map::{Map, Qry, compact},
    txn::Txn,
};
use self::{
    engine::descriptor::Descriptor,
    schema::{Maps, MapsKey, MapsVal},
};

pub struct Database {
    maps: Maps,

    pub engine: Arc<Engine>,

    _ctx: Arc<Context>,
}

impl Database {
    pub fn open(server: &Arc<Server>) -> Result<Arc<Self>> {
        Self::open_list(server, schema::MAPS)
    }

    pub(crate) fn open_list(server: &Arc<Server>, desc: &[Descriptor]) -> Result<Arc<Self>> {
        let ctx = Context::new(server)?;
        let engine = Engine::open(ctx.clone(), desc)?;

        Ok(Arc::new(Self {
            maps: schema::open_list(&engine, desc)?,
            engine,
            _ctx: ctx,
        }))
    }

    #[inline]
    pub fn get(&self, name: &str) -> Result<&Arc<Map>> {
        self.maps
            .get(name)
            .ok_or_else(|| err!(Request(NotFound("column not found"))))
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&MapsKey, &MapsVal)> + Send + '_ {
        self.maps.iter()
    }

    #[inline]
    pub fn keys(&self) -> impl Iterator<Item = &MapsKey> + Send + '_ {
        self.maps.keys()
    }

    #[inline]
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.engine.is_read_only()
    }

    #[inline]
    #[must_use]
    pub fn is_secondary(&self) -> bool {
        self.engine.is_secondary()
    }
}

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
