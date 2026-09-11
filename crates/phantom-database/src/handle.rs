use std::{
    convert::identity,
    fmt::{self, Debug},
    ops::Deref,
};

use phantom_core::Result;
use rocksdb::DBPinnableSlice;
use serde::{Deserialize, Serialize, Serializer};

use crate::keyval::{Slice, deserialize_val};

pub struct Handle<'a> {
    val: DBPinnableSlice<'a>,
}

pub trait Deserialized {
    fn map_de<T, U, F>(self, f: F) -> Result<U>
    where
        F: FnOnce(T) -> U,
        T: for<'de> Deserialize<'de>;

    #[inline]
    fn deserialized<T>(self) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
        Self: Sized,
    {
        self.map_de(identity::<T>)
    }
}

impl Handle<'_> {
    #[inline]
    pub fn de<'de, T>(&'de self) -> Result<T>
    where
        T: Deserialize<'de>,
    {
        deserialize_val(self.as_ref())
    }
}

impl<'a> Deserialized for &'a Handle<'a> {
    #[inline]
    fn map_de<T, U, F>(self, f: F) -> Result<U>
    where
        F: FnOnce(T) -> U,
        T: for<'de> Deserialize<'de>,
    {
        deserialize_val(self.as_ref()).map(f)
    }
}

impl Deserialized for Handle<'_> {
    #[inline]
    fn map_de<T, U, F>(self, f: F) -> Result<U>
    where
        F: FnOnce(T) -> U,
        T: for<'de> Deserialize<'de>,
    {
        (&self).map_de(f)
    }
}

impl Deserialized for Result<Handle<'_>> {
    #[inline]
    fn map_de<T, U, F>(self, f: F) -> Result<U>
    where
        F: FnOnce(T) -> U,
        T: for<'de> Deserialize<'de>,
    {
        self?.map_de(f)
    }
}

impl<'a> Deserialized for Result<&'a Handle<'a>> {
    #[inline]
    fn map_de<T, U, F>(self, f: F) -> Result<U>
    where
        F: FnOnce(T) -> U,
        T: for<'de> Deserialize<'de>,
    {
        self.and_then(|handle| handle.map_de(f))
    }
}

impl<'a> From<DBPinnableSlice<'a>> for Handle<'a> {
    #[inline]
    fn from(val: DBPinnableSlice<'a>) -> Self {
        Self { val }
    }
}

impl From<Handle<'_>> for Vec<u8> {
    #[inline]
    fn from(handle: Handle<'_>) -> Self {
        handle.to_vec()
    }
}

impl Deref for Handle<'_> {
    type Target = Slice;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.val
    }
}

impl AsRef<Slice> for Handle<'_> {
    #[inline]
    fn as_ref(&self) -> &Slice {
        &self.val
    }
}

impl Serialize for Handle<'_> {
    #[inline]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(self.as_ref())
    }
}

impl Debug for Handle<'_> {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let val: &Slice = self;

        out.debug_struct("Handle")
            .field("ptr", &val.as_ptr())
            .field("len", &val.len())
            .finish()
    }
}
