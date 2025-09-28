// Copyright 2025 Oxide Computer Company

use std::{collections::BTreeMap, fmt::Display};

use serde::{
    Serialize, Serializer,
    ser::{Impossible, SerializeStruct},
};

/// Serialize an instance of T into a `BTreeMap<String, String>`.
pub(crate) fn to_map<T>(input: &T) -> Result<BTreeMap<String, String>, MapError>
where
    T: Serialize,
{
    let mut serializer = MapSerializer(input);
    input.serialize(&mut serializer)
}

struct MapSerializer<T>(T);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MapError(pub String);

impl Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl serde::ser::Error for MapError {
    fn custom<T>(msg: T) -> Self
    where
        T: std::fmt::Display,
    {
        MapError(format!("{}", msg))
    }
}

impl std::error::Error for MapError {}

macro_rules! ser_err {
    ($i:ident $(, $p:ident : $t:ty )*) => {
        fn $i(self $(, $p: $t)*) -> Result<Self::Ok, Self::Error>
        {
            Err(MapError(format!("invalid type: {}",
                stringify!($i).trim_start_matches("serialize_"))))
        }
    };
    ($i:ident $(, $p:ident : $t:ty )* ,) => {
        ser_err!($i $(, $p: $t)*);
    };

}

macro_rules! ser_t_err {
    ($i:ident $(, $p:ident : $t:ty )*) => {
        fn $i<T>(self $(, $p: $t)*) -> Result<Self::Ok, Self::Error>
        where
            T: Serialize + ?Sized,
        {
            Err(MapError(format!("invalid type: {}",
                stringify!($i).trim_start_matches("serialize_"))))
        }
    };
    ($i:ident $(, $p:ident : $t:ty )* ,) => {
        ser_t_err!($i $(, $p: $t)*);
    };

}

impl<Input> Serializer for &mut MapSerializer<Input> {
    type Ok = BTreeMap<String, String>;
    type Error = MapError;

    type SerializeStruct = MapSerializeStruct;

    type SerializeSeq = Impossible<Self::Ok, Self::Error>;
    type SerializeTuple = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleStruct = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleVariant = Impossible<Self::Ok, Self::Error>;
    type SerializeMap = Impossible<Self::Ok, Self::Error>;
    type SerializeStructVariant = Impossible<Self::Ok, Self::Error>;

