//! The shapes a key and a value take on the way in and out of a column.
//!
//! Everything the engine stores is bytes, so the types here are thin: they
//! name which side of an entry a slice came from, and pair the serializer with
//! the deserializer at the two points where the map layer crosses between Rust
//! values and those bytes.

use phantom_core::Result;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::{de, ser};

/// One entry: the key it is filed under and the value stored there.
///
/// Both halves default to borrowed slices, which is what the iteration paths
/// yield. Deserializing substitutes the caller's own types.
pub type KeyVal<'a, K = &'a Slice, V = &'a Slice> = (Key<'a, K>, Val<'a, V>);

/// The key half of an entry. An alias rather than a wrapper: it exists to say
/// which half a slice came from at a callsite where both are in scope.
pub type Key<'a, T = &'a Slice> = T;

/// The value half of an entry. See [`Key`].
pub type Val<'a, T = &'a Slice> = T;

/// A buffer sized for a key, which is short enough that most stay on the
/// stack.
pub type KeyBuf = Buffer<KEY_STACK_CAP>;

/// A buffer sized for a value, which is more often large enough to spill.
pub type ValBuf = Buffer<VAL_STACK_CAP>;

/// A byte buffer that spills to the heap only once it outgrows `CAP`.
///
/// Serialization runs on the hot path of every query, so the common case of a
/// key built from a couple of identifiers should not touch the allocator.
pub type Buffer<const CAP: usize = DEF_STACK_CAP> = SmallVec<[Byte; CAP]>;

/// Borrowed bytes, whichever half of an entry they came from.
pub type Slice = [Byte];

pub type Byte = u8;

/// Bytes a key buffer holds before spilling. Sized for a room id plus an
/// event id and their separators, which covers most of the schema.
pub const KEY_STACK_CAP: usize = 128;

/// Bytes a value buffer holds before spilling. Values run to whole JSON
/// documents, so this is a compromise rather than a bound.
pub const VAL_STACK_CAP: usize = 512;

/// What [`Buffer`] holds where the caller did not say which half it is for.
pub const DEF_STACK_CAP: usize = KEY_STACK_CAP;

/// Serializes a value into a fresh key buffer.
#[inline]
pub fn serialize_key<T: Serialize>(val: T) -> Result<KeyBuf> {
    ser::serialize_to::<KeyBuf, _>(val)
}

/// Serializes a value into a fresh value buffer.
#[inline]
pub fn serialize_val<T: Serialize>(val: T) -> Result<ValBuf> {
    ser::serialize_to::<ValBuf, _>(val)
}

/// Deserializes both halves of an entry.
#[inline]
pub(crate) fn deserialize<'a, K, V>(kv: KeyVal<'a>) -> Result<KeyVal<'a, K, V>>
where
    K: Deserialize<'a>,
    V: Deserialize<'a>,
{
    Ok((deserialize_key::<K>(kv.0)?, deserialize_val::<V>(kv.1)?))
}

#[inline]
pub(crate) fn deserialize_key<'a, K>(key: Key<'a>) -> Result<Key<'a, K>>
where
    K: Deserialize<'a>,
{
    de::from_slice::<K>(key)
}

#[inline]
pub(crate) fn deserialize_val<'a, V>(val: Val<'a>) -> Result<Val<'a, V>>
where
    V: Deserialize<'a>,
{
    de::from_slice::<V>(val)
}

/// Deserializes an entry in combinator position, passing failures through
/// untouched.
///
/// This is what composes the map's raw iteration with typed results:
/// `map.raw_stream_from(&key).map(result_deserialize::<K, V>)` is the
/// serialized-query, deserialized-result combination that has no method of
/// its own.
#[inline]
pub fn result_deserialize<'a, K, V>(kv: Result<KeyVal<'a>>) -> Result<KeyVal<'a, K, V>>
where
    K: Deserialize<'a>,
    V: Deserialize<'a>,
{
    deserialize(kv?)
}

/// [`result_deserialize`] for the key half alone.
#[inline]
pub fn result_deserialize_key<'a, K>(key: Result<Key<'a>>) -> Result<Key<'a, K>>
where
    K: Deserialize<'a>,
{
    deserialize_key(key?)
}

/// Discards the value half of an entry, for `.map(keyval::key)`.
#[inline]
#[must_use]
pub fn key<K, V>(kv: KeyVal<'_, K, V>) -> Key<'_, K> {
    kv.0
}

/// Discards the key half of an entry, for `.map(keyval::val)`.
#[inline]
#[must_use]
pub fn val<K, V>(kv: KeyVal<'_, K, V>) -> Val<'_, V> {
    kv.1
}

#[cfg(test)]
mod tests {
    use crate::{Interfix, keyval::*};

    /// The point of the two buffer types: a key built from a couple of short
    /// components never reaches the allocator.
    #[test]
    fn a_short_key_stays_on_the_stack() {
        let buf = serialize_key(("!room:phantom.chat", 1_u64)).expect("serialized");

        assert!(!buf.spilled(), "a key this size should not have allocated");
        assert_eq!(buf.as_slice(), b"!room:phantom.chat\xFF\0\0\0\0\0\0\0\x01");
    }

    #[test]
    fn a_long_value_spills_to_the_heap() {
        let buf = serialize_val(("x", "y".repeat(VAL_STACK_CAP))).expect("serialized");

        assert!(buf.spilled(), "outgrowing the inline capacity must spill");
        assert_eq!(buf.len(), VAL_STACK_CAP + 2);
    }

    #[test]
    fn round_trips_through_both_halves() {
        let key = serialize_key(("@user:phantom.chat", 7_u64)).expect("serialized");
        let val = serialize_val(("!room:phantom.chat",)).expect("serialized");

        let (k, v) = deserialize::<(&str, u64), (&str,)>((&key, &val)).expect("deserialized");

        assert_eq!(k, ("@user:phantom.chat", 7));
        assert_eq!(v, ("!room:phantom.chat",));
    }

    /// A prefix is a key with the trailing separator left on, so it must not
    /// also match a longer component that merely starts the same way.
    #[test]
    fn a_prefix_key_does_not_match_a_longer_component() {
        let prefix = serialize_key(("!room", Interfix)).expect("serialized");
        let inside = serialize_key(("!room", 1_u64)).expect("serialized");
        let outside = serialize_key(("!roomier", 1_u64)).expect("serialized");

        assert!(inside.starts_with(&prefix));
        assert!(!outside.starts_with(&prefix));
    }

    #[test]
    fn key_and_val_select_a_half() {
        let kv: KeyVal<'_, u32, &str> = (1, "one");

        assert_eq!(key(kv), 1);
        assert_eq!(val(kv), "one");
    }
}
