// Copyright 2023 Oxide Computer Company

//! schemars helper functions

use schemars::schema::SchemaObject;
use schemars::JsonSchema;

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
///
/// This function is invoked recursively on subschemas.
pub(crate) fn schema2struct(
    schema: &schemars::schema::Schema,
    generator: &schemars::gen::SchemaGenerator,
    required: bool,
) -> Vec<StructMember> {
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
        }) => schema2struct(
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
                    } => results.extend(schemas.iter().flat_map(|subschema| {
                        // Note that these will be tagged as optional.
                        schema2struct(subschema, generator, false)
                    })),

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
                    } if subschemas.len() == 1 => results.extend(
                        subschemas.iter().flat_map(|subschema| {
                            schema2struct(subschema, generator, required)
                        }),
                    ),

                    // We don't expect any other types of subschemas.
                    invalid => panic!("invalid subschema {:#?}", invalid),
                }
            }

            results
        }

        // The generated schema should be an object.
        invalid => panic!("invalid type {:#?}", invalid),
    }
}

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
