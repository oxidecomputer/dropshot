// Copyright 2021 Oxide Computer Company

//! Utility functions for working with JsonSchema types.

use std::collections::HashSet;

use indexmap::IndexMap;
use schemars::schema::{
    InstanceType, Schema, SchemaObject, SingleOrVec, SubschemaValidation,
};

/// Returns true iff the input schema is a boolean, floating-point number,
/// string or integer.
pub fn type_is_scalar(
    name: &String,
    schema: &Schema,
    dependencies: &IndexMap<String, Schema>,
) -> Result<(), String> {
    type_is_scalar_common(name, schema, dependencies, |instance_type| {
        matches!(
            instance_type,
            InstanceType::Boolean
                | InstanceType::Number
                | InstanceType::String
                | InstanceType::Integer
        )
    })
}

/// Returns true iff the input schema is a string.
pub fn type_is_string(
    name: &String,
    schema: &Schema,
    dependencies: &IndexMap<String, Schema>,
) -> Result<(), String> {
    type_is_scalar_common(name, schema, dependencies, |instance_type| {
        matches!(instance_type, InstanceType::String)
    })
}

/// Helper function for scalar types.
fn type_is_scalar_common(
    name: &String,
    schema: &Schema,
    dependencies: &IndexMap<String, Schema>,
    type_check: fn(&InstanceType) -> bool,
) -> Result<(), String> {
    // Make sure we're examining a type and not a reference
    let schema = type_resolve(schema, dependencies);

    match schema {
        // Types that have no subschemas, are not arrays, are not objects, are
        // not references, and whose instance type matches the limited set of
        // scalar types.
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(instance_type)),
            subschemas: None,
            array: None,
            object: None,
            reference: None,
            ..
        }) if type_check(instance_type.as_ref()) => Ok(()),

        // Handle subschemas.
        Schema::Object(SchemaObject {
            instance_type: None,
            format: None,
            enum_values: None,
            const_value: None,
            subschemas: Some(subschemas),
            number: None,
            string: None,
            array: None,
            object: None,
            reference: None,
            ..
        }) if type_is_scalar_subschemas(
            name,
            subschemas,
            dependencies,
            type_check,
        ) =>
        {
            Ok(())
        }

        _ => Err(format!("the parameter '{}' must have a scalar type", name)),
    }
}

/// Determine if a collection of subschemas are scalar (and meet the criteria of
/// the `type_check` parameter). For `allOf` and `anyOf` subschemas, we proceed
/// only if there is a lone subschema which we check recursively. For `oneOf`
/// subschemas, we check that each subschema is scalar.
fn type_is_scalar_subschemas(
    name: &String,
    subschemas: &SubschemaValidation,
    dependencies: &IndexMap<String, Schema>,
    type_check: fn(&InstanceType) -> bool,
) -> bool {
    match subschemas {
        SubschemaValidation {
            all_of: Some(subs),
            any_of: None,
            one_of: None,
            not: None,
            if_schema: None,
            then_schema: None,
            else_schema: None,
        }
        | SubschemaValidation {
            all_of: None,
            any_of: Some(subs),
            one_of: None,
            not: None,
            if_schema: None,
            then_schema: None,
            else_schema: None,
        } if subs.len() == 1 => type_is_scalar_common(
            name,
            subs.first().unwrap(),
            dependencies,
            type_check,
        )
        .is_ok(),

        SubschemaValidation {
            all_of: None,
            any_of: None,
            one_of: Some(subs),
            not: None,
            if_schema: None,
            then_schema: None,
            else_schema: None,
        } => subs.iter().all(|schema| {
            type_is_scalar_common(name, schema, dependencies, type_check)
                .is_ok()
        }),

        _ => false,
    }
}

