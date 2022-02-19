// Copyright 2022 Oxide Computer Company

use std::{collections::BTreeMap, fmt::Display, marker::PhantomData};

use serde::{
    ser::{
        SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
        SerializeTuple, SerializeTupleStruct, SerializeTupleVariant,
    },
    Serialize, Serializer,
};

pub(crate) fn to_map<T: Serialize>(
    input: &T,
) -> Result<BTreeMap<String, String>, MapError>
where
    T: Serialize,
{
    let mut serializer = MapSerializer(input);

    input.serialize(&mut serializer)
}

struct MapSerializer<'de, T>(&'de T);

#[derive(Clone, Debug)]
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
        fn $i<T: ?Sized>(self $(, $p: $t)*) -> Result<Self::Ok, Self::Error>
        where
            T: Serialize,
        {
            Err(MapError(format!("invalid type: {}",
                stringify!($i).trim_start_matches("serialize_"))))
        }
    };
    ($i:ident $(, $p:ident : $t:ty )* ,) => {
        ser_t_err!($i $(, $p: $t)*);
    };

}

impl<'de, 'a, Input> Serializer for &'a mut MapSerializer<'de, Input> {
    type Ok = BTreeMap<String, String>;
    type Error = MapError;

    type SerializeStruct = MapSerializeStruct;

    type SerializeSeq = &'a mut UnreachableSerializer<Self::Ok>;
    type SerializeTuple = &'a mut UnreachableSerializer<Self::Ok>;
    type SerializeTupleStruct = &'a mut UnreachableSerializer<Self::Ok>;
    type SerializeTupleVariant = &'a mut UnreachableSerializer<Self::Ok>;
    type SerializeMap = &'a mut UnreachableSerializer<Self::Ok>;
    type SerializeStructVariant = &'a mut UnreachableSerializer<Self::Ok>;

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
        todo!()
    }

    fn serialize_tuple(
        self,
        _len: usize,
    ) -> Result<Self::SerializeTuple, Self::Error> {
        todo!()
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        todo!()
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        todo!()
    }

    fn serialize_map(
        self,
        _len: Option<usize>,
    ) -> Result<Self::SerializeMap, Self::Error> {
        todo!()
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
        todo!()
    }
}

struct UnreachableSerializer<T>(PhantomData<T>);

impl<'a, T> SerializeSeq for &'a mut UnreachableSerializer<T> {
    type Ok = T;

    type Error = MapError;

    fn serialize_element<E: ?Sized>(
        &mut self,
        _value: &E,
    ) -> Result<(), Self::Error>
    where
        E: Serialize,
    {
        unreachable!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }
}

impl<'a, Output> SerializeTuple for &'a mut UnreachableSerializer<Output> {
    type Ok = Output;

    type Error = MapError;

    fn serialize_element<T: ?Sized>(
        &mut self,
        _value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        unreachable!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }
}

impl<'a, Output> SerializeTupleStruct
    for &'a mut UnreachableSerializer<Output>
{
    type Ok = Output;

    type Error = MapError;

    fn serialize_field<T: ?Sized>(
        &mut self,
        _value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        unreachable!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }
}

impl<'a, Output> SerializeTupleVariant
    for &'a mut UnreachableSerializer<Output>
{
    type Ok = Output;

    type Error = MapError;

    fn serialize_field<T: ?Sized>(
        &mut self,
        _value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        unreachable!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }
}

impl<'a, Output> SerializeMap for &'a mut UnreachableSerializer<Output> {
    type Ok = Output;

    type Error = MapError;

    fn serialize_key<T: ?Sized>(&mut self, _key: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        unreachable!()
    }

    fn serialize_value<T: ?Sized>(
        &mut self,
        _value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        unreachable!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        todo!()
    }
}
impl<'a, Output> SerializeStruct for &'a mut UnreachableSerializer<Output> {
    type Ok = Output;

    type Error = MapError;

    fn serialize_field<T: ?Sized>(
        &mut self,
        _key: &'static str,
        _value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        unreachable!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        unreachable!()
    }
}
impl<'a, Output> SerializeStructVariant
    for &'a mut UnreachableSerializer<Output>
{
    type Ok = Output;

    type Error = MapError;

    fn serialize_field<T: ?Sized>(
        &mut self,
        _key: &'static str,
        _value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        todo!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        todo!()
    }
}

struct MapSerializeStruct {
    output: BTreeMap<String, String>,
}

impl SerializeStruct for MapSerializeStruct {
    type Ok = BTreeMap<String, String>;

    type Error = MapError;

    fn serialize_field<T: ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        let mut serializer = StringSerializer;

        let value = value.serialize(&mut serializer)?;
        self.output.insert(key.to_string(), value);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.output)
    }
}

struct StringSerializer;
impl<'a> Serializer for &'a mut StringSerializer {
    type Ok = String;
    type Error = MapError;

    type SerializeSeq = &'a mut UnreachableSerializer<Self::Ok>;
    type SerializeTuple = &'a mut UnreachableSerializer<Self::Ok>;
    type SerializeTupleStruct = &'a mut UnreachableSerializer<Self::Ok>;
    type SerializeTupleVariant = &'a mut UnreachableSerializer<Self::Ok>;
    type SerializeMap = &'a mut UnreachableSerializer<Self::Ok>;
    type SerializeStruct = &'a mut UnreachableSerializer<Self::Ok>;
    type SerializeStructVariant = &'a mut UnreachableSerializer<Self::Ok>;

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        Ok(v.to_string())
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
        todo!()
    }

    fn serialize_tuple(
        self,
        _len: usize,
    ) -> Result<Self::SerializeTuple, Self::Error> {
        todo!()
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        todo!()
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        todo!()
    }

    fn serialize_map(
        self,
        _len: Option<usize>,
    ) -> Result<Self::SerializeMap, Self::Error> {
        todo!()
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        todo!()
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        todo!()
    }
}

#[cfg(test)]
mod test {
    use serde::Serialize;

    use crate::to_map::to_map;

    #[test]
    fn test_to_map() {
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
}
