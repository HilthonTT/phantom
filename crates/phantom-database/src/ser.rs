use std::io::Write;

use phantom_core::{Error, Result, debug::type_name, err, result::DebugInspect, utils::exchange};
use serde::{Deserialize, Serialize, ser};

use crate::util::unhandled;

#[inline]
pub fn serialize_to_vec<T: Serialize>(val: T) -> Result<Vec<u8>> {
    serialize_to::<Vec<u8>, T>(val)
}

#[inline]
pub fn serialize_to<B, T>(val: T) -> Result<B>
where
    B: Default + Write + AsRef<[u8]>,
    T: Serialize,
{
    let mut buf = B::default();
    serialize(&mut buf, val)?;

    Ok(buf)
}

/// Serialize T into Writer W
#[inline]
#[cfg_attr(unabridged, tracing::instrument(level = "trace", skip_all))]
pub fn serialize<'a, W, T>(out: &'a mut W, val: T) -> Result<&'a [u8]>
where
    W: Write + AsRef<[u8]> + 'a,
    T: Serialize,
{
    let mut serializer = Serializer {
        out,
        depth: 0,
        sep: false,
        fin: false,
    };

    val.serialize(&mut serializer)
        .map_err(|error| err!(SerdeSer("{error}")))
        .debug_inspect(|()| {
            debug_assert_eq!(
                serializer.depth, 0,
                "Serialization completed at non-zero recursion level"
            );
        })?;

    Ok((*out).as_ref())
}

pub(crate) struct Serializer<'a, W: Write> {
    out: &'a mut W,
    depth: u32,
    sep: bool,
    fin: bool,
}

/// Newtype for JSON serialization.
#[derive(Debug, Serialize)]
pub struct Json<T>(pub T);

/// Newtype for CBOR serialization.
#[derive(Debug, Deserialize, Serialize)]
pub struct Cbor<T>(pub T);

/// Directive to force separator serialization specifically for prefix keying
/// use. This is a quirk of the database schema and prefix iterations.
#[derive(Debug, Serialize)]
pub struct Interfix;

/// Directive to force separator serialization. Separators are usually
/// serialized automatically.
#[derive(Debug, Serialize)]
pub struct Separator;

/// Record separator; an intentionally invalid-utf8 byte.
pub const SEP: u8 = b'\xFF';

impl<W: Write> Serializer<'_, W> {
    const SEP: &'static [u8] = &[SEP];

    /// A key is one flat record stream: `sep` is set once the first component
    /// has been written and stays set, so every later component is preceded by
    /// a separator. Nesting a tuple inside a tuple would serialize to the same
    /// bytes as the flattened form, which is a schema the caller did not write;
    /// this catches it rather than letting the two shapes collide on disk.
    fn tuple_start(&mut self) {
        debug_assert!(!self.sep, "Tuple start with separator set");
        self.sequence_start();
    }

    fn tuple_end(&mut self) -> Result {
        self.sequence_end()?;
        Ok(())
    }

    fn sequence_start(&mut self) {
        debug_assert!(!self.is_finalized(), "Sequence start with finalization set");
        cfg!(debug_assertions).then(|| self.depth = self.depth.saturating_add(1));
    }

    fn sequence_end(&mut self) -> Result {
        cfg!(debug_assertions).then(|| self.depth = self.depth.saturating_sub(1));
        Ok(())
    }

    fn record_start(&mut self) -> Result {
        debug_assert!(
            !self.is_finalized(),
            "Starting a record after serialization finalized"
        );
        if exchange(&mut self.sep, true) {
            self.separator()
        } else {
            Ok(())
        }
    }

    fn separator(&mut self) -> Result {
        debug_assert!(
            !self.is_finalized(),
            "Writing a separator after serialization finalized"
        );
        self.out.write_all(Self::SEP).map_err(Into::into)
    }

    fn write(&mut self, buf: &[u8]) -> Result {
        debug_assert!(
            !self.is_finalized(),
            "Writing after serialization finalized"
        );
        self.out.write_all(buf).map_err(Into::into)
    }

    fn set_finalized(&mut self) {
        debug_assert!(!self.is_finalized(), "Finalization already set");
        cfg!(debug_assertions).then(|| self.fin = true);
    }

    fn is_finalized(&self) -> bool {
        self.fin
    }
}

