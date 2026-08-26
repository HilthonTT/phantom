//! A borrowed view of a value still resident in the engine's block cache.

use std::{
    convert::identity,
    fmt::{self, Debug},
    ops::Deref,
};

use phantom_core::Result;
use rocksdb::DBPinnableSlice;
use serde::{Deserialize, Serialize, Serializer};

use crate::keyval::{Slice, deserialize_val};

/// The result of a successful read.
///
/// A read does not copy the value out: it pins the cache block holding it and
/// hands back this, which borrows into that block and unpins it on drop. The
/// bytes are therefore only valid while the handle lives, which is why the
/// deserializing accessors take a closure rather than returning the borrow.
pub struct Handle<'a> {
    val: DBPinnableSlice<'a>,
}

/// Deserializing whatever a read produced, without naming where in the result
/// the bytes are.
///
/// Implemented for a handle, a reference to one, and a `Result` of either, so
/// that `.deserialized()` reads the same whether it follows a bare read or an
/// awaited one.
pub trait Deserialized {
    /// Deserializes the value and hands it to `f`.
    ///
    /// The deserialized type may borrow from the handle, so it cannot outlive
    /// this call; `f` is where anything worth keeping is copied out.
    fn map_de<T, U, F>(self, f: F) -> Result<U>
    where
        F: FnOnce(T) -> U,
        T: for<'de> Deserialize<'de>;

    /// Deserializes into an owned value.
    ///
    /// Only available where `T` borrows nothing from the handle, which is the
    /// same bound spelled out: `T: for<'de> Deserialize<'de>`.
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
    /// Deserializes the value into a type that may borrow from this handle.
    ///
    /// [`Deserialized::deserialized`] cannot do this: deserializing into a
    /// borrow needs the handle's own lifetime, and the bound that lets that
    /// method work on a `Result` as readily as on a handle — `for<'de>
    /// Deserialize<'de>` — is exactly the bound a borrowing type does not
    /// satisfy. Use this where the value is being read rather than kept.
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

/// Re-serializing a handle writes its bytes through unchanged, so a value
/// read from one column can be used to build a key or value for another
/// without a round trip through the deserializer.
impl Serialize for Handle<'_> {
    #[inline]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(self.as_ref())
    }
}

/// The bytes are elided: a handle is most often printed while tracing a read,
/// where the value could be a whole event and the identity of the block is
/// the part worth seeing.
impl Debug for Handle<'_> {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let val: &Slice = self;

        out.debug_struct("Handle")
            .field("ptr", &val.as_ptr())
            .field("len", &val.len())
            .finish()
    }
}
