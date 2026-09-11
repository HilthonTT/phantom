use phantom_core::Result;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::codec::{deserialize::from_slice, serialize::serialize_to};

pub type KeyVal<'a, K = &'a Slice, V = &'a Slice> = (Key<'a, K>, Val<'a, V>);

pub type Key<'a, T = &'a Slice> = T;

pub type Val<'a, T = &'a Slice> = T;

pub type KeyBuf = Buffer<KEY_STACK_CAP>;

pub type ValBuf = Buffer<VAL_STACK_CAP>;

pub type Buffer<const CAP: usize = DEF_STACK_CAP> = SmallVec<[Byte; CAP]>;

pub type Slice = [Byte];

pub type Byte = u8;

pub const KEY_STACK_CAP: usize = 128;

pub const VAL_STACK_CAP: usize = 512;

pub const DEF_STACK_CAP: usize = KEY_STACK_CAP;

#[inline]
pub fn serialize_key<T: Serialize>(val: T) -> Result<KeyBuf> {
    serialize_to::<KeyBuf, _>(val)
}

#[inline]
pub fn serialize_val<T: Serialize>(val: T) -> Result<ValBuf> {
    serialize_to::<ValBuf, _>(val)
}

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
    from_slice::<K>(key)
}

#[inline]
pub(crate) fn deserialize_val<'a, V>(val: Val<'a>) -> Result<Val<'a, V>>
where
    V: Deserialize<'a>,
{
    from_slice::<V>(val)
}

#[inline]
pub fn result_deserialize<'a, K, V>(kv: Result<KeyVal<'a>>) -> Result<KeyVal<'a, K, V>>
where
    K: Deserialize<'a>,
    V: Deserialize<'a>,
{
    deserialize(kv?)
}

#[inline]
pub fn result_deserialize_key<'a, K>(key: Result<Key<'a>>) -> Result<Key<'a, K>>
where
    K: Deserialize<'a>,
{
    deserialize_key(key?)
}

#[inline]
#[must_use]
pub fn key<K, V>(kv: KeyVal<'_, K, V>) -> Key<'_, K> {
    kv.0
}

#[inline]
#[must_use]
pub fn val<K, V>(kv: KeyVal<'_, K, V>) -> Val<'_, V> {
    kv.1
}

#[cfg(test)]
mod tests {
    use crate::{Interfix, keyval::*};

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
