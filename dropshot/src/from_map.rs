// Copyright 2020 Oxide Computer Company

use paste::paste;
use serde::de::DeserializeSeed;
use serde::de::EnumAccess;
use serde::de::MapAccess;
use serde::de::SeqAccess;
use serde::de::VariantAccess;
use serde::de::Visitor;
use serde::Deserialize;
use serde::Deserializer;
use std::any::type_name;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fmt::Display;

/**
 * Deserialize a BTreeMap<String, MapValue> into a type, invoking
 * String::parse() for all values according to the required type. MapValue may
 * be either a single String or a sequence of Strings.
 */
pub(crate) fn from_map<'a, T, Z>(
    map: &'a BTreeMap<String, Z>,
) -> Result<T, String>
where
    T: Deserialize<'a>,
    Z: MapValue + Debug + Clone + 'static,
{
    let mut deserializer = MapDeserializer::from_map(map);
    T::deserialize(&mut deserializer).map_err(|e| e.0)
}

pub(crate) trait MapValue {
    fn as_value(&self) -> Result<&str, MapError>;
    fn as_seq(&self) -> Result<Box<dyn Iterator<Item = String>>, MapError>;
}

impl MapValue for String {
    fn as_value(&self) -> Result<&str, MapError> {
        Ok(self.as_str())
    }

    fn as_seq(&self) -> Result<Box<dyn Iterator<Item = String>>, MapError> {
        Err(MapError(
            "a string may not be used in place of a sequence of values"
                .to_string(),
        ))
    }
}

/**
 * Deserializer for BTreeMap<String, MapValue> that interprets the values. It has
 * two modes: about to iterate over the map or about to process a single value.
 */
#[derive(Debug)]
enum MapDeserializer<'de, Z: MapValue + Debug + Clone + 'static> {
    Map(&'de BTreeMap<String, Z>),
    Value(Z),
}

impl<'de, Z> MapDeserializer<'de, Z>
where
    Z: MapValue + Debug + Clone + 'static,
{
    fn from_map(input: &'de BTreeMap<String, Z>) -> Self {
        MapDeserializer::Map(input)
    }

    /**
     * Helper function to extract pattern match for Value. Fail if we're
     * expecting a Map or return the result of the provided function.
     */
    fn value<VV, F>(&self, deserialize: F) -> Result<VV, MapError>
    where
        F: FnOnce(&Z) -> Result<VV, MapError>,
    {
        match self {
            MapDeserializer::Value(ref raw_value) => deserialize(raw_value),
            MapDeserializer::Map(_) => Err(MapError(
                "must be applied to a flattened struct rather than a raw type"
                    .to_string(),
            )),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MapError(pub String);

impl Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl serde::de::Error for MapError {
    fn custom<T>(msg: T) -> Self
    where
        T: std::fmt::Display,
    {
        MapError(format!("{}", msg))
    }
}

impl std::error::Error for MapError {}

/**
 * Stub out Deserializer trait functions that aren't applicable.
 */
macro_rules! de_unimp {
    ($i:ident $(, $p:ident : $t:ty )*) => {
        fn $i<V>(self $(, $p: $t)*, _visitor: V) -> Result<V::Value, MapError>
        where
            V: Visitor<'de>,
        {
            unimplemented!(stringify!($i));
        }
    };
    ($i:ident $(, $p:ident : $t:ty )* ,) => {
        de_unimp!($i $(, $p: $t)*);
    };
}

/*
 * Generate handlers for primitive types using FromStr::parse() to deserialize
 * from the string form. Note that for integral types parse does not accept
 * prefixes such as "0x", but we could add this easily with a custom handler.
 */
macro_rules! de_value {
    ($i:ident) => {
        paste! {
            fn [<deserialize_ $i>]<V>(self, visitor: V)
                -> Result<V::Value, MapError>
            where
                V: Visitor<'de>,
            {
                self.value(|raw_value| match raw_value.as_value()?.parse::<$i>() {
                    Ok(value) => visitor.[<visit_ $i>](value),
                    Err(_) => Err(MapError(format!(
                        "unable to parse '{}' as {}",
                        raw_value.as_value()?,
                        type_name::<$i>()
                    ))),
                })
            }
        }
    };
}

impl<'de, 'a, Z> Deserializer<'de> for &'a mut MapDeserializer<'de, Z>
where
    Z: MapValue + Debug + Clone + 'static,
{
    type Error = MapError;

    // Simple values
    de_value!(bool);
    de_value!(i8);
    de_value!(i16);
    de_value!(i32);
    de_value!(i64);
    de_value!(u8);
    de_value!(u16);
    de_value!(u32);
    de_value!(u64);
    de_value!(f32);
    de_value!(f64);
    de_value!(char);

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.value(|raw_value| visitor.visit_str(raw_value.as_value()?))
    }
    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.value(|raw_value| visitor.visit_str(raw_value.as_value()?))
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        // None is a missing field, so this must be Some.
        visitor.visit_some(self)
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_map(visitor)
    }
    /*
     * This will only be called when deserializing a structure that contains a
     * flattened structure. See `deserialize_any` below for details.
     */
    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self {
            MapDeserializer::Map(map) => {
                let xx = map.clone();
                let x = Box::new(xx.into_iter());
                let m = MapMapAccess::<Z> {
                    iter: x,
                    value: None,
                };
                visitor.visit_map(m)
            }
            MapDeserializer::Value(_) => Err(MapError(
                "destination struct must be fully flattened".to_string(),
            )),
        }
    }
    fn deserialize_identifier<V>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.value(|raw_value| visitor.visit_str(raw_value.as_value()?))
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_enum(self)
    }

