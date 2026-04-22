// Copyright 2023 Oxide Computer Company

use crate::api_description::ApiSchemaGenerator;
use crate::pagination::PAGINATION_PARAM_SENTINEL;
use crate::schema_util::schema2struct;
use crate::schema_util::schema_extensions;
use crate::schema_util::StructMember;
use crate::websocket::WEBSOCKET_PARAM_SENTINEL;
use crate::ApiEndpointParameter;
use crate::ApiEndpointParameterLocation;
use crate::ExtensionMode;
use crate::ExtractorMetadata;
use schemars::JsonSchema;

/// Convenience function to generate parameter metadata from types implementing
/// `JsonSchema` for use with `Query` and `Path` `Extractors`.
pub(crate) fn get_metadata<ParamType>(
    loc: &ApiEndpointParameterLocation,
) -> ExtractorMetadata
where
    ParamType: JsonSchema,
{
    let settings = schemars::generate::SchemaSettings::openapi3();

    // Generate the type for `ParamType` then pluck out each member of
    // the structure to encode as an individual parameter.
    let mut generator = schemars::generate::SchemaGenerator::new(settings);
    let schema = generator.root_schema_for::<ParamType>();

    let extension_mode = match schema_extensions(&schema) {
        Some(extensions) => {
            let paginated = extensions.get(PAGINATION_PARAM_SENTINEL);
            let websocket = extensions.get(WEBSOCKET_PARAM_SENTINEL);
            match (paginated, websocket) {
                (None, None) => ExtensionMode::None,
                (None, Some(_)) => ExtensionMode::Websocket,
                (Some(first_page_schema), None) => {
                    ExtensionMode::Paginated(first_page_schema.clone())
                }
                (Some(_), Some(_)) => panic!(
                    "Cannot use websocket and pagination in the same endpoint!"
                ),
            }
        }
        None => ExtensionMode::None,
    };

    let is_header = matches!(loc, ApiEndpointParameterLocation::Header);
    let dependencies: indexmap::IndexMap<String, schemars::Schema> = generator
        .definitions()
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.clone()
                    .try_into()
                    .expect("schemars definition must be an object or bool"),
            )
        })
        .collect();

    // Convert our collection of struct members list of parameters.
    let parameters = schema2struct(
        &ParamType::schema_name(),
        "parameters",
        &schema,
        &generator,
        true,
    )
    .into_iter()
    .map(|StructMember { name, description, mut schema, required }| {
        // Header values can never be null; schemars' OpenAPI 3.0 output
        // attaches `"nullable": true` to `Option<T>` fields, but that's wrong
        // for headers, where an absent value is represented by the absence
        // of the header rather than a null value. Strip it here so the
        // emitted OpenAPI matches the 0.8 behavior of `option_nullable=false`.
        if is_header {
            if let Some(obj) = schema.as_object_mut() {
                obj.remove("nullable");
            }
        }
        ApiEndpointParameter::new_named(
            loc,
            name,
            description,
            required,
            ApiSchemaGenerator::Static {
                schema: Box::new(schema),
                dependencies: dependencies.clone(),
            },
            Vec::new(),
        )
    })
    .collect::<Vec<_>>();

    ExtractorMetadata { extension_mode, parameters }
}

#[cfg(test)]
mod test {
    use crate::api_description::ExtensionMode;
    use crate::{
        api_description::ApiEndpointParameterMetadata, ApiEndpointParameter,
        ApiEndpointParameterLocation, PaginationParams,
    };
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use super::get_metadata;
    use super::ExtractorMetadata;

    #[derive(Deserialize, Serialize, JsonSchema)]
    #[allow(dead_code)]
    struct A {
        foo: String,
        bar: u32,
        baz: Option<String>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct B<T> {
        #[serde(flatten)]
        page: T,

        limit: Option<u64>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    #[schemars(untagged)]
    enum C<T> {
        First(T),
        Next { page_token: String },
    }

    fn compare(
        actual: ExtractorMetadata,
        extension_mode: ExtensionMode,
        parameters: Vec<(&str, bool)>,
    ) {
        assert_eq!(actual.extension_mode, extension_mode);

        // This is order-dependent. We might not really care if the order
        // changes, but it will be interesting to understand why if it does.
        actual.parameters.iter().zip(parameters.iter()).for_each(
            |(param, (name, required))| {
                if let ApiEndpointParameter {
                    metadata: ApiEndpointParameterMetadata::Path(aname),
                    required: arequired,
                    ..
                } = param
                {
                    assert_eq!(aname, name);
                    assert_eq!(arequired, required, "mismatched for {}", name);
                } else {
                    panic!();
                }
            },
        );
    }

    #[test]
    fn test_metadata_simple() {
        let params = get_metadata::<A>(&ApiEndpointParameterLocation::Path);
        let expected = vec![("bar", true), ("baz", false), ("foo", true)];

        compare(params, ExtensionMode::None, expected);
    }

    #[test]
    fn test_metadata_flattened() {
        let params = get_metadata::<B<A>>(&ApiEndpointParameterLocation::Path);
        let expected = vec![
            ("bar", true),
            ("baz", false),
            ("foo", true),
            ("limit", false),
        ];

        compare(params, ExtensionMode::None, expected);
    }

    #[test]
    fn test_metadata_flattened_enum() {
        let params =
            get_metadata::<B<C<A>>>(&ApiEndpointParameterLocation::Path);
        let expected = vec![
            ("limit", false),
            ("bar", false),
            ("baz", false),
            ("foo", false),
            ("page_token", false),
        ];

        compare(params, ExtensionMode::None, expected);
    }

    #[test]
    fn test_metadata_pagination() {
        let params = get_metadata::<PaginationParams<A, A>>(
            &ApiEndpointParameterLocation::Path,
        );
        let expected = vec![
            ("bar", false),
            ("baz", false),
            ("foo", false),
            ("limit", false),
            ("page_token", false),
        ];

        compare(
            params,
            ExtensionMode::Paginated(serde_json::json!(
                {
                    "required": ["foo", "bar"]
                }
            )),
            expected,
        );
    }
}
