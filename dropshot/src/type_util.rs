// Copyright 2023 Oxide Computer Company

//! Utility functions for working with JsonSchema types.

use std::collections::HashSet;

use indexmap::IndexMap;
use schemars::Schema;
use serde_json::Value;

/// Returns true iff the input schema is a boolean, floating-point number,
/// string or integer.
pub fn type_is_scalar(
    operation_id: &str,
    name: &str,
    schema: &Schema,
    dependencies: &IndexMap<String, Schema>,
) -> Result<(), String> {
    type_is_scalar_common(
        operation_id,
        name,
        schema,
        dependencies,
        |instance_type| {
            matches!(instance_type, "boolean" | "number" | "string" | "integer")
        },
    )
}

/// Returns true iff the input schema is a string.
pub fn type_is_string(
    operation_id: &str,
    name: &str,
    schema: &Schema,
    dependencies: &IndexMap<String, Schema>,
) -> Result<(), String> {
    type_is_scalar_common(
        operation_id,
        name,
        schema,
        dependencies,
        |instance_type| instance_type == "string",
    )
}

/// Helper function for scalar types.
fn type_is_scalar_common(
    operation_id: &str,
    name: &str,
    schema: &Schema,
    dependencies: &IndexMap<String, Schema>,
    type_check: fn(&str) -> bool,
) -> Result<(), String> {
    // Make sure we're examining a type and not a reference
    let schema = type_resolve(schema, dependencies);

    let Some(obj) = schema.as_object() else {
        return Err(format!(
            "for endpoint {} the parameter '{}' must have a scalar type",
            operation_id, name
        ));
    };

    let instance_type = obj.get("type").and_then(Value::as_str);
    let has_subschemas = obj.contains_key("allOf")
        || obj.contains_key("anyOf")
        || obj.contains_key("oneOf");
    let has_other_body = obj.contains_key("properties")
        || obj.contains_key("items")
        || obj.contains_key("$ref");

    // Types that have no subschemas, are not arrays/objects/references, and
    // whose type matches the limited set of scalar types.
    if let Some(ty) = instance_type {
        if !has_subschemas && !has_other_body && type_check(ty) {
            return Ok(());
        }
    }

    // Handle subschemas — only if there are no other body parts that would
    // indicate a primitive type or ref.
    if instance_type.is_none()
        && !has_other_body
        && !obj.contains_key("format")
        && !obj.contains_key("enum")
        && !obj.contains_key("const")
        && has_subschemas
        && type_is_scalar_subschemas(
            operation_id,
            name,
            obj,
            dependencies,
            type_check,
        )
    {
        return Ok(());
    }

    Err(format!(
        "for endpoint {} the parameter '{}' must have a scalar type",
        operation_id, name
    ))
}

/// Determine if a collection of subschemas are scalar (and meet the criteria of
/// the `type_check` parameter). For `allOf` and `anyOf` subschemas, we proceed
/// only if there is a lone subschema which we check recursively. For `oneOf`
/// subschemas, we check that each subschema is scalar.
fn type_is_scalar_subschemas(
    operation_id: &str,
    name: &str,
    obj: &serde_json::Map<String, Value>,
    dependencies: &IndexMap<String, Schema>,
    type_check: fn(&str) -> bool,
) -> bool {
    let all_of = obj.get("allOf").and_then(Value::as_array);
    let any_of = obj.get("anyOf").and_then(Value::as_array);
    let one_of = obj.get("oneOf").and_then(Value::as_array);
    let has_not = obj.contains_key("not");
    let has_if_then = obj.contains_key("if")
        || obj.contains_key("then")
        || obj.contains_key("else");

    if has_not || has_if_then {
        return false;
    }

    match (all_of, any_of, one_of) {
        (Some(subs), None, None) | (None, Some(subs), None) => {
            if subs.len() != 1 {
                return false;
            }
            let Ok(sub_schema) = <&Schema>::try_from(&subs[0]) else {
                return false;
            };
            type_is_scalar_common(
                operation_id,
                name,
                sub_schema,
                dependencies,
                type_check,
            )
            .is_ok()
        }
        (None, None, Some(subs)) => subs.iter().all(|schema_value| {
            let Ok(sub_schema) = <&Schema>::try_from(schema_value) else {
                return false;
            };
            type_is_scalar_common(
                operation_id,
                name,
                sub_schema,
                dependencies,
                type_check,
            )
            .is_ok()
        }),
        _ => false,
    }
}