    fn deserialize_ignored_any<V>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.value(|raw_value| visitor.visit_str(raw_value.as_value()?))
    }

    /*
     * We really shouldn't have to implement this, and we can't actually do so
     * properly, but due to the way that serde currently handles flattened
     * structs this will be called for all members in flattened (i.e. non-
     * root) structs
     *
     * See serde-rs/serde#1183 for details. The macro for serde::Deserialize
     * can't know the members of flattened structs (those are in a different
     * scope) so serde forces all items to be deserialized, saves them in a
     * map, and then deserialized them into the flattened structures without
     * interpretation (as opposed to the interpretation of strings that we
     * do in *this* Deserializer for a similar map). This is generally true
     * for all non-self-describing formats.
     *
     * A better approach in serde might be to defer type assignment with a
     * new function analogous to deserialize_any, but with the option of
     * returning the raw data, frozen for future processing. The serde
     * internal `FlatMapDeserializer` would then need to be able to send
     * those "frozen" values back to the Deserializer to be processed with
     * type information.
     */
    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.value(|raw_value| visitor.visit_str(raw_value.as_value()?))
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    de_unimp!(deserialize_bytes);
    de_unimp!(deserialize_byte_buf);
    de_unimp!(deserialize_unit);
    de_unimp!(deserialize_unit_struct, _name: &'static str);
    de_unimp!(deserialize_tuple, _len: usize);
    de_unimp!(deserialize_tuple_struct, _name: &'static str, _len: usize);

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.value(|raw_value| {
            visitor.visit_seq(MapSeqAccess {
                iter: raw_value.as_seq()?,
            })
        })
    }
}

/*
 * Deserializer component for processing enums.
 */
impl<'de, 'a, Z> EnumAccess<'de> for &mut MapDeserializer<'de, Z>
where
    Z: MapValue + Debug + Clone + 'static,
{
    type Error = MapError;
    type Variant = Self;

    fn variant_seed<V>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), MapError>
    where
        V: DeserializeSeed<'de>,
    {
        seed.deserialize(&mut *self).map(|v| (v, self))
    }
}

/*
 * Deserializer component for processing enum variants.
 */
impl<'de, 'a, Z> VariantAccess<'de> for &mut MapDeserializer<'de, Z>
where
    Z: MapValue + Clone + Debug + 'static,
{
    type Error = MapError;

    fn unit_variant(self) -> Result<(), MapError> {
        Ok(())
    }

    fn newtype_variant_seed<T>(self, _seed: T) -> Result<T::Value, MapError>
    where
        T: DeserializeSeed<'de>,
    {
        unimplemented!("newtype_variant_seed");
    }

    fn tuple_variant<V>(
        self,
        _len: usize,
        _visitor: V,
    ) -> Result<V::Value, MapError>
    where
        V: Visitor<'de>,
    {
        unimplemented!("tuple_variant");
    }

    fn struct_variant<V>(
        self,
        _fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, MapError>
    where
        V: Visitor<'de>,
    {
        unimplemented!("struct_variant");
    }
}

/*
 * Deserializer component for iterating over the Map.
 */
struct MapMapAccess<Z> {
    /** Iterator through the Map */
    iter: Box<dyn Iterator<Item = (String, Z)>>,
    /** Pending value in a key-value pair */
    value: Option<Z>,
}

impl<'de, 'a, Z> MapAccess<'de> for MapMapAccess<Z>
where
    Z: MapValue + Debug + Clone + 'static,
{
    type Error = MapError;

    fn next_key_seed<K>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error>
    where
        K: DeserializeSeed<'de>,
    {
        match self.iter.next() {
            Some((key, value)) => {
                /* Save the value for later. */
                self.value.replace(value);
                /* Create a Deserializer for that single value. */
                let mut deserializer = MapDeserializer::Value(key);
                seed.deserialize(&mut deserializer).map(Some)
            }
            None => Ok(None),
        }
    }
    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        match self.value.take() {
            Some(value) => {
                let mut deserializer = MapDeserializer::Value(value);
                seed.deserialize(&mut deserializer)
            }
            /*
             * This means we were called without a corresponding call to
             * next_key_seed() which should not be possible.
             */
            None => unreachable!(),
        }
    }
}

