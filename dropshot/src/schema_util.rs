// Copyright 2025 Oxide Computer Company

//! schemars helper functions

use schemars::JsonSchema;
use serde_json::{Map, Value};

use std::fmt;

#[derive(Debug)]
pub(crate) struct StructMember {
    pub name: String,
    pub description: Option<String>,
    pub schema: schemars::Schema,
    pub required: bool,
}

/// This helper function produces a list of the structure members for the
/// given schema. For each it returns:
///   (name: &String, schema: &Schema, required: bool)
///
/// If the input schema is not a flat structure the result will be a runtime
/// failure reflective of a programming error (likely an invalid type specified
/// in a handler function).
pub(crate) fn schema2struct(
    schema_name: &str,
    kind: &str,
    schema: &schemars::Schema,
    generator: &schemars::SchemaGenerator,
    required: bool,
) -> Vec<StructMember> {
    match schema2struct_impl(schema, generator, required) {
        Ok(results) => results,
        Err(error) => {
            panic!(
                "while generating schema for {} ({}): {}",
                schema_name, kind, error
            );
        }
    }
}

/// Look up the value of the given `$ref`-only schema in the generator's
/// definitions. Returns `None` if the schema is not a bare `$ref`.
fn deref_schema<'a>(
    generator: &'a schemars::SchemaGenerator,
    schema: &'a schemars::Schema,
) -> Option<&'a schemars::Schema> {
    let obj = schema.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    let reference = obj.get("$ref")?.as_str()?;
    let name =
        reference.strip_prefix("#/components/schemas/").unwrap_or(reference);
    let value = generator.definitions().get(name)?;
    <&schemars::Schema>::try_from(value).ok()
}

/// Categorize an object schema body by its set of "body" keys (properties,
/// required, subschemas, enum, $ref, type, etc.) so we can match shapes
/// similar to how the old typed-struct pattern matching worked.
struct ObjectView<'a> {
    map: &'a Map<String, Value>,
}

impl<'a> ObjectView<'a> {
    fn new(map: &'a Map<String, Value>) -> Self {
        Self { map }
    }

    fn get(&self, key: &str) -> Option<&'a Value> {
        self.map.get(key)
    }

    fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    fn get_type(&self) -> Option<&'a str> {
        self.get("type")?.as_str()
    }

    fn get_array_items(&self, key: &str) -> Option<&'a Vec<Value>> {
        self.get(key)?.as_array()
    }
}

/// This function is invoked recursively on subschemas.
fn schema2struct_impl(
    schema: &schemars::Schema,
    generator: &schemars::SchemaGenerator,
    required: bool,
) -> Result<Vec<StructMember>, Box<Schema2StructError>> {
    let Some(obj) = schema.as_object() else {
        return Err(Box::new(Schema2StructError::InvalidType(schema.clone())));
    };
    let view = ObjectView::new(obj);

    // Bare `$ref` schema — resolve and recurse.
    if view.has("$ref") && obj.len() == 1 {
        let target =
            deref_schema(generator, schema).expect("invalid reference");
        return schema2struct_impl(target, generator, required);
    }

    // An object (possibly with subschemas) — walk properties and subschemas.
    let is_object = view.get_type() == Some("object") || view.has("properties");
    let has_subschemas =
        view.has("allOf") || view.has("anyOf") || view.has("oneOf");

    // Enum schemas (unit enums without doc comments) are reported as errors
    // with a helpful hint.
    if view.has("enum") && !is_object && !has_subschemas {
        return Err(Box::new(Schema2StructError::InvalidEnum(schema.clone())));
    }

    // Complex enums (subschemas without an object type) — they look like
    // `{ "oneOf": [...] }` with no other body — are reported similarly.
    if has_subschemas
        && !is_object
        && !view.has("enum")
        && !view.has("const")
        && !view.has("$ref")
        && !has_number_body(&view)
        && !view.has("items")
    {
        // Distinguish: a single-child `allOf` or `anyOf` wrapping an object
        // (often used to carry a description) should be unwrapped. Otherwise
        // treat as an enum error.
        let wrapper_kind = view
            .get_array_items("allOf")
            .filter(|arr| arr.len() == 1 && view.map.len() <= 2)
            .or_else(|| {
                view.get_array_items("anyOf")
                    .filter(|_| view.map.len() <= 2)
            });

        if wrapper_kind.is_none() {
            return Err(Box::new(Schema2StructError::InvalidEnum(
                schema.clone(),
            )));
        }
    }

    if !is_object && !has_subschemas {
        return Err(Box::new(Schema2StructError::InvalidType(schema.clone())));
    }

    let mut results = Vec::new();

    // Walk `properties`.
    if let Some(Value::Object(props)) = view.get("properties") {
        let required_set = view
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let required_names: std::collections::HashSet<&str> =
            required_set.iter().filter_map(Value::as_str).collect();

        for (name, prop_value) in props {
            let prop_schema: &schemars::Schema = prop_value
                .try_into()
                .expect("property schema must be an object or bool");
            let (description, schema) = schema_extract_description(prop_schema);
            results.push(StructMember {
                name: name.clone(),
                description,
                schema,
                required: required && required_names.contains(name.as_str()),
            });
        }
    }

    // Walk subschemas. We might see these for flattened enums/structs with
    // doc comments.
    let process_subschemas = |arr: &Vec<Value>,
                              sub_required: bool,
                              out: &mut Vec<StructMember>|
     -> Result<(), Box<Schema2StructError>> {
        for sub in arr {
            let sub_schema: &schemars::Schema =
                sub.try_into().expect("subschema must be an object or bool");
            out.extend(schema2struct_impl(
                sub_schema,
                generator,
                sub_required,
            )?);
        }
        Ok(())
    };

    let any_of = view.get_array_items("anyOf");
    let all_of = view.get_array_items("allOf");
    let one_of = view.get_array_items("oneOf");
    let has_not = view.has("not");
    let has_if = view.has("if") || view.has("then") || view.has("else");

    match (all_of, any_of, one_of, has_not, has_if) {
        (None, None, None, false, false) => {}
        // any_of in the case of an enum
        (None, Some(schemas), None, false, false) => {
            process_subschemas(schemas, false, &mut results)?;
        }
        // all_of with a single element — doc comment wrapper
        (Some(schemas), None, None, false, false) if schemas.len() == 1 => {
            process_subschemas(schemas, required, &mut results)?;
        }
        _ => {
            return Err(Box::new(Schema2StructError::InvalidSubschema(
                schema.clone(),
            )));
        }
    }

    Ok(results)
}

