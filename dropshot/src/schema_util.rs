// Copyright 2025 Oxide Computer Company

//! schemars helper functions

use schemars::schema::SchemaObject;
use schemars::JsonSchema;

use std::fmt;

#[derive(Debug)]
pub(crate) struct StructMember {
    pub name: String,
    pub description: Option<String>,
    pub schema: schemars::schema::Schema,
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
    schema: &schemars::schema::Schema,
    generator: &schemars::gen::SchemaGenerator,
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

/// This function is invoked recursively on subschemas.
fn schema2struct_impl(
    schema: &schemars::schema::Schema,
    generator: &schemars::gen::SchemaGenerator,
    required: bool,
) -> Result<Vec<StructMember>, Box<Schema2StructError>> {
    // We ignore schema.metadata, which includes things like doc comments, and
    // schema.extensions. We call these out explicitly rather than eliding them
    // as .. since we match all other fields in the structure.
    match schema {
        // We expect references to be on their own.
        schemars::schema::Schema::Object(schemars::schema::SchemaObject {
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
            reference: Some(_),
            extensions: _,
        }) => schema2struct_impl(
            generator.dereference(schema).expect("invalid reference"),
            generator,
            required,
        ),

        // Match objects and subschemas.
        schemars::schema::Schema::Object(schemars::schema::SchemaObject {
            metadata: _,
            instance_type: Some(schemars::schema::SingleOrVec::Single(_)),
            format: None,
            enum_values: None,
            const_value: None,
            subschemas,
            number: None,
            string: None,
            array: None,
            object,
            reference: None,
            extensions: _,
        }) => {
            let mut results = Vec::new();

            // If there's a top-level object, add its members to the list of
            // parameters.
            if let Some(object) = object {
                results.extend(object.properties.iter().map(
                    |(name, schema)| {
                        let (description, schema) =
                            schema_extract_description(schema);
                        StructMember {
                            name: name.clone(),
                            description,
                            schema,
                            required: required
                                && object.required.contains(name),
                        }
                    },
                ));
            }

            // We might see subschemas here in the case of flattened enums
            // or flattened structures that have associated doc comments.
            if let Some(subschemas) = subschemas {
                match subschemas.as_ref() {
                    // We expect any_of in the case of an enum.
                    schemars::schema::SubschemaValidation {
                        all_of: None,
                        any_of: Some(schemas),
                        one_of: None,
                        not: None,
                        if_schema: None,
                        then_schema: None,
                        else_schema: None,
                    } => {
                        for schema in schemas {
                            results.extend(schema2struct_impl(
                                schema, generator, false,
                            )?);
                        }
                    }

                    // With an all_of, there should be a single element. We
                    // typically see this in the case where there is a doc
                    // comment on a structure as OpenAPI 3.0.x doesn't have
                    // a description field directly on schemas.
                    schemars::schema::SubschemaValidation {
                        all_of: Some(subschemas),
                        any_of: None,
                        one_of: None,
                        not: None,
                        if_schema: None,
                        then_schema: None,
                        else_schema: None,
                    } if subschemas.len() == 1 => {
                        results.extend(schema2struct_impl(
                            subschemas.first().unwrap(),
                            generator,
                            required,
                        )?);
                    }

                    // We don't expect any other types of subschemas.
                    invalid => {
                        return Err(Box::new(
                            Schema2StructError::InvalidSubschema(
                                invalid.clone(),
                            ),
                        ))
                    }
                }
            }

            Ok(results)
        }

        // Unit enums that do not have doc comments result in a schema with
        // enum_values.
        schemars::schema::Schema::Object(schemars::schema::SchemaObject {
            metadata: _,
            instance_type: _,
            format: None,
            enum_values: Some(_),
            const_value: None,
            subschemas: None,
            number: None,
            string: None,
            array: None,
            object: None,
            reference: None,
            extensions: _,
        }) => Err(Box::new(Schema2StructError::InvalidEnum(schema.clone()))),

        // Complex enums, and unit enums that have doc comments, are represented
        // as subschemas with instance_type and reference set to None.
        schemars::schema::Schema::Object(schemars::schema::SchemaObject {
            metadata: _,
            instance_type: None,
            format: None,
            enum_values: None,
            const_value: None,
            subschemas: Some(_),
            number: None,
            string: None,
            array: None,
            object: None,
            reference: None,
            extensions: _,
        }) => Err(Box::new(Schema2StructError::InvalidEnum(schema.clone()))),

        // The generated schema should be an object.
        invalid => {
            Err(Box::new(Schema2StructError::InvalidType(invalid.clone())))
        }
    }
}

#[derive(Debug)]
pub(crate) enum Schema2StructError {
    InvalidEnum(schemars::schema::Schema),
    InvalidType(schemars::schema::Schema),
    InvalidSubschema(schemars::schema::SubschemaValidation),
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
            Schema2StructError::InvalidSubschema(subschemas) => {
                write!(
                    f,
                    "invalid subschema: {}",
                    serde_json::to_string_pretty(subschemas).unwrap(),
                )
            }
        }
    }
}

