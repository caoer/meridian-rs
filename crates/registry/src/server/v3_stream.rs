//! Streaming v3 response serializer: renames the top-level `body` fingerprint
//! keys (`wire_serve::rev::RESPONSE_KEYS`) and appends `meta.duration_us` in a
//! single `serde` pass without materializing an intermediate `serde_json::Value`
//! DOM.
//!
//! `Hello` (cap-list rewriting), `Diff` (nested delta frames), and `Error`
//! (error-code remapping) stay on `wire_serve::rev::project_response`, keeping
//! those rules in one single source of truth; every hot data body (`Cat`,
//! `Read`, `Toc`, `Extract`, `Links`, `Walk`, `Pins`, `Sql`, `Splice`, …)
//! streams straight into the output JSON string.

use serde::Serialize;
use serde::ser::{self, SerializeMap, SerializeStruct, SerializeStructVariant, Serializer};
use wire::{RequestId, Response, ResponseBody, ResponsePayload};

/// Serialize a v3 wire response directly when it is a data body, or fall back
/// to the DOM projector for `Hello`, `Diff`, and `Error`.
pub(super) fn serialize_v3(response: &Response, duration_us: Option<u64>) -> String {
    match &response.payload {
        ResponsePayload::Body { body }
            if !matches!(body, ResponseBody::Hello { .. } | ResponseBody::Diff { .. }) =>
        {
            let frame = V3DataFrame {
                id: response.id,
                ok: response.ok,
                body: V3Body(body),
                meta: duration_us.map(|duration_us| V3Meta { duration_us }),
            };
            serde_json::to_string(&frame).expect("wire response serializes")
        }
        _ => {
            let mut v = serde_json::to_value(response).expect("wire response serializes");
            wire_serve::rev::project_response(&mut v);
            if let Some(us) = duration_us {
                wire_serve::rev::attach_meta(&mut v, us);
            }
            serde_json::to_string(&v).expect("wire response serializes")
        }
    }
}

#[derive(Serialize)]
struct V3DataFrame<'a> {
    id: Option<RequestId>,
    ok: bool,
    body: V3Body<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<V3Meta>,
}

#[derive(Serialize)]
struct V3Meta {
    duration_us: u64,
}

struct V3Body<'a>(&'a ResponseBody);

impl Serialize for V3Body<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(TopLevelKeyRenamer(serializer))
    }
}

#[inline]
fn rename_key_static(key: &'static str) -> &'static str {
    for (v2, v3) in wire_serve::rev::RESPONSE_KEYS {
        if key == v2 {
            return v3;
        }
    }
    key
}

#[inline]
fn rename_key_str(key: &str) -> &str {
    for (v2, v3) in wire_serve::rev::RESPONSE_KEYS {
        if key == v2 {
            return v3;
        }
    }
    key
}

/// Wraps a serializer so that ONLY the outermost struct/map keys of a
/// `ResponseBody` are renamed from v2 to v3 vocabulary. Nested values serialize
/// directly through the inner serializer untouched.
struct TopLevelKeyRenamer<S>(S);

impl<S: Serializer> Serializer for TopLevelKeyRenamer<S> {
    type Ok = S::Ok;
    type Error = S::Error;
    type SerializeSeq = S::SerializeSeq;
    type SerializeTuple = S::SerializeTuple;
    type SerializeTupleStruct = S::SerializeTupleStruct;
    type SerializeTupleVariant = S::SerializeTupleVariant;
    type SerializeMap = RenameMap<S::SerializeMap>;
    type SerializeStruct = RenameStruct<S::SerializeStruct>;
    type SerializeStructVariant = RenameStructVariant<S::SerializeStructVariant>;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_bool(v)
    }
    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_i8(v)
    }
    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_i16(v)
    }
    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_i32(v)
    }
    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_i64(v)
    }
    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_u8(v)
    }
    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_u16(v)
    }
    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_u32(v)
    }
    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_u64(v)
    }
    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_f32(v)
    }
    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_f64(v)
    }
    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_char(v)
    }
    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_str(v)
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_bytes(v)
    }
    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_none()
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_some(value)
    }
    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_unit()
    }
    fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_unit_struct(name)
    }
    fn serialize_unit_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_unit_variant(name, variant_index, variant)
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_newtype_struct(name, value)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.0
            .serialize_newtype_variant(name, variant_index, variant, value)
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        self.0.serialize_seq(len)
    }
    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.0.serialize_tuple(len)
    }
    fn serialize_tuple_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.0.serialize_tuple_struct(name, len)
    }
    fn serialize_tuple_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        self.0
            .serialize_tuple_variant(name, variant_index, variant, len)
    }
    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(RenameMap(self.0.serialize_map(len)?))
    }
    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(RenameStruct(self.0.serialize_struct(name, len)?))
    }
    fn serialize_struct_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(RenameStructVariant(self.0.serialize_struct_variant(
            name,
            variant_index,
            variant,
            len,
        )?))
    }
}

struct RenameStruct<T>(T);

impl<T: SerializeStruct> SerializeStruct for RenameStruct<T> {
    type Ok = T::Ok;
    type Error = T::Error;

    fn serialize_field<V: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &V,
    ) -> Result<(), Self::Error> {
        self.0.serialize_field(rename_key_static(key), value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.0.end()
    }
}

struct RenameStructVariant<T>(T);