fn has_number_body(view: &ObjectView<'_>) -> bool {
    view.has("minimum")
        || view.has("maximum")
        || view.has("exclusiveMinimum")
        || view.has("exclusiveMaximum")
        || view.has("multipleOf")
}

#[derive(Debug)]
pub(crate) enum Schema2StructError {
    InvalidEnum(schemars::Schema),
    InvalidType(schemars::Schema),
    InvalidSubschema(schemars::Schema),
}

impl fmt::Display for Schema2StructError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Schema2StructError::InvalidEnum(schema) => {
                write!(
                    f,
                    "invalid type: {}\n\
                    (hint: this appears to be an enum, \
                     which needs to be wrapped in a struct)",
                    serde_json::to_string_pretty(schema).unwrap(),
                )
            }
            Schema2StructError::InvalidType(schema) => {
                write!(
                    f,
                    "invalid type: {}",
                    serde_json::to_string_pretty(schema).unwrap(),
                )
            }
            Schema2StructError::InvalidSubschema(schema) => {
                write!(
                    f,
                    "invalid subschema: {}",
                    serde_json::to_string_pretty(schema).unwrap(),
                )
            }
        }
    }
}

impl std::error::Error for Schema2StructError {}

pub(crate) fn make_subschema_for<T: JsonSchema>(
    gen: &mut schemars::SchemaGenerator,
) -> schemars::Schema {
    gen.subschema_for::<T>()
}

/// Returns the full object map of the schema (for looking up extension keys
/// or any other fields). Returns `None` for bool schemas.
pub(crate) fn schema_extensions(
    schema: &schemars::Schema,
) -> Option<&Map<String, Value>> {
    schema.as_object()
}

pub(crate) fn schema_extract_description(
    schema: &schemars::Schema,
) -> (Option<String>, schemars::Schema) {
    // Because the OpenAPI v3.0.x Schema cannot include a description with
    // a reference, we may see a schema with a description and an `all_of`
    // with a single subschema. In this case, we flatten the trivial subschema.
    if let Some(obj) = schema.as_object() {
        let view = ObjectView::new(obj);

        // Detect: { description, allOf: [single] } with no other body.
        if let Some(Value::Array(all_of)) = view.get("allOf") {
            if all_of.len() == 1
                && view.get("description").is_some()
                && all_of_single_wrapper_only(obj)
            {
                let description = view
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Ok(sub) = <&schemars::Schema>::try_from(&all_of[0]) {
                    return (description, sub.clone());
                }
            }
        }

        // Otherwise, pop the description and return the remainder.
        if let Some(description) =
            view.get("description").and_then(Value::as_str).map(str::to_owned)
        {
            let mut new_map = obj.clone();
            new_map.remove("description");
            return (Some(description), new_map.into());
        }
    }
    (None, schema.clone())
}