pub fn type_is_string_enum(
    name: &String,
    schema: &Schema,
    dependencies: &IndexMap<String, Schema>,
) -> Result<(), String> {
    // Make sure we're examining a type and not a reference
    let schema = type_resolve(schema, dependencies);

    match schema {
        Schema::Object(SchemaObject {
            metadata: _,
            instance_type:
                Some(schemars::schema::SingleOrVec::Single(instance_type)),
            format: None,
            enum_values: None,
            const_value: None,
            subschemas: None,
            number: None,
            string: None,
            array: Some(array_validation),
            object: None,
            reference: None,
            extensions: _,
        }) if instance_type.as_ref() == &InstanceType::Array => {
            match array_validation.as_ref() {
                schemars::schema::ArrayValidation {
                    items:
                        Some(schemars::schema::SingleOrVec::Single(item_schema)),
                    additional_items: None,
                    ..
                } => type_is_string(name, item_schema, dependencies).map_err(
                    |_| {
                        format!(
                            "the parameter '{}' must be an array of strings",
                            name
                        )
                    },
                ),
                _ => {
                    panic!("the parameter '{}' has an invalid array type", name)
                }
            }
        }
        _ => {
            Err(format!("the parameter '{}' must be an array of strings", name))
        }
    }
}

fn type_resolve<'a>(
    mut schema: &'a Schema,
    dependencies: &'a IndexMap<String, Schema>,
) -> &'a Schema {
    let mut set = HashSet::new();
    while let Schema::Object(SchemaObject {
        metadata: _,
        instance_type: None,
        format: None,
        enum_values: None,
        const_value: None,
        subschemas: None,
        number: None,
        string: None,
        array: None,
        object: None,
        reference: Some(ref_schema),
        extensions: _,
    }) = schema
    {
        if set.contains(ref_schema) {
            eprintln!("{:#?}", schema);
            eprintln!(
                "consider #[serde(rename = \"...\")] or #[serde(transparent)]"
            );
            panic!("type reference cycle detected");
        }
        set.insert(ref_schema);
        const PREFIX: &str = "#/components/schemas/";
        assert!(ref_schema.starts_with(PREFIX));
        schema = dependencies
            .get(&ref_schema[PREFIX.len()..])
            .unwrap_or_else(|| panic!("invalid reference {}", ref_schema));
    }
    schema
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;
    use schemars::{
        schema::{Schema, SchemaObject},
        JsonSchema,
    };

    use crate::schema_util::schema2struct;

    use super::type_resolve;

    #[test]
    #[should_panic(expected = "type reference cycle detected")]
    fn test_reflexive_type() {
        let name = "#/components/schemas/Selfie".to_string();
        let schema = Schema::Object(SchemaObject {
            reference: Some(name),
            ..Default::default()
        });

        let mut dependencies = IndexMap::new();
        dependencies.insert("Selfie".to_string(), schema);
        let schema_ref = &dependencies[0];

        type_resolve(schema_ref, &dependencies);
    }

    #[test]
    #[should_panic(expected = "type reference cycle detected")]
    fn test_recursive_type() {
        let jack_schema = Schema::Object(SchemaObject {
            reference: Some("#/components/schemas/JohnJackson".to_string()),
            ..Default::default()
        });
        let john_schema = Schema::Object(SchemaObject {
            reference: Some("#/components/schemas/JackJohnson".to_string()),
            ..Default::default()
        });

        let mut dependencies = IndexMap::new();
        dependencies.insert("JackJohnson".to_string(), jack_schema);
        dependencies.insert("JohnJackson".to_string(), john_schema);
        let schema_ref = &dependencies[0];

        type_resolve(schema_ref, &dependencies);
    }

    #[test]
    fn test_commented_ref() {
        #![allow(dead_code)]

        #[derive(JsonSchema)]
        enum Things {
            Salami,
            Tamale,
            Lolly,
        }

        #[derive(JsonSchema)]
        struct ThingHolder {
            /// This is my thing
            thing: Things,
        }

        let mut generator = schemars::gen::SchemaGenerator::new(
            schemars::gen::SchemaSettings::openapi3(),
        );
        let schema: schemars::schema::Schema =
            generator.root_schema_for::<ThingHolder>().schema.into();

        let struct_props = schema2struct(&schema, &generator, true);

        assert_eq!(struct_props.len(), 1);

        let only = struct_props.first().unwrap();
        assert_eq!(only.name, "thing");
        assert_eq!(only.description, Some("This is my thing".to_string()));
        assert!(only.required);
    }
}