impl<W: Write> ser::Serializer for &mut Serializer<'_, W> {
    type Error = Error;
    type Ok = ();
    type SerializeMap = Self;
    type SerializeSeq = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq> {
        self.sequence_start();
        Ok(self)
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple> {
        self.tuple_start();
        Ok(self)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct> {
        self.tuple_start();
        Ok(self)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _idx: u32,
        _var: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        unhandled!("serialize Tuple Variant not implemented")
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap> {
        unhandled!(
            "serialize Map not implemented; did you mean to use database::Json() around your \
			 serde_json::Value?"
        )
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Self::SerializeStruct> {
        unhandled!(
            "serialize Struct not implemented at this time; did you mean to use \
			 database::Json() around your struct?"
        )
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _idx: u32,
        _var: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant> {
        unhandled!("serialize Struct Variant not implemented")
    }

    #[allow(clippy::needless_borrows_for_generic_args)] // buggy
    fn serialize_newtype_struct<T>(self, name: &'static str, value: &T) -> Result<Self::Ok>
    where
        T: Serialize + ?Sized,
    {
        debug_assert!(
            name != "Json" || type_name::<T>() != "alloc::boxed::Box<serde_json::raw::RawValue>",
            "serializing a Json(RawValue); you can skip serialization instead"
        );

        match name {
            "Json" => serde_json::to_writer(&mut self.out, value).map_err(Into::into),
            "Cbor" => {
                use minicbor::encode::write::Writer;
                use minicbor_serde::Serializer;

                value
                    .serialize(&mut Serializer::new(&mut Writer::new(&mut self.out)))
                    .map_err(|e| Self::Error::SerdeSer(e.to_string().into()))
            }
            _ => unhandled!("Unrecognized serialization Newtype {name:?}"),
        }
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _idx: u32,
        _var: &'static str,
        _value: &T,
    ) -> Result<Self::Ok> {
        unhandled!("serialize Newtype Variant not implemented")
    }

    fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok> {
        match name {
            "Interfix" => {
                self.set_finalized();
            }
            "Separator" => {
                self.separator()?;
            }
            _ => unhandled!("Unrecognized serialization directive: {name:?}"),
        }

        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _idx: u32,
        _var: &'static str,
    ) -> Result<Self::Ok> {
        unhandled!("serialize Unit Variant not implemented")
    }

    fn serialize_some<T: Serialize + ?Sized>(self, val: &T) -> Result<Self::Ok> {
        val.serialize(self)
    }

    fn serialize_none(self) -> Result<Self::Ok> {
        Ok(())
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok> {
        let mut buf: [u8; 4] = [0; 4];
        self.serialize_str(v.encode_utf8(&mut buf))
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok> {
        debug_assert!(
            self.depth > 0,
            "serializing string at the top-level; you can skip serialization instead"
        );

        self.serialize_bytes(v.as_bytes())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok> {
        debug_assert!(
            self.depth > 0,
            "serializing byte array at the top-level; you can skip serialization instead"
        );

        self.write(v)
    }

    fn serialize_f64(self, _v: f64) -> Result<Self::Ok> {
        unhandled!("serialize f64 not implemented")
    }

    fn serialize_f32(self, _v: f32) -> Result<Self::Ok> {
        unhandled!("serialize f32 not implemented")
    }

    fn serialize_i64(self, v: i64) -> Result<Self::Ok> {
        self.write(&v.to_be_bytes())
    }

    fn serialize_i32(self, v: i32) -> Result<Self::Ok> {
        self.write(&v.to_be_bytes())
    }

    fn serialize_i16(self, _v: i16) -> Result<Self::Ok> {
        unhandled!("serialize i16 not implemented")
    }

    fn serialize_i8(self, _v: i8) -> Result<Self::Ok> {
        unhandled!("serialize i8 not implemented")
    }

    fn serialize_u64(self, v: u64) -> Result<Self::Ok> {
        self.write(&v.to_be_bytes())
    }

    fn serialize_u32(self, v: u32) -> Result<Self::Ok> {
        self.write(&v.to_be_bytes())
    }

    fn serialize_u16(self, _v: u16) -> Result<Self::Ok> {
        unhandled!("serialize u16 not implemented")
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok> {
        self.write(&[v])
    }

    fn serialize_bool(self, _v: bool) -> Result<Self::Ok> {
        unhandled!("serialize bool not implemented")
    }

    fn serialize_unit(self) -> Result<Self::Ok> {
        unhandled!("serialize unit not implemented")
    }
}

impl<W: Write> ser::SerializeSeq for &mut Serializer<'_, W> {
    type Error = Error;
    type Ok = ();

    fn serialize_element<T: Serialize + ?Sized>(&mut self, val: &T) -> Result<Self::Ok> {
        val.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok> {
        self.sequence_end()
    }
}

impl<W: Write> ser::SerializeTuple for &mut Serializer<'_, W> {
    type Error = Error;
    type Ok = ();

    fn serialize_element<T: Serialize + ?Sized>(&mut self, val: &T) -> Result<Self::Ok> {
        self.record_start()?;
        val.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok> {
        self.tuple_end()
    }
}

impl<W: Write> ser::SerializeTupleStruct for &mut Serializer<'_, W> {
    type Error = Error;
    type Ok = ();

    fn serialize_field<T: Serialize + ?Sized>(&mut self, val: &T) -> Result<Self::Ok> {
        self.record_start()?;
        val.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok> {
        self.tuple_end()
    }
}

impl<W: Write> ser::SerializeTupleVariant for &mut Serializer<'_, W> {
    type Error = Error;
    type Ok = ();

    fn serialize_field<T: Serialize + ?Sized>(&mut self, val: &T) -> Result<Self::Ok> {
        self.record_start()?;
        val.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok> {
        self.tuple_end()
    }
}

impl<W: Write> ser::SerializeMap for &mut Serializer<'_, W> {
    type Error = Error;
    type Ok = ();

    fn serialize_key<T: Serialize + ?Sized>(&mut self, _key: &T) -> Result<Self::Ok> {
        unhandled!("serialize Map Key not implemented")
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, _val: &T) -> Result<Self::Ok> {
        unhandled!("serialize Map Val not implemented")
    }

    fn end(self) -> Result<Self::Ok> {
        unhandled!("serialize Map End not implemented")
    }
}

impl<W: Write> ser::SerializeStruct for &mut Serializer<'_, W> {
    type Error = Error;
    type Ok = ();

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _key: &'static str,
        _val: &T,
    ) -> Result<Self::Ok> {
        unhandled!("serialize Struct Field not implemented")
    }

    fn end(self) -> Result<Self::Ok> {
        unhandled!("serialize Struct End not implemented")
    }
}

impl<W: Write> ser::SerializeStructVariant for &mut Serializer<'_, W> {
    type Error = Error;
    type Ok = ();

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _key: &'static str,
        _val: &T,
    ) -> Result<Self::Ok> {
        unhandled!("serialize Struct Variant Field not implemented")
    }

    fn end(self) -> Result<Self::Ok> {
        unhandled!("serialize Struct Variant End not implemented")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ser<T: Serialize>(val: T) -> Vec<u8> {
        serialize_to_vec(val).expect("serialized")
    }

    /// Byte order is what orders the keyspace, so integers have to go out
    /// most-significant byte first or the engine iterates them out of order.
    #[test]
    fn integers_are_big_endian() {
        assert_eq!(ser(1_u64), [0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(ser(1_u32), [0, 0, 0, 1]);
        assert_eq!(ser(-1_i32), [0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(ser(7_u8), [7]);

        assert!(ser(1_u64) < ser(2_u64), "byte order follows numeric order");
    }

    #[test]
    fn tuple_components_are_separated() {
        assert_eq!(ser(("ab", "cd")), b"ab\xFFcd");
        assert_eq!(ser(("ab", "cd", "ef")), b"ab\xFFcd\xFFef");
    }

    /// What makes a key usable as an iteration prefix: the trailing separator
    /// stops `ab` from also matching `abc`.
    #[test]
    fn interfix_leaves_a_trailing_separator() {
        assert_eq!(ser(("ab", Interfix)), b"ab\xFF");
    }

    #[test]
    fn separator_is_written_where_it_is_asked_for() {
        assert_eq!(ser(("ab", Separator, "cd")), b"ab\xFF\xFF\xFFcd");
    }

    /// A sequence is one component, not several: its elements run together so
    /// that a byte string survives the round trip whatever it holds.
    #[test]
    fn sequence_elements_are_not_separated() {
        assert_eq!(ser((vec![1_u8, 2, 3], "x")), b"\x01\x02\x03\xFFx");
    }

    #[test]
    fn json_newtype_writes_json() {
        assert_eq!(
            ser(("k", Json(serde_json::json!({ "a": 1 })))),
            b"k\xFF{\"a\":1}"
        );
    }

    #[test]
    fn cbor_newtype_writes_cbor() {
        // 0x18 0x2A is CBOR for the unsigned integer 42.
        assert_eq!(ser(("k", Cbor(42_u32))), b"k\xFF\x18\x2A");
    }

    #[test]
    fn serialize_fills_the_callers_buffer() {
        let mut buf = Vec::new();
        let key = serialize(&mut buf, ("ab", "cd")).expect("serialized");

        assert_eq!(key, b"ab\xFFcd");
        assert_eq!(buf, b"ab\xFFcd");
    }

    /// `None` contributes nothing, so an optional component is absent rather
    /// than empty. The separator around it still goes out.
    #[test]
    fn none_serializes_to_nothing() {
        assert_eq!(ser(("ab", None::<u64>, "cd")), b"ab\xFF\xFFcd");
        assert_eq!(ser(("ab", Some(1_u8), "cd")), b"ab\xFF\x01\xFFcd");
    }

    /// Nesting would serialize to the same bytes as the flattened tuple, so
    /// the two shapes would collide on disk.
    #[test]
    #[should_panic(expected = "Tuple start with separator set")]
    #[cfg(debug_assertions)]
    fn nested_tuples_are_rejected() {
        let _ = serialize_to_vec((("ab", "cd"), "ef"));
    }

    /// `Interfix` says the key ends here; anything after it would land outside
    /// the prefix the caller asked for.
    #[test]
    #[should_panic(expected = "after serialization finalized")]
    #[cfg(debug_assertions)]
    fn nothing_may_follow_an_interfix() {
        let _ = serialize_to_vec(("ab", Interfix, "cd"));
    }
}