/// Returns true iff the object has a `description` and an `allOf` and no other
/// "body" keys (standard schema keywords). Allow `title` alongside.
fn all_of_single_wrapper_only(obj: &Map<String, Value>) -> bool {
    for key in obj.keys() {
        match key.as_str() {
            "description" | "allOf" | "title" => {}
            _ => return false,
        }
    }
    true
}

/// Convert from JSON Schema into OpenAPI.
// TODO Initially this seemed like it was going to be a win, but the versions
// of JSON Schema that the schemars and openapiv3 crates adhere to are just
// different enough to make the conversion a real pain in the neck. A better
// approach might be a derive(OpenAPI)-like thing, or even a generic
// derive(schema) that we could then marshall into OpenAPI.
// The schemars crate also seems a bit inflexible when it comes to how the
// schema is generated wrt references vs. inline types.
pub(crate) fn j2oas_schema(
    name: Option<&str>,
    schema: &schemars::Schema,
) -> openapiv3::ReferenceOr<openapiv3::Schema> {
    // The permissive, "match anything" schema. We'll typically see this
    // when consumers use a type such as serde_json::Value.
    if schema.as_bool() == Some(true) {
        return openapiv3::ReferenceOr::Item(openapiv3::Schema {
            schema_data: Default::default(),
            schema_kind: openapiv3::SchemaKind::Any(Default::default()),
        });
    }
    // The unsatisfiable, "match nothing" schema. We represent this as
    // the `not` of the permissive schema.
    if schema.as_bool() == Some(false) {
        return openapiv3::ReferenceOr::Item(openapiv3::Schema {
            schema_data: Default::default(),
            schema_kind: openapiv3::SchemaKind::Not {
                not: Box::new(openapiv3::ReferenceOr::Item(
                    openapiv3::Schema {
                        schema_data: Default::default(),
                        schema_kind: openapiv3::SchemaKind::Any(
                            Default::default(),
                        ),
                    },
                )),
            },
        });
    }
    j2oas_schema_object(name, schema)
}