impl std::error::Error for Schema2StructError {}

pub(crate) fn make_subschema_for<T: JsonSchema>(
    gen: &mut schemars::gen::SchemaGenerator,
) -> schemars::schema::Schema {
    gen.subschema_for::<T>()
}

pub(crate) fn schema_extensions(
    schema: &schemars::schema::Schema,
) -> Option<&schemars::Map<String, serde_json::Value>> {
    match schema {
        schemars::schema::Schema::Bool(_) => None,
        schemars::schema::Schema::Object(object) => Some(&object.extensions),
    }
}

pub(crate) fn schema_extract_description(
    schema: &schemars::schema::Schema,
) -> (Option<String>, schemars::schema::Schema) {
    // Because the OpenAPI v3.0.x Schema cannot include a description with
    // a reference, we may see a schema with a description and an `all_of`
    // with a single subschema. In this case, we flatten the trivial subschema.
    if let schemars::schema::Schema::Object(schemars::schema::SchemaObject {
        metadata,
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
        extensions: _,
    }) = schema
    {
        if let schemars::schema::SubschemaValidation {
            all_of: Some(subschemas),
            any_of: None,
            one_of: None,
            not: None,
            if_schema: None,
            then_schema: None,
            else_schema: None,
        } = subschemas.as_ref()
        {
            match (subschemas.first(), subschemas.len()) {
                (Some(subschema), 1) => {
                    let description = metadata
                        .as_ref()
                        .and_then(|m| m.as_ref().description.clone());
                    return (description, subschema.clone());
                }
                _ => (),
            }
        }
    }

    match schema {
        schemars::schema::Schema::Bool(_) => (None, schema.clone()),

        schemars::schema::Schema::Object(object) => {
            let description = object
                .metadata
                .as_ref()
                .and_then(|m| m.as_ref().description.clone());
            (
                description,
                schemars::schema::SchemaObject {
                    metadata: None,
                    ..object.clone()
                }
                .into(),
            )
        }
    }
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
    name: Option<&String>,
    schema: &schemars::schema::Schema,
) -> openapiv3::ReferenceOr<openapiv3::Schema> {
    match schema {
        // The permissive, "match anything" schema. We'll typically see this
        // when consumers use a type such as serde_json::Value.
        schemars::schema::Schema::Bool(true) => {
            openapiv3::ReferenceOr::Item(openapiv3::Schema {
                schema_data: openapiv3::SchemaData::default(),
                schema_kind: openapiv3::SchemaKind::Any(
                    openapiv3::AnySchema::default(),
                ),
            })
        }
        schemars::schema::Schema::Bool(false) => {
            panic!("We don't expect to see a schema that matches the null set")
        }
        schemars::schema::Schema::Object(obj) => j2oas_schema_object(name, obj),
    }
}

fn j2oas_schema_vec(
    schemas: &Option<Vec<schemars::schema::Schema>>,
) -> Vec<openapiv3::ReferenceOr<openapiv3::Schema>> {
    schemas
        .as_ref()
        .map(|v| v.iter().map(|schema| j2oas_schema(None, schema)).collect())
        .unwrap_or_default()
}

