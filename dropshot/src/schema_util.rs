// Copyright 2023 Oxide Computer Company

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

        // Unit enums that do not have doc comments are presented as enum_values.
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
        }) => Err(Box::new(Schema2StructError::UnitEnum(schema.clone()))),

        // Complex enums, and unit enums that have doc comments, are presented
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
        }) => Err(Box::new(Schema2StructError::ComplexEnum(schema.clone()))),

        // The generated schema should be an object.
        invalid => {
            Err(Box::new(Schema2StructError::InvalidType(invalid.clone())))
        }
    }
}

#[derive(Debug)]
pub(crate) enum Schema2StructError {
    InvalidType(schemars::schema::Schema),
    InvalidSubschema(schemars::schema::SubschemaValidation),
    UnitEnum(schemars::schema::Schema),
    ComplexEnum(schemars::schema::Schema),
}

impl fmt::Display for Schema2StructError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Schema2StructError::InvalidType(schema) => {
                write!(f, "invalid type {:#?}", schema)
            }
            Schema2StructError::InvalidSubschema(subschemas) => {
                write!(f, "invalid subschema {:#?}", subschemas)
            }
            Schema2StructError::UnitEnum(schema) => {
                write!(
                    f,
                    "invalid type {:#?}\n\
                     (hint: this appears to be an enum with unit variants, \
                     and needs to be wrapped in a struct)",
                    schema
                )
            }
            Schema2StructError::ComplexEnum(schema) => {
                write!(
                    f,
                    "invalid type {:#?}\n\
                     (hint: this appears to be an enum -- \
                     if your enum only has unit variants, \
                     it can be used in this position but needs to be wrapped in a struct)",
                    schema
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

/// Used to visit all schemas and collect all dependencies.
pub(crate) struct ReferenceVisitor<'a> {
    generator: &'a schemars::gen::SchemaGenerator,
    dependencies: indexmap::IndexMap<String, schemars::schema::Schema>,
}

impl<'a> ReferenceVisitor<'a> {
    pub fn new(generator: &'a schemars::gen::SchemaGenerator) -> Self {
        Self { generator, dependencies: indexmap::IndexMap::new() }
    }

    pub fn dependencies(
        self,
    ) -> indexmap::IndexMap<String, schemars::schema::Schema> {
        self.dependencies
    }
}

impl<'a> schemars::visit::Visitor for ReferenceVisitor<'a> {
    fn visit_schema_object(&mut self, schema: &mut SchemaObject) {
        if let Some(refstr) = &schema.reference {
            let definitions_path = &self.generator.settings().definitions_path;
            let name = &refstr[definitions_path.len()..];

            if !self.dependencies.contains_key(name) {
                let mut refschema = self
                    .generator
                    .definitions()
                    .get(name)
                    .expect("invalid reference")
                    .clone();
                self.dependencies.insert(
                    name.to_string(),
                    schemars::schema::Schema::Bool(false),
                );
                schemars::visit::visit_schema(self, &mut refschema);
                self.dependencies.insert(name.to_string(), refschema);
            }
        }

        schemars::visit::visit_schema_object(self, schema);
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

fn j2oas_schema_object(
    name: Option<&String>,
    obj: &schemars::schema::SchemaObject,
) -> openapiv3::ReferenceOr<openapiv3::Schema> {
    if let Some(reference) = &obj.reference {
        return openapiv3::ReferenceOr::Reference {
            reference: reference.clone(),
        };
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

    let kind = match (ty, &obj.subschemas) {
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
            j2oas_object(&obj.object)
        }
        (Some(schemars::schema::InstanceType::Array), None) => {
            j2oas_array(&obj.array)
        }
        (Some(schemars::schema::InstanceType::Number), None) => {
            j2oas_number(&obj.format, &obj.number, &obj.enum_values)
        }
        (Some(schemars::schema::InstanceType::String), None) => {
            j2oas_string(&obj.format, &obj.string, &obj.enum_values)
        }
        (Some(schemars::schema::InstanceType::Integer), None) => {
            j2oas_integer(&obj.format, &obj.number, &obj.enum_values)
        }
        (None, Some(subschema)) => j2oas_subschemas(subschema),
        (None, None) => {
            openapiv3::SchemaKind::Any(openapiv3::AnySchema::default())
        }
        (Some(_), Some(_)) => panic!(
            "a schema can't have both a type and subschemas:\n{}",
            serde_json::to_string_pretty(&obj)
                .unwrap_or_else(|_| "<can't serialize>".to_string())
        ),
    };

    let mut data = openapiv3::SchemaData::default();

    if matches!(
        &obj.extensions.get("nullable"),
        Some(serde_json::Value::Bool(true))
    ) {
        data.nullable = true;
    }

    if let Some(metadata) = &obj.metadata {
        data.title = metadata.title.clone();
        data.description = metadata.description.clone();
        data.default = metadata.default.clone();
        data.deprecated = metadata.deprecated;
        data.read_only = metadata.read_only;
        data.write_only = metadata.write_only;
    }

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

fn j2oas_subschemas(
    subschemas: &schemars::schema::SubschemaValidation,
) -> openapiv3::SchemaKind {
    match (
        &subschemas.all_of,
        &subschemas.any_of,
        &subschemas.one_of,
        &subschemas.not,
    ) {
        (Some(all_of), None, None, None) => openapiv3::SchemaKind::AllOf {
            all_of: all_of
                .iter()
                .map(|schema| j2oas_schema(None, schema))
                .collect::<Vec<_>>(),
        },
        (None, Some(any_of), None, None) => openapiv3::SchemaKind::AnyOf {
            any_of: any_of
                .iter()
                .map(|schema| j2oas_schema(None, schema))
                .collect::<Vec<_>>(),
        },
        (None, None, Some(one_of), None) => openapiv3::SchemaKind::OneOf {
            one_of: one_of
                .iter()
                .map(|schema| j2oas_schema(None, schema))
                .collect::<Vec<_>>(),
        },
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
) -> openapiv3::SchemaKind {
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

    openapiv3::SchemaKind::Type(openapiv3::Type::Number(
        openapiv3::NumberType {
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

fn j2oas_string(
    format: &Option<String>,
    string: &Option<Box<schemars::schema::StringValidation>>,
    enum_values: &Option<Vec<serde_json::value::Value>>,
) -> openapiv3::SchemaKind {
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
            v.iter().map(|vv| match vv {
                serde_json::Value::Null => None,
                serde_json::Value::String(s) => Some(s.clone()),
                _ => panic!("unexpected enumeration value {:?}", vv),
            })
        })
        .collect::<Vec<_>>();

    openapiv3::SchemaKind::Type(openapiv3::Type::String(
        openapiv3::StringType {
            format,
            pattern,
            enumeration,
            min_length,
            max_length,
        },
    ))
}

fn j2oas_array(
    array: &Option<Box<schemars::schema::ArrayValidation>>,
) -> openapiv3::SchemaKind {
    let arr = array.as_ref().unwrap();

    openapiv3::SchemaKind::Type(openapiv3::Type::Array(openapiv3::ArrayType {
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
    }))
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
) -> openapiv3::SchemaKind {
    match object {
        None => openapiv3::SchemaKind::Type(openapiv3::Type::Object(
            openapiv3::ObjectType::default(),
        )),
        Some(obj) => openapiv3::SchemaKind::Type(openapiv3::Type::Object(
            openapiv3::ObjectType {
                properties: obj
                    .properties
                    .iter()
                    .map(|(prop, schema)| {
                        (
                            prop.clone(),
                            box_reference_or(j2oas_schema(None, schema)),
                        )
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
        )),
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

        let mut generator = schemars::gen::SchemaGenerator::new(
            schemars::gen::SchemaSettings::openapi3(),
        );
        {
            let schema: schemars::schema::Schema =
                generator.root_schema_for::<People>().schema.into();

            let error =
                schema2struct_impl(&schema, &generator, true).unwrap_err();
            assert!(
                matches!(*error, super::Schema2StructError::UnitEnum(_)),
                "People consists of unit variants"
            );
        }

        {
            let schema: schemars::schema::Schema =
                generator.root_schema_for::<PeopleWithComments>().schema.into();

            let error =
                schema2struct_impl(&schema, &generator, true).unwrap_err();
            assert!(
                matches!(*error, super::Schema2StructError::ComplexEnum(_)),
                "PeopleWithComments consists of variants that look complex to schemars"
            );
        }
    }

    #[test]
    #[should_panic]
    fn test_bad_schema() {
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
        }

        let schema = schemars::schema_for!(BlackSheep).schema;

        let _ = j2oas_schema_object(None, &schema);
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
}