fn j2oas_schema_vec(
    schemas: Option<&Vec<Value>>,
) -> Vec<openapiv3::ReferenceOr<openapiv3::Schema>> {
    schemas
        .map(|v| {
            v.iter()
                .map(|value| {
                    let schema: &schemars::Schema = value
                        .try_into()
                        .expect("subschema must be an object or bool");
                    j2oas_schema(None, schema)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn j2oas_schema_object(
    name: Option<&str>,
    schema: &schemars::Schema,
) -> openapiv3::ReferenceOr<openapiv3::Schema> {
    let obj = schema
        .as_object()
        .expect("j2oas_schema_object called with a non-object schema");
    let view = ObjectView::new(obj);

    if let Some(reference) = view.get("$ref").and_then(Value::as_str) {
        return openapiv3::ReferenceOr::Reference {
            reference: reference.to_owned(),
        };
    }

    let kind = j2oas_schema_object_kind(&view);

    let mut data = openapiv3::SchemaData::default();

    if matches!(view.get("nullable"), Some(Value::Bool(true))) {
        data.nullable = true;
    }

    data.title = view.get("title").and_then(Value::as_str).map(str::to_owned);
    data.description =
        view.get("description").and_then(Value::as_str).map(str::to_owned);
    data.default = view.get("default").cloned();
    data.deprecated =
        view.get("deprecated").and_then(Value::as_bool).unwrap_or(false);
    data.read_only =
        view.get("readOnly").and_then(Value::as_bool).unwrap_or(false);
    data.write_only =
        view.get("writeOnly").and_then(Value::as_bool).unwrap_or(false);

    // Preserve x-* extensions.
    data.extensions = obj
        .iter()
        .filter(|(key, _)| key.starts_with("x-"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();

    if let Some(name) = name {
        data.title = Some(name.to_owned());
    }
    if let Some(example) = view.get("example") {
        data.example = Some(example.clone());
    }

    openapiv3::ReferenceOr::Item(openapiv3::Schema {
        schema_data: data,
        schema_kind: kind,
    })
}

fn j2oas_schema_object_kind(view: &ObjectView<'_>) -> openapiv3::SchemaKind {
    // If the JSON schema is attempting to express an unsatisfiable schema
    // using an empty enumerated values array, that presents a problem
    // translating to the openapiv3 crate's representation. While we might
    // like to simply use the schema `false` that is *also* challenging to
    // represent via the openapiv3 crate *and* it precludes us from preserving
    // extensions, if they happen to be present. Instead we'll represent this
    // construction using `{ not: {} }` i.e. the opposite of the permissive
    // schema.
    if let Some(Value::Array(enum_values)) = view.get("enum") {
        if enum_values.is_empty() {
            return openapiv3::SchemaKind::Not {
                not: Box::new(openapiv3::ReferenceOr::Item(
                    openapiv3::Schema {
                        schema_data: Default::default(),
                        schema_kind: openapiv3::SchemaKind::Any(
                            Default::default(),
                        ),
                    },
                )),
            };
        }
    }

    let ty = match view.get("type") {
        Some(Value::String(s)) => Some(s.as_str()),
        Some(Value::Array(_)) => panic!(
            "a type array is unsupported by openapiv3:\n{}",
            serde_json::to_string_pretty(view.map)
                .unwrap_or_else(|_| "<can't serialize>".to_string())
        ),
        _ => None,
    };

    let has_subschemas = view.has("allOf")
        || view.has("anyOf")
        || view.has("oneOf")
        || view.has("not");

    match (ty, has_subschemas) {
        (Some("null"), false) => openapiv3::SchemaKind::Type(
            openapiv3::Type::String(openapiv3::StringType {
                enumeration: vec![None],
                ..Default::default()
            }),
        ),
        (Some("boolean"), false) => {
            let enumeration = view
                .get("enum")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .map(|vv| match vv {
                            Value::Null => None,
                            Value::Bool(b) => Some(*b),
                            _ => {
                                panic!("unexpected enumeration value {:?}", vv)
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            openapiv3::SchemaKind::Type(openapiv3::Type::Boolean(
                openapiv3::BooleanType { enumeration },
            ))
        }
        (Some("object"), false) => openapiv3::SchemaKind::Type(
            openapiv3::Type::Object(j2oas_object(view)),
        ),
        (Some("array"), false) => openapiv3::SchemaKind::Type(
            openapiv3::Type::Array(j2oas_array(view)),
        ),
        (Some("number"), false) => openapiv3::SchemaKind::Type(
            openapiv3::Type::Number(j2oas_number(view)),
        ),
        (Some("string"), false) => openapiv3::SchemaKind::Type(
            openapiv3::Type::String(j2oas_string(view)),
        ),
        (Some("integer"), false) => j2oas_integer(view),
        (None, true) => j2oas_subschemas(view),
        (None, false) => {
            openapiv3::SchemaKind::Any(openapiv3::AnySchema::default())
        }
        (Some(_), true) => j2oas_any(ty, view),
        (Some(other), false) => {
            panic!("unsupported JSON-schema type: {:?}", other)
        }
    }
}

fn j2oas_any(ty: Option<&str>, view: &ObjectView<'_>) -> openapiv3::SchemaKind {
    let typ = ty.map(str::to_owned);

    let enum_values = view.get("enum").and_then(Value::as_array);
    let const_value = view.get("const");
    let enumeration = match (enum_values, const_value) {
        (None, None) => Default::default(),
        (Some(values), Some(one_value)) if !values.contains(one_value) => {
            // It would be weird to have enum and const... and it would be
            // really really weird for them to conflict.
            panic!("both enum and const are present and also conflict")
        }
        (_, Some(value)) => vec![value.clone()],
        (Some(values), None) => values.clone(),
    };

    let mut any = openapiv3::AnySchema {
        typ,
        enumeration,
        format: view.get("format").and_then(Value::as_str).map(str::to_owned),
        ..Default::default()
    };

    if view.has("properties")
        || view.get_type() == Some("object")
        || view.has("additionalProperties")
        || view.has("required")
        || view.has("minProperties")
        || view.has("maxProperties")
    {
        let openapiv3::ObjectType {
            properties,
            required,
            additional_properties,
            min_properties,
            max_properties,
        } = j2oas_object(view);
        any.properties = properties;
        any.required = required;
        any.additional_properties = additional_properties;
        any.min_properties = min_properties;
        any.max_properties = max_properties;
    }

    if view.has("items")
        || view.get_type() == Some("array")
        || view.has("minItems")
        || view.has("maxItems")
        || view.has("uniqueItems")
    {
        let openapiv3::ArrayType {
            items,
            min_items,
            max_items,
            unique_items: _,
        } = j2oas_array(view);
        any.items = items;
        any.min_items = min_items;
        any.max_items = max_items;
        any.unique_items = view.get("uniqueItems").and_then(Value::as_bool);
    }

    if view.has("pattern")
        || view.has("minLength")
        || view.has("maxLength")
        || view.get_type() == Some("string")
    {
        let openapiv3::StringType {
            format: _,
            pattern,
            enumeration: _,
            min_length,
            max_length,
        } = j2oas_string(view);
        any.pattern = pattern;
        any.min_length = min_length;
        any.max_length = max_length;
    }

    if has_number_body(view)
        || view.get_type() == Some("number")
        || view.get_type() == Some("integer")
    {
        let openapiv3::NumberType {
            format: _,
            multiple_of,
            exclusive_minimum,
            exclusive_maximum,
            minimum,
            maximum,
            enumeration: _,
        } = j2oas_number(view);
        any.multiple_of = multiple_of;
        any.exclusive_minimum = exclusive_minimum.then_some(true);
        any.exclusive_maximum = exclusive_maximum.then_some(true);
        any.minimum = minimum;
        any.maximum = maximum;
    }

    if view.has("allOf")
        || view.has("anyOf")
        || view.has("oneOf")
        || view.has("not")
    {
        any.all_of = j2oas_schema_vec(view.get_array_items("allOf"));
        any.any_of = j2oas_schema_vec(view.get_array_items("anyOf"));
        any.one_of = j2oas_schema_vec(view.get_array_items("oneOf"));
        any.not = view.get("not").map(|not_value| {
            let not_schema: &schemars::Schema = not_value
                .try_into()
                .expect("not-schema must be an object or bool");
            Box::new(j2oas_schema(None, not_schema))
        });
    }

    openapiv3::SchemaKind::Any(any)
}

fn j2oas_subschemas(view: &ObjectView<'_>) -> openapiv3::SchemaKind {
    let all_of = view.get_array_items("allOf");
    let any_of = view.get_array_items("anyOf");
    let one_of = view.get_array_items("oneOf");
    let not_value = view.get("not");

    match (all_of, any_of, one_of, not_value) {
        (Some(_), None, None, None) => {
            openapiv3::SchemaKind::AllOf { all_of: j2oas_schema_vec(all_of) }
        }
        (None, Some(_), None, None) => {
            openapiv3::SchemaKind::AnyOf { any_of: j2oas_schema_vec(any_of) }
        }
        (None, None, Some(_), None) => {
            openapiv3::SchemaKind::OneOf { one_of: j2oas_schema_vec(one_of) }
        }
        (None, None, None, Some(not_value)) => {
            let not_schema: &schemars::Schema = not_value
                .try_into()
                .expect("not-schema must be an object or bool");
            openapiv3::SchemaKind::Not {
                not: Box::new(j2oas_schema(None, not_schema)),
            }
        }
        _ => panic!("invalid subschema {:#?}", view.map),
    }
}

fn j2oas_integer(view: &ObjectView<'_>) -> openapiv3::SchemaKind {
    let format = match view.get("format").and_then(Value::as_str) {
        None => openapiv3::VariantOrUnknownOrEmpty::Empty,
        Some("int32") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::IntegerFormat::Int32,
        ),
        Some("int64") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::IntegerFormat::Int64,
        ),
        Some(other) => {
            openapiv3::VariantOrUnknownOrEmpty::Unknown(other.to_string())
        }
    };

    let multiple_of =
        view.get("multipleOf").and_then(Value::as_f64).map(|f| f as i64);
    let (minimum, exclusive_minimum) =
        number_bound_int(view.get("minimum"), view.get("exclusiveMinimum"));
    let (maximum, exclusive_maximum) =
        number_bound_int(view.get("maximum"), view.get("exclusiveMaximum"));

    let enumeration = view
        .get("enum")
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .map(|vv| match vv {
                    Value::Null => None,
                    Value::Number(value) => Some(value.as_i64().unwrap()),
                    _ => panic!("unexpected enumeration value {:?}", vv),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    openapiv3::SchemaKind::Type(openapiv3::Type::Integer(
        openapiv3::IntegerType {
            format,
            multiple_of,
            exclusive_minimum,
            exclusive_maximum,
            minimum,
            maximum,
            enumeration,
        },
    ))
}

fn number_bound_int(
    inclusive: Option<&Value>,
    exclusive: Option<&Value>,
) -> (Option<i64>, bool) {
    match (inclusive.and_then(Value::as_f64), exclusive.and_then(Value::as_f64))
    {
        (None, None) => (None, false),
        (Some(v), None) => (Some(v as i64), false),
        (None, Some(v)) => (Some(v as i64), true),
        _ => panic!("invalid"),
    }
}

fn number_bound_f64(
    inclusive: Option<&Value>,
    exclusive: Option<&Value>,
) -> (Option<f64>, bool) {
    match (inclusive.and_then(Value::as_f64), exclusive.and_then(Value::as_f64))
    {
        (None, None) => (None, false),
        (s @ Some(_), None) => (s, false),
        (None, s @ Some(_)) => (s, true),
        _ => panic!("invalid"),
    }
}

fn j2oas_number(view: &ObjectView<'_>) -> openapiv3::NumberType {
    let format = match view.get("format").and_then(Value::as_str) {
        None => openapiv3::VariantOrUnknownOrEmpty::Empty,
        Some("float") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::NumberFormat::Float,
        ),
        Some("double") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::NumberFormat::Double,
        ),
        Some(other) => {
            openapiv3::VariantOrUnknownOrEmpty::Unknown(other.to_string())
        }
    };

    let multiple_of = view.get("multipleOf").and_then(Value::as_f64);
    let (minimum, exclusive_minimum) =
        number_bound_f64(view.get("minimum"), view.get("exclusiveMinimum"));
    let (maximum, exclusive_maximum) =
        number_bound_f64(view.get("maximum"), view.get("exclusiveMaximum"));

    let enumeration = view
        .get("enum")
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .map(|vv| match vv {
                    Value::Null => None,
                    Value::Number(value) => Some(value.as_f64().unwrap()),
                    _ => panic!("unexpected enumeration value {:?}", vv),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    openapiv3::NumberType {
        format,
        multiple_of,
        exclusive_minimum,
        exclusive_maximum,
        minimum,
        maximum,
        enumeration,
    }
}

fn j2oas_string(view: &ObjectView<'_>) -> openapiv3::StringType {
    let format = match view.get("format").and_then(Value::as_str) {
        None => openapiv3::VariantOrUnknownOrEmpty::Empty,
        Some("date") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::StringFormat::Date,
        ),
        Some("date-time") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::StringFormat::DateTime,
        ),
        Some("password") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::StringFormat::Password,
        ),
        Some("byte") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::StringFormat::Byte,
        ),
        Some("binary") => openapiv3::VariantOrUnknownOrEmpty::Item(
            openapiv3::StringFormat::Binary,
        ),
        Some(other) => {
            openapiv3::VariantOrUnknownOrEmpty::Unknown(other.to_string())
        }
    };

    let max_length =
        view.get("maxLength").and_then(Value::as_u64).map(|n| n as usize);
    let min_length =
        view.get("minLength").and_then(Value::as_u64).map(|n| n as usize);
    let pattern =
        view.get("pattern").and_then(Value::as_str).map(str::to_owned);

    let enumeration = view
        .get("enum")
        .and_then(Value::as_array)
        .map(|v| {
            assert!(!v.is_empty());
            v.iter()
                .map(|vv| match vv {
                    Value::Null => None,
                    Value::String(s) => Some(s.clone()),
                    _ => panic!("unexpected enumeration value {:?}", vv),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    openapiv3::StringType {
        format,
        pattern,
        enumeration,
        min_length,
        max_length,
    }
}

fn j2oas_array(view: &ObjectView<'_>) -> openapiv3::ArrayType {
    let items = match view.get("items") {
        Some(Value::Array(_)) => {
            panic!("OpenAPI v3.0.x cannot support tuple-like arrays")
        }
        Some(item_value) => {
            let item_schema: &schemars::Schema = item_value
                .try_into()
                .expect("array items schema must be an object or bool");
            Some(box_reference_or(j2oas_schema(None, item_schema)))
        }
        None => None,
    };

    openapiv3::ArrayType {
        items,
        min_items: view
            .get("minItems")
            .and_then(Value::as_u64)
            .map(|n| n as usize),
        max_items: view
            .get("maxItems")
            .and_then(Value::as_u64)
            .map(|n| n as usize),
        unique_items: view
            .get("uniqueItems")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn box_reference_or<T>(
    r: openapiv3::ReferenceOr<T>,
) -> openapiv3::ReferenceOr<Box<T>> {
    match r {
        openapiv3::ReferenceOr::Item(schema) => {
            openapiv3::ReferenceOr::boxed_item(schema)
        }
        openapiv3::ReferenceOr::Reference { reference } => {
            openapiv3::ReferenceOr::Reference { reference }
        }
    }
}

fn j2oas_object(view: &ObjectView<'_>) -> openapiv3::ObjectType {
    let properties = view
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| {
            props
                .iter()
                .map(|(prop, prop_value)| {
                    let prop_schema: &schemars::Schema = prop_value
                        .try_into()
                        .expect("property schema must be an object or bool");
                    (
                        prop.clone(),
                        box_reference_or(j2oas_schema(None, prop_schema)),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    let required = view
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter().filter_map(Value::as_str).map(str::to_owned).collect()
        })
        .unwrap_or_default();

    let additional_properties =
        view.get("additionalProperties").map(|ap| match ap {
            Value::Bool(b) => openapiv3::AdditionalProperties::Any(*b),
            Value::Object(_) => {
                let inner: &schemars::Schema = ap.try_into().expect(
                    "additionalProperties schema must be an object or bool",
                );
                openapiv3::AdditionalProperties::Schema(Box::new(j2oas_schema(
                    None, inner,
                )))
            }
            _ => panic!(
                "additionalProperties must be an object or bool: {:?}",
                ap
            ),
        });

    openapiv3::ObjectType {
        properties,
        required,
        additional_properties,
        min_properties: view
            .get("minProperties")
            .and_then(Value::as_u64)
            .map(|n| n as usize),
        max_properties: view
            .get("maxProperties")
            .and_then(Value::as_u64)
            .map(|n| n as usize),
    }
}

#[cfg(test)]
mod test {
    use crate::schema_util::schema2struct_impl;

    use super::j2oas_schema;
    use super::j2oas_schema_object;
    use schemars::JsonSchema;

    #[test]
    fn test_empty_struct() {
        #[derive(JsonSchema)]
        struct Empty {}

        let settings = schemars::generate::SchemaSettings::openapi3();
        let mut generator = schemars::generate::SchemaGenerator::new(settings);

        let schema = Empty::json_schema(&mut generator);
        let _ = j2oas_schema(None, &schema);
    }

    #[test]
    fn test_garbage_barge_structure_conversion() {
        #[allow(dead_code)]
        #[derive(JsonSchema)]
        struct SuperGarbage {
            string: String,
            strings: Vec<String>,
            more_strings: [String; 3],
            substruct: Substruct,
            more: Option<Substruct>,
            union: Union,
            map: std::collections::BTreeMap<String, String>,
        }

        #[allow(dead_code)]
        #[derive(JsonSchema)]
        struct Substruct {
            ii32: i32,
            uu64: u64,
            ff: f32,
            dd: f64,
            b: bool,
        }

        #[allow(dead_code)]
        #[derive(JsonSchema)]
        enum Union {
            A { a: u32 },
            B { b: f32 },
        }

        let settings = schemars::generate::SchemaSettings::openapi3();
        let mut generator = schemars::generate::SchemaGenerator::new(settings);

        let schema = SuperGarbage::json_schema(&mut generator);
        let _ = j2oas_schema(None, &schema);
        for (key, value) in generator.definitions().iter() {
            let schema: &schemars::Schema = value.try_into().unwrap();
            let _ = j2oas_schema(Some(key), schema);
        }
    }

    #[test]
    fn test_additional_properties() {
        #[allow(dead_code)]
        #[derive(JsonSchema)]
        enum Union {
            A { a: u32 },
        }
        let settings = schemars::generate::SchemaSettings::openapi3();
        let mut generator = schemars::generate::SchemaGenerator::new(settings);
        let schema = Union::json_schema(&mut generator);
        let _ = j2oas_schema(None, &schema);
        for (key, value) in generator.definitions().iter() {
            let schema: &schemars::Schema = value.try_into().unwrap();
            let _ = j2oas_schema(Some(key), schema);
        }
    }

    #[test]
    fn test_nullable() {
        #[allow(dead_code)]
        #[derive(JsonSchema)]
        struct Foo {
            bar: String,
        }
        let settings = schemars::generate::SchemaSettings::openapi3();
        let generator = schemars::generate::SchemaGenerator::new(settings);
        let schema = generator.into_root_schema_for::<Option<Foo>>();
        let os = j2oas_schema_object(None, &schema);

        // Under schemars 1.x's OpenAPI 3.0 settings, `Option<Foo>` becomes
        // `anyOf: [Foo, {nullable: true}]` after the `AddNullable` transform.
        // This shape still semantically represents "nullable Foo" in OpenAPI.
        assert_eq!(
            os,
            openapiv3::ReferenceOr::Item(openapiv3::Schema {
                schema_data: openapiv3::SchemaData {
                    title: Some("Nullable_Foo".to_string()),
                    ..Default::default()
                },
                schema_kind: openapiv3::SchemaKind::AnyOf {
                    any_of: vec![
                        openapiv3::ReferenceOr::Reference {
                            reference: "#/components/schemas/Foo".to_string(),
                        },
                        openapiv3::ReferenceOr::Item(openapiv3::Schema {
                            schema_data: openapiv3::SchemaData {
                                nullable: true,
                                ..Default::default()
                            },
                            schema_kind: openapiv3::SchemaKind::Any(
                                openapiv3::AnySchema::default(),
                            ),
                        }),
                    ],
                },
            })
        );
    }

    /// These cases are errors, but we want to provide good error messages for
    /// them.
    #[test]
    fn test_schema2struct_with_enum_variants() {
        #![allow(dead_code)]

        #[derive(JsonSchema)]
        enum People {
            Alice,
            Bob,
            Mallory,
        }

        #[derive(JsonSchema)]
        enum PeopleWithComments {
            /// Alice
            Alice,

            /// Bob
            Bob,

            // Mallory doesn't get a doc comment but still gets reflected as a
            // complex variant, because the other two have comments.
            Mallory,
        }

        #[derive(JsonSchema)]
        #[schemars(tag = "people")]
        enum PeopleInternallyTagged {
            Alice,
            Bob,
            Mallory,
        }

        #[derive(JsonSchema)]
        #[schemars(tag = "people", content = "name")]
        enum PeopleAdjacentlyTagged {
            Alice,
            Bob,
            Mallory,
        }

        #[derive(JsonSchema)]
        enum PeopleWithData {
            Alice(usize),
            Bob { id: String },
            Mallory,
        }

        #[derive(JsonSchema)]
        #[schemars(tag = "people")]
        enum PeopleWithDataInternallyTagged {
            Alice(usize),
            Bob {
                id: String,
            },
            /// Doc comment!
            Mallory,
        }

        #[derive(JsonSchema)]
        #[schemars(tag = "people")]
        enum PeopleWithDataAdjacentlyTagged {
            Alice(usize),
            Bob {
                id: String,
            },
            /// Doc comment!
            Mallory,
        }

        #[derive(JsonSchema)]
        #[schemars(untagged)]
        enum PeopleUntagged {
            Alice(usize),
            Bob(String),
            Mallory(f32),
        }

        fn assert_enum_error<T: JsonSchema>() {
            let mut generator = schemars::generate::SchemaGenerator::new(
                schemars::generate::SchemaSettings::openapi3(),
            );
            let schema = generator.root_schema_for::<T>();
            let error =
                schema2struct_impl(&schema, &generator, true).unwrap_err();
            println!("for {}: {}\n", T::schema_name(), error);
            assert!(
                matches!(*error, super::Schema2StructError::InvalidEnum(_)),
                "{} correctly recognized as an enum",
                T::schema_name(),
            );
        }

        assert_enum_error::<People>();
        assert_enum_error::<PeopleWithComments>();
        assert_enum_error::<PeopleInternallyTagged>();
        assert_enum_error::<PeopleAdjacentlyTagged>();
        assert_enum_error::<PeopleWithData>();
        assert_enum_error::<PeopleWithDataInternallyTagged>();
        assert_enum_error::<PeopleWithDataAdjacentlyTagged>();
        assert_enum_error::<PeopleUntagged>();
    }

    #[test]
    fn test_embedded_schema() {
        #![allow(unused)]

        #[derive(JsonSchema)]
        #[schemars(tag = "which")]
        enum Which {
            This,
            That,
        }

        #[derive(JsonSchema)]
        struct BlackSheep {
            #[schemars(flatten)]
            you_can_get_with: Which,

            back: String,
            front: Option<String>,
        }

        let schema = schemars::generate::SchemaGenerator::new(
            schemars::generate::SchemaSettings::openapi3(),
        )
        .into_root_schema_for::<BlackSheep>();

        let out = j2oas_schema_object(None, &schema);
        let value = serde_json::to_value(&out).unwrap();

        let expected = serde_json::json!({
            "title": "BlackSheep",
            "type": "object",
            "properties": {
                "back": {
                    "type": "string"
                },
                "front": {
                    "nullable": true,
                    "type": "string"
                }
            },
            "required": [
                "back"
            ],
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "which": {
                            "type": "string",
                            "enum": [
                                "This"
                            ]
                        }
                    },
                    "required": [
                        "which"
                    ]
                },
                {
                    "type": "object",
                    "properties": {
                        "which": {
                            "type": "string",
                            "enum": [
                                "That"
                            ]
                        }
                    },
                    "required": [
                        "which"
                    ]
                }
            ]
        });

        assert_eq!(value, expected);
    }

    /// Under schemars 0.8 a flattened unit-variant enum produced a schema
    /// with both a `SingleOrVec::Vec` instance_type and subschemas, which
    /// wasn't representable in OpenAPI 3.0 and used to panic. In schemars
    /// 1.x the same construct is represented as `type: "object"` alongside
    /// a `oneOf`, which is representable (as an AnySchema) and no longer
    /// panics.
    #[test]
    fn test_two_types() {
        #![allow(unused)]

        #[derive(JsonSchema)]
        enum One {
            One,
        }

        #[derive(JsonSchema)]
        struct Uno {
            #[schemars(flatten)]
            one: One,
        }

        let schema = schemars::schema_for!(Uno);
        let _ = j2oas_schema_object(None, &schema);
    }

    #[test]
    fn test_extension_conversion() {
        let j = serde_json::json!({
            "type": "object",
            "x-stuff": {
                "a": "b",
                "c": [ "d", "e" ]
            }
        });

        let pre: schemars::Schema = j.clone().try_into().unwrap();
        let post = j2oas_schema(None, &pre);
        let v = serde_json::to_value(post.as_item().unwrap()).unwrap();
        assert_eq!(j, v);
    }
}