fn j2oas_schema_object(
    name: Option<&String>,
    obj: &schemars::schema::SchemaObject,
) -> openapiv3::ReferenceOr<openapiv3::Schema> {
    if let Some(reference) = &obj.reference {
        return openapiv3::ReferenceOr::Reference {
            reference: reference.clone(),
        };
    }

    let kind = j2oas_schema_object_kind(obj);

    let mut data = openapiv3::SchemaData::default();

    if matches!(
        &obj.extensions.get("nullable"),
        Some(serde_json::Value::Bool(true))
    ) {
        data.nullable = true;
    }

    if let Some(metadata) = &obj.metadata {
        data.title.clone_from(&metadata.title);
        data.description.clone_from(&metadata.description);
        data.default.clone_from(&metadata.default);
        data.deprecated = metadata.deprecated;
        data.read_only = metadata.read_only;
        data.write_only = metadata.write_only;
    }

    // Preserve extensions
    data.extensions = obj
        .extensions
        .iter()
        .filter(|(key, _)| key.starts_with("x-"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();

    if let Some(name) = name {
        data.title = Some(name.clone());
    }
    if let Some(example) = obj.extensions.get("example") {
        data.example = Some(example.clone());
    }

    openapiv3::ReferenceOr::Item(openapiv3::Schema {
        schema_data: data,
        schema_kind: kind,
    })
}

fn j2oas_schema_object_kind(
    obj: &schemars::schema::SchemaObject,
) -> openapiv3::SchemaKind {
    // If the JSON schema is attempting to express an unsatisfiable schema
    // using an empty enumerated values array, that presents a problem
    // translating to the openapiv3 crate's representation. While we might
    // like to simply use the schema `false` that is *also* challenging to
    // represent via the openapiv3 crate *and* it precludes us from preserving
    // extensions, if they happen to be present. Instead we'll represent this
    // construction using `{ not: {} }` i.e. the opposite of the permissive
    // schema.
    if let Some(enum_values) = &obj.enum_values {
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

    let ty = match &obj.instance_type {
        Some(schemars::schema::SingleOrVec::Single(ty)) => Some(ty.as_ref()),
        Some(schemars::schema::SingleOrVec::Vec(_)) => {
            panic!(
                "a type array is unsupported by openapiv3:\n{}",
                serde_json::to_string_pretty(obj)
                    .unwrap_or_else(|_| "<can't serialize>".to_string())
            )
        }
        None => None,
    };

    match (ty, &obj.subschemas) {
        (Some(schemars::schema::InstanceType::Null), None) => {
            openapiv3::SchemaKind::Type(openapiv3::Type::String(
                openapiv3::StringType {
                    enumeration: vec![None],
                    ..Default::default()
                },
            ))
        }
        (Some(schemars::schema::InstanceType::Boolean), None) => {
            let enumeration = obj
                .enum_values
                .as_ref()
                .map(|values| {
                    values
                        .iter()
                        .map(|vv| match vv {
                            serde_json::Value::Null => None,
                            serde_json::Value::Bool(b) => Some(*b),
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
        (Some(schemars::schema::InstanceType::Object), None) => {
            openapiv3::SchemaKind::Type(openapiv3::Type::Object(j2oas_object(
                &obj.object,
            )))
        }
        (Some(schemars::schema::InstanceType::Array), None) => {
            openapiv3::SchemaKind::Type(openapiv3::Type::Array(j2oas_array(
                &obj.array,
            )))
        }
        (Some(schemars::schema::InstanceType::Number), None) => {
            openapiv3::SchemaKind::Type(openapiv3::Type::Number(j2oas_number(
                &obj.format,
                &obj.number,
                &obj.enum_values,
            )))
        }
        (Some(schemars::schema::InstanceType::String), None) => {
            openapiv3::SchemaKind::Type(openapiv3::Type::String(j2oas_string(
                &obj.format,
                &obj.string,
                &obj.enum_values,
            )))
        }
        (Some(schemars::schema::InstanceType::Integer), None) => {
            j2oas_integer(&obj.format, &obj.number, &obj.enum_values)
        }
        (None, Some(subschema)) => j2oas_subschemas(subschema),
        (None, None) => {
            openapiv3::SchemaKind::Any(openapiv3::AnySchema::default())
        }
        (Some(_), Some(_)) => j2oas_any(ty, obj),
    }
}

fn j2oas_any(
    ty: Option<&schemars::schema::InstanceType>,
    obj: &SchemaObject,
) -> openapiv3::SchemaKind {
    let typ = ty.map(|ty| {
        match ty {
            schemars::schema::InstanceType::Null => "null",
            schemars::schema::InstanceType::Boolean => "boolean",
            schemars::schema::InstanceType::Object => "object",
            schemars::schema::InstanceType::Array => "array",
            schemars::schema::InstanceType::Number => "number",
            schemars::schema::InstanceType::String => "string",
            schemars::schema::InstanceType::Integer => "integer",
        }
        .to_string()
    });

    let enumeration = match (&obj.enum_values, &obj.const_value) {
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
        format: obj.format.clone(),
        ..Default::default()
    };

    if obj.object.is_some() {
        let openapiv3::ObjectType {
            properties,
            required,
            additional_properties,
            min_properties,
            max_properties,
        } = j2oas_object(&obj.object);
        any.properties = properties;
        any.required = required;
        any.additional_properties = additional_properties;
        any.min_properties = min_properties;
        any.max_properties = max_properties;
    }

    if let Some(av) = &obj.array {
        let openapiv3::ArrayType {
            items,
            min_items,
            max_items,
            unique_items: _,
        } = j2oas_array(&obj.array);
        any.items = items;
        any.min_items = min_items;
        any.max_items = max_items;
        any.unique_items = av.unique_items;
    }

    if obj.string.is_some() {
        let openapiv3::StringType {
            format: _,
            pattern,
            enumeration: _,
            min_length,
            max_length,
        } = j2oas_string(&None, &obj.string, &None);
        any.pattern = pattern;
        any.min_length = min_length;
        any.max_length = max_length;
    }

    if obj.number.is_some() {
        let openapiv3::NumberType {
            format: _,
            multiple_of,
            exclusive_minimum,
            exclusive_maximum,
            minimum,
            maximum,
            enumeration: _,
        } = j2oas_number(&None, &obj.number, &None);
        any.multiple_of = multiple_of;
        any.exclusive_minimum = exclusive_minimum.then_some(true);
        any.exclusive_maximum = exclusive_maximum.then_some(true);
        any.minimum = minimum;
        any.maximum = maximum;
    }

    if let Some(subschemas) = &obj.subschemas {
        any.all_of = j2oas_schema_vec(&subschemas.all_of);
        any.any_of = j2oas_schema_vec(&subschemas.any_of);
        any.one_of = j2oas_schema_vec(&subschemas.one_of);
        any.not = subschemas
            .not
            .as_ref()
            .map(|schema| Box::new(j2oas_schema(None, schema)));
    }

    openapiv3::SchemaKind::Any(any)
}

fn j2oas_subschemas(
    subschemas: &schemars::schema::SubschemaValidation,
) -> openapiv3::SchemaKind {
    match (
        &subschemas.all_of,
        &subschemas.any_of,
        &subschemas.one_of,
        &subschemas.not,
    ) {
        (all_of @ Some(_), None, None, None) => {
            openapiv3::SchemaKind::AllOf { all_of: j2oas_schema_vec(all_of) }
        }
        (None, any_of @ Some(_), None, None) => {
            openapiv3::SchemaKind::AnyOf { any_of: j2oas_schema_vec(any_of) }
        }
        (None, None, one_of @ Some(_), None) => {
            openapiv3::SchemaKind::OneOf { one_of: j2oas_schema_vec(one_of) }
        }
        (None, None, None, Some(not)) => openapiv3::SchemaKind::Not {
            not: Box::new(j2oas_schema(None, not)),
        },
        _ => panic!("invalid subschema {:#?}", subschemas),
    }
}

fn j2oas_integer(
    format: &Option<String>,
    number: &Option<Box<schemars::schema::NumberValidation>>,
    enum_values: &Option<Vec<serde_json::value::Value>>,
) -> openapiv3::SchemaKind {
    let format = match format.as_ref().map(|s| s.as_str()) {
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

    let (multiple_of, minimum, exclusive_minimum, maximum, exclusive_maximum) =
        match number {
            None => (None, None, false, None, false),
            Some(number) => {
                let multiple_of = number.multiple_of.map(|f| f as i64);
                let (minimum, exclusive_minimum) =
                    match (number.minimum, number.exclusive_minimum) {
                        (None, None) => (None, false),
                        (Some(f), None) => (Some(f as i64), false),
                        (None, Some(f)) => (Some(f as i64), true),
                        _ => panic!("invalid"),
                    };
                let (maximum, exclusive_maximum) =
                    match (number.maximum, number.exclusive_maximum) {
                        (None, None) => (None, false),
                        (Some(f), None) => (Some(f as i64), false),
                        (None, Some(f)) => (Some(f as i64), true),
                        _ => panic!("invalid"),
                    };

                (
                    multiple_of,
                    minimum,
                    exclusive_minimum,
                    maximum,
                    exclusive_maximum,
                )
            }
        };

    let enumeration = enum_values
        .iter()
        .flat_map(|v| {
            v.iter().map(|vv| match vv {
                serde_json::Value::Null => None,
                serde_json::Value::Number(value) => {
                    Some(value.as_i64().unwrap())
                }
                _ => panic!("unexpected enumeration value {:?}", vv),
            })
        })
        .collect::<Vec<_>>();

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

fn j2oas_number(
    format: &Option<String>,
    number: &Option<Box<schemars::schema::NumberValidation>>,
    enum_values: &Option<Vec<serde_json::value::Value>>,
) -> openapiv3::NumberType {
    let format = match format.as_ref().map(|s| s.as_str()) {
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

    let (multiple_of, minimum, exclusive_minimum, maximum, exclusive_maximum) =
        match number {
            None => (None, None, false, None, false),
            Some(number) => {
                let multiple_of = number.multiple_of;
                let (minimum, exclusive_minimum) =
                    match (number.minimum, number.exclusive_minimum) {
                        (None, None) => (None, false),
                        (s @ Some(_), None) => (s, false),
                        (None, s @ Some(_)) => (s, true),
                        _ => panic!("invalid"),
                    };
                let (maximum, exclusive_maximum) =
                    match (number.maximum, number.exclusive_maximum) {
                        (None, None) => (None, false),
                        (s @ Some(_), None) => (s, false),
                        (None, s @ Some(_)) => (s, true),
                        _ => panic!("invalid"),
                    };

                (
                    multiple_of,
                    minimum,
                    exclusive_minimum,
                    maximum,
                    exclusive_maximum,
                )
            }
        };

    let enumeration = enum_values
        .iter()
        .flat_map(|v| {
            v.iter().map(|vv| match vv {
                serde_json::Value::Null => None,
                serde_json::Value::Number(value) => {
                    Some(value.as_f64().unwrap())
                }
                _ => panic!("unexpected enumeration value {:?}", vv),
            })
        })
        .collect::<Vec<_>>();

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

fn j2oas_string(
    format: &Option<String>,
    string: &Option<Box<schemars::schema::StringValidation>>,
    enum_values: &Option<Vec<serde_json::value::Value>>,
) -> openapiv3::StringType {
    let format = match format.as_ref().map(|s| s.as_str()) {
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

    let (max_length, min_length, pattern) = match string.as_ref() {
        None => (None, None, None),
        Some(string) => (
            string.max_length.map(|n| n as usize),
            string.min_length.map(|n| n as usize),
            string.pattern.clone(),
        ),
    };

    let enumeration = enum_values
        .iter()
        .flat_map(|v| {
            assert!(!v.is_empty());
            v.iter().map(|vv| match vv {
                serde_json::Value::Null => None,
                serde_json::Value::String(s) => Some(s.clone()),
                _ => panic!("unexpected enumeration value {:?}", vv),
            })
        })
        .collect::<Vec<_>>();

    openapiv3::StringType {
        format,
        pattern,
        enumeration,
        min_length,
        max_length,
    }
}

fn j2oas_array(
    array: &Option<Box<schemars::schema::ArrayValidation>>,
) -> openapiv3::ArrayType {
    let arr = array.as_ref().unwrap();

    openapiv3::ArrayType {
        items: match &arr.items {
            Some(schemars::schema::SingleOrVec::Single(schema)) => {
                Some(box_reference_or(j2oas_schema(None, &schema)))
            }
            Some(schemars::schema::SingleOrVec::Vec(_)) => {
                panic!("OpenAPI v3.0.x cannot support tuple-like arrays")
            }
            None => None,
        },
        min_items: arr.min_items.map(|n| n as usize),
        max_items: arr.max_items.map(|n| n as usize),
        unique_items: arr.unique_items.unwrap_or(false),
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

fn j2oas_object(
    object: &Option<Box<schemars::schema::ObjectValidation>>,
) -> openapiv3::ObjectType {
    match object {
        None => Default::default(),
        Some(obj) => openapiv3::ObjectType {
            properties: obj
                .properties
                .iter()
                .map(|(prop, schema)| {
                    (prop.clone(), box_reference_or(j2oas_schema(None, schema)))
                })
                .collect::<_>(),
            required: obj.required.iter().cloned().collect::<_>(),
            additional_properties: obj.additional_properties.as_ref().map(
                |schema| match schema.as_ref() {
                    schemars::schema::Schema::Bool(b) => {
                        openapiv3::AdditionalProperties::Any(*b)
                    }
                    schemars::schema::Schema::Object(obj) => {
                        openapiv3::AdditionalProperties::Schema(Box::new(
                            j2oas_schema_object(None, obj),
                        ))
                    }
                },
            ),
            min_properties: obj.min_properties.map(|n| n as usize),
            max_properties: obj.max_properties.map(|n| n as usize),
        },
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

        let settings = schemars::gen::SchemaSettings::openapi3();
        let mut generator = schemars::gen::SchemaGenerator::new(settings);

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

        let settings = schemars::gen::SchemaSettings::openapi3();
        let mut generator = schemars::gen::SchemaGenerator::new(settings);

        let schema = SuperGarbage::json_schema(&mut generator);
        let _ = j2oas_schema(None, &schema);
        for (key, schema) in generator.definitions().iter() {
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
        let settings = schemars::gen::SchemaSettings::openapi3();
        let mut generator = schemars::gen::SchemaGenerator::new(settings);
        let schema = Union::json_schema(&mut generator);
        let _ = j2oas_schema(None, &schema);
        for (key, schema) in generator.definitions().iter() {
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
        let settings = schemars::gen::SchemaSettings::openapi3();
        let generator = schemars::gen::SchemaGenerator::new(settings);
        let root_schema = generator.into_root_schema_for::<Option<Foo>>();
        let schema = root_schema.schema;
        let os = j2oas_schema_object(None, &schema);

        assert_eq!(
            os,
            openapiv3::ReferenceOr::Item(openapiv3::Schema {
                schema_data: openapiv3::SchemaData {
                    title: Some("Nullable_Foo".to_string()),
                    nullable: true,
                    ..Default::default()
                },
                schema_kind: openapiv3::SchemaKind::AllOf {
                    all_of: vec![openapiv3::ReferenceOr::Reference {
                        reference: "#/components/schemas/Foo".to_string()
                    }],
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
            let mut generator = schemars::gen::SchemaGenerator::new(
                schemars::gen::SchemaSettings::openapi3(),
            );
            let schema: schemars::schema::Schema =
                generator.root_schema_for::<T>().schema.into();
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

        let schema = schemars::gen::SchemaGenerator::new(
            schemars::gen::SchemaSettings::openapi3(),
        )
        .into_root_schema_for::<BlackSheep>()
        .schema;

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

    #[test]
    #[should_panic]
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

        let schema = schemars::schema_for!(Uno).schema;

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

        let pre: schemars::schema::Schema =
            serde_json::from_value(j.clone()).unwrap();
        let post = j2oas_schema(None, &pre);
        let v = serde_json::to_value(post.as_item().unwrap()).unwrap();
        assert_eq!(j, v);
    }
}