impl<T: SerializeStructVariant> SerializeStructVariant for RenameStructVariant<T> {
    type Ok = T::Ok;
    type Error = T::Error;

    fn serialize_field<V: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &V,
    ) -> Result<(), Self::Error> {
        self.0.serialize_field(rename_key_static(key), value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.0.end()
    }
}

struct RenameMap<T>(T);

impl<T: SerializeMap> SerializeMap for RenameMap<T> {
    type Ok = T::Ok;
    type Error = T::Error;

    fn serialize_key<K: ?Sized + Serialize>(&mut self, key: &K) -> Result<(), Self::Error> {
        self.0.serialize_key(&RenamedKey(key))
    }

    fn serialize_value<V: ?Sized + Serialize>(&mut self, value: &V) -> Result<(), Self::Error> {
        self.0.serialize_value(value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.0.end()
    }
}

struct RenamedKey<'a, K: ?Sized>(&'a K);

impl<K: ?Sized + Serialize> Serialize for RenamedKey<'_, K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(KeyStrRenamer(serializer))
    }
}

struct KeyStrRenamer<S>(S);

impl<S: Serializer> Serializer for KeyStrRenamer<S> {
    type Ok = S::Ok;
    type Error = S::Error;
    type SerializeSeq = ser::Impossible<S::Ok, S::Error>;
    type SerializeTuple = ser::Impossible<S::Ok, S::Error>;
    type SerializeTupleStruct = ser::Impossible<S::Ok, S::Error>;
    type SerializeTupleVariant = ser::Impossible<S::Ok, S::Error>;
    type SerializeMap = ser::Impossible<S::Ok, S::Error>;
    type SerializeStruct = ser::Impossible<S::Ok, S::Error>;
    type SerializeStructVariant = ser::Impossible<S::Ok, S::Error>;

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_str(rename_key_str(v))
    }

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_bool(v)
    }
    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_i8(v)
    }
    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_i16(v)
    }
    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_i32(v)
    }
    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_i64(v)
    }
    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_u8(v)
    }
    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_u16(v)
    }
    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_u32(v)
    }
    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_u64(v)
    }
    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_f32(v)
    }
    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_f64(v)
    }
    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_char(v)
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_bytes(v)
    }
    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_none()
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_some(value)
    }
    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_unit()
    }
    fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_unit_struct(name)
    }
    fn serialize_unit_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_unit_variant(name, variant_index, variant)
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_newtype_struct(name, value)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.0
            .serialize_newtype_variant(name, variant_index, variant, value)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(ser::Error::custom("map key must be scalar"))
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(ser::Error::custom("map key must be scalar"))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(ser::Error::custom("map key must be scalar"))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(ser::Error::custom("map key must be scalar"))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(ser::Error::custom("map key must be scalar"))
    }
    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(ser::Error::custom("map key must be scalar"))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(ser::Error::custom("map key must be scalar"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use wire::{ErrorBody, ErrorCode, NodeRev, Path, Root, Span};

    fn dom_reference(response: &Response, duration_us: Option<u64>) -> Value {
        let mut v = serde_json::to_value(response).unwrap();
        wire_serve::rev::project_response(&mut v);
        if let Some(us) = duration_us {
            wire_serve::rev::attach_meta(&mut v, us);
        }
        v
    }

    fn assert_matches_dom(response: &Response, duration_us: Option<u64>) {
        let streamed: Value = serde_json::from_str(&serialize_v3(response, duration_us)).unwrap();
        let expected = dom_reference(response, duration_us);
        assert_eq!(streamed, expected);
    }

    #[test]
    fn streaming_v3_matches_dom_projector_across_body_shapes() {
        let cases = [
            Response {
                id: Some(7),
                ok: true,
                payload: ResponsePayload::Body {
                    body: ResponseBody::Root {
                        root: Root("blake3:abc".into()),
                        seq: 42,
                        tree_instance: None,
                        scope: None,
                        scope_bytes: None,
                    },
                },
            },
            Response {
                id: Some(8),
                ok: true,
                payload: ResponsePayload::Body {
                    body: ResponseBody::Cat {
                        content: "# Hello\n".into(),
                        span: Span(0, 8),
                        node_rev: NodeRev("deadbeef".into()),
                    },
                },
            },
            Response {
                id: None,
                ok: true,
                payload: ResponsePayload::Body {
                    body: ResponseBody::Toc {
                        path: Path("notes/root.md".into()),
                        root: Root("blake3:def".into()),
                        file_rev: NodeRev("cafebabe".into()),
                        nodes: Vec::new(),
                    },
                },
            },
            Response {
                id: Some(9),
                ok: true,
                payload: ResponsePayload::Body {
                    body: ResponseBody::Sql {
                        as_of_fingerprint: "blake3:111".into(),
                        live: Some("blake3:111".into()),
                        state: "FRESH_AT_SAMPLE".into(),
                        columns: Vec::new(),
                        rows: vec![vec![serde_json::json!({"root": "must_not_rename"})]],
                        row_count: 1,
                        error: None,
                    },
                },
            },
            Response {
                id: Some(10),
                ok: false,
                payload: ResponsePayload::Error {
                    error: ErrorBody::new(ErrorCode::RootMismatch),
                },
            },
        ];

        for resp in &cases {
            assert_matches_dom(resp, None);
            assert_matches_dom(resp, Some(1234));
        }
    }
}