    ser_err!(serialize_bool, _v: bool);
    ser_err!(serialize_i8, _v: i8);
    ser_err!(serialize_i16, _v: i16);
    ser_err!(serialize_i32, _v: i32);
    ser_err!(serialize_i64, _v: i64);
    ser_err!(serialize_u8, _v: u8);
    ser_err!(serialize_u16, _v: u16);
    ser_err!(serialize_u32, _v: u32);
    ser_err!(serialize_u64, _v: u64);
    ser_err!(serialize_f32, _v: f32);
    ser_err!(serialize_f64, _v: f64);
    ser_err!(serialize_char, _v: char);
    ser_err!(serialize_str, _v: &str);
    ser_err!(serialize_bytes, _v: &[u8]);
    ser_err!(serialize_none);
    ser_err!(serialize_unit);
    ser_t_err!(serialize_some, _value: &T);
    ser_err!(serialize_unit_struct, _name: &'static str);
    ser_err!(
        serialize_unit_variant,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    );
    ser_t_err!(
        serialize_newtype_variant,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    );
    ser_t_err!(serialize_newtype_struct, _name: &'static str, _value: &T);

    fn serialize_seq(
        self,
        _len: Option<usize>,
    ) -> Result<Self::SerializeSeq, Self::Error> {
        Err(MapError("cannot serialize a sequence".to_string()))
    }

    fn serialize_tuple(
        self,
        _len: usize,
    ) -> Result<Self::SerializeTuple, Self::Error> {
        Err(MapError("cannot serialize a tuple".to_string()))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(MapError("cannot serialize a tuple struct".to_string()))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(MapError("cannot serialize a tuple variant".to_string()))
    }

    fn serialize_map(
        self,
        _len: Option<usize>,
    ) -> Result<Self::SerializeMap, Self::Error> {
        Err(MapError("cannot serialize a map".to_string()))
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(MapSerializeStruct { output: BTreeMap::new() })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(MapError("cannot serialize a struct variant".to_string()))
    }
}

/// Used to serialize structs for `MapSerializer`.
struct MapSerializeStruct {
    output: BTreeMap<String, String>,
}

impl SerializeStruct for MapSerializeStruct {
    type Ok = BTreeMap<String, String>;
    type Error = MapError;

    fn serialize_field<T>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized,
    {
        let mut serializer = ValueSerializer { key, output: &mut self.output };
        value.serialize(&mut serializer)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.output)
    }
}

/// A trivial `Serializer` used to extract a `String`. One could imagine
/// extending this to convert other scalars into strings, but for now we'll just
/// work with strings.
struct ValueSerializer<'a> {
    key: &'a str,
    output: &'a mut BTreeMap<String, String>,
}
impl Serializer for &mut ValueSerializer<'_> {
    type Ok = ();
    type Error = MapError;

    type SerializeSeq = Impossible<Self::Ok, Self::Error>;
    type SerializeTuple = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleStruct = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleVariant = Impossible<Self::Ok, Self::Error>;
    type SerializeMap = Impossible<Self::Ok, Self::Error>;
    type SerializeStruct = Impossible<Self::Ok, Self::Error>;
    type SerializeStructVariant = Impossible<Self::Ok, Self::Error>;

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        self.output.insert(self.key.to_string(), v.to_string());
        Ok(())
    }

    ser_err!(serialize_bool, _v: bool);
    ser_err!(serialize_i8, _v: i8);
    ser_err!(serialize_i16, _v: i16);
    ser_err!(serialize_i32, _v: i32);
    ser_err!(serialize_i64, _v: i64);
    ser_err!(serialize_u8, _v: u8);
    ser_err!(serialize_u16, _v: u16);
    ser_err!(serialize_u32, _v: u32);
    ser_err!(serialize_u64, _v: u64);
    ser_err!(serialize_f32, _v: f32);
    ser_err!(serialize_f64, _v: f64);
    ser_err!(serialize_char, _v: char);
    ser_err!(serialize_bytes, _v: &[u8]);
    ser_err!(serialize_unit);
    ser_err!(serialize_unit_struct, _name: &'static str);
    ser_err!(
        serialize_unit_variant,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    );
    ser_t_err!(
        serialize_newtype_variant,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    );
    ser_t_err!(serialize_newtype_struct, _name: &'static str, _value: &T);

    fn serialize_seq(
        self,
        _len: Option<usize>,
    ) -> Result<Self::SerializeSeq, Self::Error> {
        Err(MapError("cannot serialize a sequence".to_string()))
    }

    fn serialize_tuple(
        self,
        _len: usize,
    ) -> Result<Self::SerializeTuple, Self::Error> {
        Err(MapError("cannot serialize a tuple".to_string()))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(MapError("cannot serialize a tuple struct".to_string()))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(MapError("cannot serialize a tuple variant".to_string()))
    }

    fn serialize_map(
        self,
        _len: Option<usize>,
    ) -> Result<Self::SerializeMap, Self::Error> {
        Err(MapError("cannot serialize a map".to_string()))
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(MapError("cannot serialize a struct".to_string()))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(MapError("cannot serialize a struct variant".to_string()))
    }
}

#[cfg(test)]
mod test {
    use serde::Serialize;

    use crate::to_map::{MapError, to_map};

    #[test]
    fn test_to_map_valid() {
        #[derive(Serialize)]
        struct Valid {
            a: String,
            b: String,
        }

        let valid = Valid { a: "a".to_string(), b: "b".to_string() };

        let map = to_map(&valid).unwrap();

        assert_eq!(map.get("a"), Some(&"a".to_string()));
        assert_eq!(map.get("b"), Some(&"b".to_string()));
    }

    #[test]
    fn test_to_map_seq() {
        #[derive(Serialize)]
        struct Bad {
            a: Vec<String>,
            b: String,
        }

        let bad = Bad { a: vec!["a".to_string()], b: "b".to_string() };

        assert_eq!(
            to_map(&bad),
            Err(MapError("cannot serialize a sequence".to_string()))
        );
    }

    #[test]
    fn test_to_map_num() {
        #[derive(Serialize)]
        struct Bad {
            a: String,
            b: u32,
        }

        let bad = Bad { a: "a".to_string(), b: 0xb };

        assert_eq!(
            to_map(&bad),
            Err(MapError("invalid type: u32".to_string()))
        );
    }

    #[test]
    fn test_to_map_enum() {
        #[derive(Serialize)]
        enum Bad {
            A { a: String },
        }

        let bad = Bad::A { a: "a".to_string() };

        assert_eq!(
            to_map(&bad),
            Err(MapError("cannot serialize a struct variant".to_string()))
        );
    }

    #[test]
    fn test_to_map_vec() {
        let bad = vec!["a", "b"];

        assert_eq!(
            to_map(&bad),
            Err(MapError("cannot serialize a sequence".to_string()))
        );
    }

    #[test]
    fn test_struct_with_options() {
        #[derive(Serialize)]
        struct X {
            a: String,
            b: Option<String>,
            c: Option<String>,
        }

        let x = X { a: "A".to_string(), b: Some("B".to_string()), c: None };

        let map = to_map(&x).expect("should be a valid map");
        assert_eq!(map.get("a"), Some(&"A".to_string()));
        assert_eq!(map.get("b"), Some(&"B".to_string()));
        assert_eq!(map.get("c"), None);
    }
}