struct MapSeqAccess<Z> {
    iter: Box<dyn Iterator<Item = Z>>,
}

impl<'de, 'a, Z> SeqAccess<'de> for MapSeqAccess<Z>
where
    Z: MapValue + Debug + Clone + 'static,
{
    type Error = MapError;

    fn next_element_seed<T>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        match self.iter.next() {
            Some(value) => {
                let mut deserializer = MapDeserializer::Value(value);
                seed.deserialize(&mut deserializer).map(Some)
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod test {
    use super::from_map;
    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[test]
    fn test_lone_literal() {
        let map = BTreeMap::new();
        match from_map::<String, String>(&map) {
            Err(s) => assert_eq!(
                s,
                "must be applied to a flattened struct rather than a raw type"
            ),
            Ok(_) => panic!("unexpected success"),
        }
    }

    #[test]
    fn test_deep() {
        #[derive(Deserialize, Debug)]
        struct A {
            b: B,
        }
        #[derive(Deserialize, Debug)]
        struct B {
            s: String,
        }
        let mut map = BTreeMap::new();
        map.insert("b".to_string(), "B".to_string());
        match from_map::<A, String>(&map) {
            Err(s) => {
                assert_eq!(s, "destination struct must be fully flattened")
            }
            Ok(_) => panic!("unexpected success"),
        }
    }
    #[test]
    fn test_missing_data1() {
        #[derive(Deserialize, Debug)]
        struct A {
            aaa: String,
            #[serde(flatten)]
            bbb: B,
        }
        #[derive(Deserialize, Debug)]
        struct B {
            sss: String,
        }
        let mut map = BTreeMap::new();
        map.insert("aaa".to_string(), "A".to_string());
        match from_map::<A, String>(&map) {
            Err(s) => assert_eq!(s, "missing field `sss`"),
            Ok(_) => panic!("unexpected success"),
        }
    }
    #[test]
    fn test_missing_data2() {
        #[derive(Deserialize, Debug)]
        struct A {
            aaa: String,
            #[serde(flatten)]
            bbb: B,
        }
        #[derive(Deserialize, Debug)]
        struct B {
            sss: String,
        }
        let mut map = BTreeMap::new();
        map.insert("sss".to_string(), "stringy".to_string());
        match from_map::<A, String>(&map) {
            Err(s) => assert_eq!(s, "missing field `aaa`"),
            Ok(_) => panic!("unexpected success"),
        }
    }
    #[test]
    fn test_types() {
        #[derive(Deserialize, Debug)]
        struct Newbie(String);

        #[derive(Deserialize, Debug)]
        struct A {
            astring: String,
            au32: u32,
            ai16: i16,
            abool: bool,
            aoption: Option<i8>,
            anew: Newbie,

            #[serde(flatten)]
            bbb: B,
        }
        #[derive(Deserialize, Debug)]
        struct B {
            bstring: String,
            boption: Option<String>,
        }
        let mut map = BTreeMap::new();
        map.insert("astring".to_string(), "A string".to_string());
        map.insert("au32".to_string(), "1000".to_string());
        map.insert("ai16".to_string(), "-1000".to_string());
        map.insert("abool".to_string(), "false".to_string());
        map.insert("aoption".to_string(), "8".to_string());
        map.insert("anew".to_string(), "New string".to_string());
        map.insert("bstring".to_string(), "B string".to_string());
        match from_map::<A, String>(&map) {
            Ok(a) => {
                assert_eq!(a.astring, "A string");
                assert_eq!(a.au32, 1000);
                assert_eq!(a.ai16, -1000);
                assert_eq!(a.abool, false);
                assert_eq!(a.aoption, Some(8));
                assert_eq!(a.anew.0, "New string");
                assert_eq!(a.bbb.bstring, "B string");
                assert_eq!(a.bbb.boption, None);
            }
            Err(s) => panic!("error: {}", s),
        }
    }
    #[test]
    fn wherefore_art_thou_a_valid_sequence_when_in_fact_you_are_a_lone_value() {
        #[derive(Deserialize, Debug)]
        struct A {
            b: Vec<String>,
        }
        let mut map = BTreeMap::new();
        map.insert("b".to_string(), "stringy".to_string());
        match from_map::<A, String>(&map) {
            Err(s) => assert_eq!(
                s,
                "a string may not be used in place of a sequence of values"
            ),
            Ok(_) => panic!("unexpected success"),
        }
    }
}