pub fn type_is_string_enum(
    operation_id: &str,
    name: &str,
    schema: &Schema,
    dependencies: &IndexMap<String, Schema>,
) -> Result<(), String> {
    // Make sure we're examining a type and not a reference
    let schema = type_resolve(schema, dependencies);

    let Some(obj) = schema.as_object() else {
        return Err(format!(
            "the parameter '{}' must be an array of strings",
            name
        ));
    };

    if obj.get("type").and_then(Value::as_str) != Some("array") {
        return Err(format!(
            "the parameter '{}' must be an array of strings",
            name
        ));
    }

    match obj.get("items") {
        Some(Value::Array(_)) => {
            panic!("the parameter '{}' has an invalid array type", name)
        }
        Some(items_value) => {
            let Ok(item_schema) = <&Schema>::try_from(items_value) else {
                panic!("the parameter '{}' has an invalid array type", name);
            };
            type_is_string(operation_id, name, item_schema, dependencies)
                .map_err(|_| {
                    format!(
                        "the parameter '{}' must be an array of strings",
                        name
                    )
                })
        }
        None => {
            panic!("the parameter '{}' has an invalid array type", name)
        }
    }
}

fn type_resolve<'a>(
    mut schema: &'a Schema,
    dependencies: &'a IndexMap<String, Schema>,
) -> &'a Schema {
    let mut set = HashSet::new();
    while let Some(ref_schema) = bare_ref(schema) {
        if set.contains(&ref_schema) {
            eprintln!("{:#?}", schema);
            eprintln!(
                "consider #[serde(rename = \"...\")] or #[serde(transparent)]"
            );
            panic!("type reference cycle detected");
        }
        set.insert(ref_schema.to_owned());
        const PREFIX: &str = "#/components/schemas/";
        assert!(ref_schema.starts_with(PREFIX));
        schema = dependencies
            .get(&ref_schema[PREFIX.len()..])
            .unwrap_or_else(|| panic!("invalid reference {}", ref_schema));
    }
    schema
}

/// Returns the `$ref` string if the schema is a bare reference (i.e. an
/// object with exactly one key, `$ref`). Otherwise returns `None`.
fn bare_ref(schema: &Schema) -> Option<String> {
    let obj = schema.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    obj.get("$ref").and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;
    use schemars::{json_schema, JsonSchema, Schema};

    use crate::schema_util::schema2struct;

    use super::type_resolve;

    #[test]
    #[should_panic(expected = "type reference cycle detected")]
    fn test_reflexive_type() {
        let name = "#/components/schemas/Selfie";
        let schema: Schema = json_schema!({ "$ref": name });

        let mut dependencies = IndexMap::new();
        dependencies.insert("Selfie".to_string(), schema);
        let schema_ref = &dependencies[0];

        type_resolve(schema_ref, &dependencies);
    }

    #[test]
    #[should_panic(expected = "type reference cycle detected")]
    fn test_recursive_type() {
        let jack_schema: Schema = json_schema!({
            "$ref": "#/components/schemas/JohnJackson"
        });
        let john_schema: Schema = json_schema!({
            "$ref": "#/components/schemas/JackJohnson"
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

        let mut generator = schemars::generate::SchemaGenerator::new(
            schemars::generate::SchemaSettings::openapi3(),
        );
        let schema = generator.root_schema_for::<ThingHolder>();

        let struct_props = schema2struct(
            &ThingHolder::schema_name(),
            "test",
            &schema,
            &generator,
            true,
        );

        assert_eq!(struct_props.len(), 1);

        let only = struct_props.first().unwrap();
        assert_eq!(only.name, "thing");
        assert_eq!(only.description, Some("This is my thing".to_string()));
        assert!(only.required);
    }
}
