// Copyright 2026 Oxide Computer Company

//! Generate TypeSpec source from an `ApiDescription`.
//!
//! This module walks the same internal structures as `gen_openapi()` —
//! `ApiEndpoint`, `ApiEndpointParameter`, `ApiSchemaGenerator`, and schemars
//! `Schema` objects — to produce idiomatic TypeSpec output.

use crate::api_description::ApiEndpointBodyContentType;
use crate::api_description::ApiEndpointParameterMetadata;
use crate::api_description::ApiSchemaGenerator;
use crate::api_description::ExtensionMode;
use crate::server::ServerContext;
use crate::ApiDescription;

use indexmap::IndexMap;
use schemars::schema::InstanceType;
use schemars::schema::Schema;
use schemars::schema::SchemaObject;
use schemars::schema::SingleOrVec;

use std::fmt::Write as _;

/// Generate TypeSpec source text from an `ApiDescription`.
pub fn api_to_typespec<C: ServerContext>(
    api: &ApiDescription<C>,
    title: &str,
    version: &semver::Version,
) -> String {
    let mut ctx = TypeSpecContext::new();

    // Collect endpoints and schemas.
    let settings = schemars::gen::SchemaSettings::openapi3();
    let mut generator = schemars::gen::SchemaGenerator::new(settings);
    let mut definitions = IndexMap::<String, Schema>::new();

    let mut ops = Vec::new();

    for (path, method, endpoint) in api.router.endpoints(Some(version)) {
        if !endpoint.visible {
            continue;
        }

        let mut op = OpInfo {
            method: method.to_lowercase(),
            path,
            operation_id: endpoint.operation_id.clone(),
            summary: endpoint.summary.clone(),
            description: endpoint.description.clone(),
            tags: endpoint.tags.clone(),
            deprecated: endpoint.deprecated,
            params: Vec::new(),
            body: None,
            response: ResponseInfo::default(),
            error_type: None,
            extension_mode: endpoint.extension_mode.clone(),
        };

        // Parameters (path, query, header).
        for param in &endpoint.parameters {
            match &param.metadata {
                ApiEndpointParameterMetadata::Body(ct) => {
                    // For octet-stream and multipart bodies, the schema
                    // is `{type: "string", format: "binary"}` which we
                    // represent as `bytes` in TypeSpec.
                    let ts_type = match ct {
                        ApiEndpointBodyContentType::Bytes
                        | ApiEndpointBodyContentType::MultipartFormData => {
                            "bytes".to_string()
                        }
                        _ => schema_gen_to_typespec(
                            &param.schema,
                            &mut generator,
                            &mut definitions,
                        ),
                    };
                    op.body =
                        Some(BodyInfo { ts_type, content_type: ct.clone() });
                }
                ApiEndpointParameterMetadata::Path(name) => {
                    let ts_type = schema_to_typespec_inline(
                        &param.schema,
                        &mut definitions,
                    );
                    op.params.push(ParamInfo {
                        location: ParamLocation::Path,
                        name: name.clone(),
                        ts_type,
                        required: param.required,
                        description: param.description.clone(),
                    });
                }
                ApiEndpointParameterMetadata::Query(name) => {
                    let ts_type = schema_to_typespec_inline(
                        &param.schema,
                        &mut definitions,
                    );
                    op.params.push(ParamInfo {
                        location: ParamLocation::Query,
                        name: name.clone(),
                        ts_type,
                        required: param.required,
                        description: param.description.clone(),
                    });
                }
                ApiEndpointParameterMetadata::Header(name) => {
                    let ts_type = schema_to_typespec_inline(
                        &param.schema,
                        &mut definitions,
                    );
                    op.params.push(ParamInfo {
                        location: ParamLocation::Header,
                        name: name.clone(),
                        ts_type,
                        required: param.required,
                        description: param.description.clone(),
                    });
                }
            }
        }

        // Response.
        if let Some(schema) = &endpoint.response.schema {
            let ts_type = schema_gen_to_typespec(
                schema,
                &mut generator,
                &mut definitions,
            );
            op.response.ts_type = Some(ts_type);
        }
        if let Some(code) = &endpoint.response.success {
            op.response.status_code = Some(code.as_u16());
        }
        if let Some(desc) = &endpoint.response.description {
            op.response.description = Some(desc.clone());
        }
        // When both schema and status code are None, the endpoint returns
        // a hand-rolled Response<Body> (freeform).
        if endpoint.response.schema.is_none()
            && endpoint.response.success.is_none()
        {
            op.response.freeform = true;
        }

        // Response headers.
        for header in &endpoint.response.headers {
            let ts_type =
                schema_to_typespec_inline(&header.schema, &mut definitions);
            op.response.headers.push(HeaderInfo {
                name: header.name.clone(),
                ts_type,
                required: header.required,
            });
        }

        // Error type.
        if let Some(error) = &endpoint.error {
            let error_name = schema_gen_to_typespec(
                &error.schema,
                &mut generator,
                &mut definitions,
            );
            op.error_type = Some(error_name);
        }

        ops.push(op);
    }

    // Collect all definitions from the schemars generator.
    let root_schema = generator.into_root_schema_for::<()>();
    for (name, schema) in &root_schema.definitions {
        if !definitions.contains_key(name) {
            definitions.insert(name.clone(), schema.clone());
        }
    }

    // Detect ResultsPage instantiations and build a rewrite map.
    let results_pages = detect_results_pages(&definitions);
    ctx.results_page_rewrites = results_pages;

    // Collect error type names and freeform flag from operations.
    for op in &ops {
        if let Some(error_name) = &op.error_type {
            ctx.error_types.insert(error_name.clone());
        }
        if op.response.freeform {
            ctx.needs_freeform_response = true;
        }
    }

    // Check if any schema uses format: "uuid" so we emit the uuid scalar.
    ctx.needs_uuid_scalar =
        definitions.values().any(|schema| schema_uses_uuid_format(schema));

    // Pre-scan definitions to identify enums and discriminated unions.
    // This context is used when emitting default values.
    ctx.known_enums = detect_enum_types(&definitions);
    ctx.discriminated_unions = detect_discriminated_union_types(&definitions);

    // Emit preamble.
    ctx.emit_preamble(title, version);

    // Emit the generic ResultsPage model if any instantiations were found.
    if !ctx.results_page_rewrites.is_empty() {
        writeln!(ctx.out, "model ResultsPage<T> {{").unwrap();
        writeln!(ctx.out, "  @doc(\"list of items on this page of results\")")
            .unwrap();
        writeln!(ctx.out, "  items: T[];").unwrap();
        writeln!(
            ctx.out,
            "  @doc(\"token used to fetch the next page of results (if any)\")"
        )
        .unwrap();
        writeln!(ctx.out, "  next_page?: string | null;").unwrap();
        writeln!(ctx.out, "}}").unwrap();
        writeln!(ctx.out).unwrap();
    }

    // Emit uuid scalar if any schema uses format: "uuid".
    if ctx.needs_uuid_scalar {
        writeln!(ctx.out, "@format(\"uuid\")").unwrap();
        writeln!(ctx.out, "scalar uuid extends string;").unwrap();
        writeln!(ctx.out).unwrap();
    }

    // Emit the FreeformResponse model if any operation needs it.
    // @defaultResponse tells TypeSpec/OpenAPI this is the catch-all response
    // (rendered as `default` in OpenAPI, matching dropshot's behavior).
    if ctx.needs_freeform_response {
        writeln!(ctx.out, "@doc(\"Response with untyped body\")").unwrap();
        writeln!(ctx.out, "@defaultResponse").unwrap();
        writeln!(ctx.out, "model FreeformResponse {{").unwrap();
        writeln!(ctx.out, "  @body body: bytes;").unwrap();
        writeln!(ctx.out, "}}").unwrap();
        writeln!(ctx.out).unwrap();
    }

    // Emit model definitions (skipping ResultsPage instantiations).
    for (name, schema) in &definitions {
        if ctx.results_page_rewrites.contains_key(name.as_str()) {
            continue;
        }
        ctx.emit_model(name, schema);
    }

    if !definitions.is_empty() && !ops.is_empty() {
        ctx.out.push('\n');
    }

    // Emit operations.
    for (i, op) in ops.iter().enumerate() {
        if i > 0 {
            ctx.out.push('\n');
        }
        ctx.emit_op(op);
    }

    // Rewrite concrete ResultsPage names to generic form.
    // Sort by name length descending so longer names are replaced first,
    // preventing shorter names from matching as substrings within longer
    // ones (e.g. "DiskResultsPage" within "PhysicalDiskResultsPage").
    let mut rewrites: Vec<_> = ctx.results_page_rewrites.iter().collect();
    rewrites.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (concrete, item_type) in rewrites {
        let generic = format!("ResultsPage<{}>", item_type);
        ctx.out = ctx.out.replace(concrete.as_str(), &generic);
    }

    ctx.out
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

struct TypeSpecContext {
    out: String,
    /// Map from concrete ResultsPage name (e.g. "WidgetResultsPage") to the
    /// item type name (e.g. "Widget"). When a ref targets one of these, we
    /// emit `ResultsPage<Widget>` instead.
    results_page_rewrites: std::collections::HashMap<String, String>,
    /// Set of model names that are error types (should get `@error` decorator).
    error_types: std::collections::HashSet<String>,
    /// Whether any operation uses a freeform response.
    needs_freeform_response: bool,
    /// Whether any type references `uuid` (needs scalar definition).
    needs_uuid_scalar: bool,
    /// Map from enum type name to its member values. Used to emit enum
    /// defaults as `EnumName.member` instead of `"member"`.
    known_enums: std::collections::HashMap<String, Vec<String>>,
    /// Set of type names that are discriminated unions. Object defaults
    /// on these types are skipped because they reference variant-specific
    /// fields not present on the base model.
    discriminated_unions: std::collections::HashSet<String>,
}

impl TypeSpecContext {
    fn new() -> Self {
        Self {
            out: String::new(),
            results_page_rewrites: std::collections::HashMap::new(),
            error_types: std::collections::HashSet::new(),
            needs_freeform_response: false,
            needs_uuid_scalar: false,
            known_enums: std::collections::HashMap::new(),
            discriminated_unions: std::collections::HashSet::new(),
        }
    }

    fn emit_preamble(&mut self, title: &str, version: &semver::Version) {
        writeln!(self.out, "import \"@typespec/http\";").unwrap();
        writeln!(self.out, "import \"@typespec/openapi\";").unwrap();
        writeln!(self.out).unwrap();
        writeln!(self.out, "using Http;").unwrap();
        writeln!(self.out, "using OpenAPI;").unwrap();
        writeln!(self.out).unwrap();
        writeln!(
            self.out,
            "@service(#{{ title: \"{}\" }})",
            escape_tsp_string(title)
        )
        .unwrap();
        writeln!(self.out, "@info(#{{ version: \"{}\" }})", version).unwrap();
        writeln!(self.out, "namespace {};", to_namespace_id(title)).unwrap();
        writeln!(self.out).unwrap();
    }

    fn emit_model(&mut self, name: &str, schema: &Schema) {
        match schema {
            Schema::Object(obj) => self.emit_model_from_object(name, obj),
            Schema::Bool(true) => {
                writeln!(self.out, "model {} is Record<unknown>;", name)
                    .unwrap();
                writeln!(self.out).unwrap();
            }
            Schema::Bool(false) => {
                writeln!(self.out, "model {} is never;", name).unwrap();
                writeln!(self.out).unwrap();
            }
        }
    }

    fn emit_model_from_object(&mut self, name: &str, obj: &SchemaObject) {
        // String enums → TypeSpec enum.
        if let Some(enum_values) = &obj.enum_values {
            self.emit_enum(name, obj, enum_values);
            return;
        }

        // oneOf → TypeSpec union.
        if let Some(subschemas) = &obj.subschemas {
            if let Some(one_of) = &subschemas.one_of {
                self.emit_union(name, obj, one_of);
                return;
            }
        }

        // Object type → model with properties.
        if let Some(object) = &obj.object {
            self.emit_doc_from_metadata(obj.metadata.as_deref());
            if self.error_types.contains(name) {
                writeln!(self.out, "@error").unwrap();
            }
            writeln!(self.out, "model {} {{", name).unwrap();

            // Error models get a synthetic @statusCode field so TypeSpec
            // knows which HTTP status codes this error covers.
            if self.error_types.contains(name) {
                writeln!(self.out, "  @minValue(400)").unwrap();
                writeln!(self.out, "  @maxValue(599)").unwrap();
                writeln!(self.out, "  @statusCode statusCode: int32;").unwrap();
            }

            let required_set: std::collections::HashSet<&str> =
                object.required.iter().map(|s| s.as_str()).collect();

            for (prop_name, prop_schema) in &object.properties {
                let ts_type = schemars_to_typespec(prop_schema);
                let optional = if required_set.contains(prop_name.as_str()) {
                    ""
                } else {
                    "?"
                };
                let desc = schema_description(prop_schema);
                if let Some(d) = &desc {
                    writeln!(self.out, "  @doc(\"{}\")", escape_tsp_string(d))
                        .unwrap();
                }
                emit_validation_decorators(&mut self.out, prop_schema, "  ");
                let default_suffix = schema_default(prop_schema)
                    .and_then(|d| self.refine_default(&ts_type, d))
                    .map(|d| format!(" = {}", d))
                    .unwrap_or_default();
                writeln!(
                    self.out,
                    "  {}{}: {}{};",
                    escape_property_name(prop_name),
                    optional,
                    ts_type,
                    default_suffix,
                )
                .unwrap();
            }

            // additional_properties → spread Record<T>
            if let Some(ap) = &object.additional_properties {
                match ap.as_ref() {
                    Schema::Bool(true) => {
                        writeln!(self.out, "  ...Record<unknown>;").unwrap();
                    }
                    Schema::Object(_) => {
                        let ts_type = schemars_to_typespec(ap);
                        writeln!(self.out, "  ...Record<{}>;", ts_type)
                            .unwrap();
                    }
                    _ => {}
                }
            }

            writeln!(self.out, "}}").unwrap();
            writeln!(self.out).unwrap();
            return;
        }

        // Non-object type alias (e.g. a named string type).
        let ts_type = schemars_obj_to_typespec(obj);
        self.emit_doc_from_metadata(obj.metadata.as_deref());

        // If the resolved type is a union (e.g. from anyOf/oneOf), emit as
        // a TypeSpec `union` rather than `scalar extends` — scalars can only
        // extend a single base type.
        if ts_type.contains(" | ") {
            writeln!(self.out, "union {} {{", name).unwrap();
            for variant in ts_type.split(" | ") {
                writeln!(self.out, "  {},", variant).unwrap();
            }
            writeln!(self.out, "}}").unwrap();
        } else if ts_type == "unknown" {
            // `unknown` is a TypeSpec keyword; can't use it as a
            // scalar base. Emit a type alias instead.
            writeln!(self.out, "alias {} = unknown;", name).unwrap();
        } else {
            emit_validation_decorators(
                &mut self.out,
                &Schema::Object(obj.clone()),
                "",
            );
            writeln!(self.out, "scalar {} extends {};", name, ts_type).unwrap();
        }
        writeln!(self.out).unwrap();
    }

    fn emit_enum(
        &mut self,
        name: &str,
        obj: &SchemaObject,
        values: &[serde_json::Value],
    ) {
        self.emit_doc_from_metadata(obj.metadata.as_deref());
        writeln!(self.out, "enum {} {{", name).unwrap();
        for (i, val) in values.iter().enumerate() {
            let comma = if i + 1 < values.len() { "," } else { "" };
            match val {
                serde_json::Value::String(s) => {
                    // Use the string as the enum member name if it's a
                    // valid identifier, otherwise quote it.
                    if is_valid_tsp_ident(s) {
                        writeln!(self.out, "  {}{}", s, comma).unwrap();
                    } else {
                        writeln!(
                            self.out,
                            "  `{}`: \"{}\"{comma}",
                            s,
                            escape_tsp_string(s),
                        )
                        .unwrap();
                    }
                }
                serde_json::Value::Number(n) => {
                    writeln!(self.out, "  value{}: {}{}", i, n, comma).unwrap();
                }
                _ => {
                    writeln!(self.out, "  // unsupported enum value: {}", val)
                        .unwrap();
                }
            }
        }
        writeln!(self.out, "}}").unwrap();
        writeln!(self.out).unwrap();
    }

    fn emit_union(
        &mut self,
        name: &str,
        obj: &SchemaObject,
        variants: &[Schema],
    ) {
        // Detect described string enums: oneOf where every variant is
        // {type: "string", enum: ["SingleValue"]} with an optional
        // description. Emit as a TypeSpec enum with @doc per member.
        if let Some(members) = detect_described_string_enum(variants) {
            self.emit_doc_from_metadata(obj.metadata.as_deref());
            writeln!(self.out, "enum {} {{", name).unwrap();
            for (i, member) in members.iter().enumerate() {
                let comma = if i + 1 < members.len() { "," } else { "" };
                if let Some(desc) = &member.description {
                    writeln!(
                        self.out,
                        "  @doc(\"{}\")",
                        escape_tsp_string(desc)
                    )
                    .unwrap();
                }
                if is_valid_tsp_ident(&member.value) {
                    writeln!(self.out, "  {}{}", member.value, comma).unwrap();
                } else {
                    writeln!(
                        self.out,
                        "  `{}`: \"{}\"{comma}",
                        member.value,
                        escape_tsp_string(&member.value),
                    )
                    .unwrap();
                }
            }
            writeln!(self.out, "}}").unwrap();
            writeln!(self.out).unwrap();
            return;
        }

        // Detect discriminated unions: oneOf where each variant is an
        // object with a shared property whose schema is a single-value
        // string enum (the tag). Emit as model inheritance with
        // @discriminator on the base model.
        if let Some(mut disc) = detect_discriminator(variants) {
            // Qualify variant model names with the parent name.
            for v in &mut disc.variants {
                v.model_name =
                    format!("{}{}", name, to_pascal_case(&v.tag_value));
            }

            self.emit_doc_from_metadata(obj.metadata.as_deref());
            writeln!(self.out, "@discriminator(\"{}\")", disc.tag).unwrap();
            writeln!(self.out, "model {} {{", name).unwrap();
            writeln!(self.out, "  {}: string;", disc.tag).unwrap();
            writeln!(self.out, "}}").unwrap();
            writeln!(self.out).unwrap();
            // Emit variant models that extend the base.
            for variant in &disc.variants {
                if let Some(desc) = &variant.description {
                    writeln!(self.out, "@doc(\"{}\")", escape_tsp_string(desc))
                        .unwrap();
                }
                writeln!(
                    self.out,
                    "model {} extends {} {{",
                    variant.model_name, name,
                )
                .unwrap();
                writeln!(
                    self.out,
                    "  {}: \"{}\";",
                    disc.tag, variant.tag_value
                )
                .unwrap();
                for prop in &variant.properties {
                    if let Some(d) = &prop.description {
                        writeln!(
                            self.out,
                            "  @doc(\"{}\")",
                            escape_tsp_string(d),
                        )
                        .unwrap();
                    }
                    emit_validation_decorators(
                        &mut self.out,
                        &prop.schema,
                        "  ",
                    );
                    let default_suffix = schema_default(&prop.schema)
                        .and_then(|d| self.refine_default(&prop.ts_type, d))
                        .map(|d| format!(" = {}", d))
                        .unwrap_or_default();
                    writeln!(
                        self.out,
                        "  {}{}: {}{};",
                        escape_property_name(&prop.name),
                        if prop.required { "" } else { "?" },
                        prop.ts_type,
                        default_suffix,
                    )
                    .unwrap();
                }
                writeln!(self.out, "}}").unwrap();
                writeln!(self.out).unwrap();
            }
            return;
        }

        // Externally-tagged unions: each variant is an object with a
        // single required property (the variant key).
        if let Some(mut ext) = detect_externally_tagged_union(variants) {
            for v in &mut ext.variants {
                v.model_name = format!("{}{}", name, to_pascal_case(&v.key));
            }

            // Emit variant models.
            for v in &ext.variants {
                writeln!(self.out, "model {} {{", v.model_name).unwrap();
                writeln!(
                    self.out,
                    "  {}: {};",
                    escape_property_name(&v.key),
                    v.value_type,
                )
                .unwrap();
                writeln!(self.out, "}}").unwrap();
                writeln!(self.out).unwrap();
            }

            // Emit union referencing variant models.
            self.emit_doc_from_metadata(obj.metadata.as_deref());
            writeln!(self.out, "union {} {{", name).unwrap();
            for v in &ext.variants {
                writeln!(self.out, "  {},", v.model_name).unwrap();
            }
            writeln!(self.out, "}}").unwrap();
            writeln!(self.out).unwrap();
            return;
        }

        // Non-discriminated union: emit as TypeSpec union with inline types.
        // Preserve `title` from variant metadata as named union members.
        self.emit_doc_from_metadata(obj.metadata.as_deref());
        writeln!(self.out, "union {} {{", name).unwrap();
        for variant in variants {
            let ts_type = schemars_to_typespec(variant);
            let title = match variant {
                Schema::Object(v) => {
                    v.metadata.as_ref().and_then(|m| m.title.clone())
                }
                _ => None,
            };
            if let Some(title) = title {
                writeln!(
                    self.out,
                    "  {}: {},",
                    escape_property_name(&title),
                    ts_type,
                )
                .unwrap();
            } else {
                writeln!(self.out, "  {},", ts_type).unwrap();
            }
        }
        writeln!(self.out, "}}").unwrap();
        writeln!(self.out).unwrap();
    }

    fn emit_op(&mut self, op: &OpInfo) {
        // Doc comment.
        let doc = match (&op.summary, &op.description) {
            (Some(s), Some(d)) => Some(format!("{}\n\n{}", s, d)),
            (Some(s), None) => Some(s.clone()),
            (None, Some(d)) => Some(d.clone()),
            (None, None) => None,
        };
        if let Some(doc) = doc {
            writeln!(self.out, "@doc(\"{}\")", escape_tsp_string(&doc))
                .unwrap();
        }

        // Tags.
        for tag in &op.tags {
            writeln!(self.out, "@tag(\"{}\")", escape_tsp_string(tag)).unwrap();
        }

        // Dropshot extensions.
        match &op.extension_mode {
            ExtensionMode::None => {}
            ExtensionMode::Paginated(value) => {
                let tsp_val = json_to_tsp_literal(value);
                writeln!(
                    self.out,
                    "@extension(\"x-dropshot-pagination\", {})",
                    tsp_val,
                )
                .unwrap();
            }
            ExtensionMode::Websocket => {
                writeln!(
                    self.out,
                    "@extension(\"x-dropshot-websocket\", #{{}})",
                )
                .unwrap();
            }
        }

        // Deprecated.
        if op.deprecated {
            writeln!(self.out, "#deprecated \"deprecated\"").unwrap();
        }

        // Route and method.
        writeln!(self.out, "@route(\"{}\")", op.path).unwrap();
        writeln!(self.out, "@{}", op.method).unwrap();

        // Operation signature.
        write!(self.out, "op {}(", op.operation_id).unwrap();

        // Each param part is (optional @doc string, param line).
        let mut params_parts: Vec<(Option<&str>, String)> = Vec::new();

        // Path, query, header params.
        for p in &op.params {
            let decorator = match p.location {
                ParamLocation::Path => "@path",
                ParamLocation::Query => "@query",
                ParamLocation::Header => "@header",
            };
            let optional = if p.required { "" } else { "?" };
            params_parts.push((
                p.description.as_deref(),
                format!(
                    "{} {}{}: {}",
                    decorator,
                    escape_property_name(&p.name),
                    optional,
                    p.ts_type,
                ),
            ));
        }

        // Body param.
        if let Some(body) = &op.body {
            match body.content_type {
                ApiEndpointBodyContentType::Json => {
                    params_parts
                        .push((None, format!("@body body: {}", body.ts_type)));
                }
                ApiEndpointBodyContentType::Bytes => {
                    params_parts.push((
                        None,
                        format!(
                            "@header contentType: \
                             \"application/octet-stream\", \
                             @body body: {}",
                            body.ts_type
                        ),
                    ));
                }
                ApiEndpointBodyContentType::UrlEncoded => {
                    params_parts.push((
                        None,
                        format!(
                            "@header contentType: \
                             \"application/x-www-form-urlencoded\", \
                             @body body: {}",
                            body.ts_type
                        ),
                    ));
                }
                ApiEndpointBodyContentType::MultipartFormData => {
                    params_parts.push((
                        None,
                        format!(
                            "@header contentType: \
                             \"multipart/form-data\", \
                             @body body: {}",
                            body.ts_type
                        ),
                    ));
                }
            }
        }

        let has_descriptions = params_parts.iter().any(|(d, _)| d.is_some());
        if params_parts.len() <= 2 && !has_descriptions {
            // Inline params.
            let parts: Vec<&str> =
                params_parts.iter().map(|(_, s)| s.as_str()).collect();
            write!(self.out, "{}", parts.join(", ")).unwrap();
            write!(self.out, ")").unwrap();
        } else {
            // Multi-line params.
            writeln!(self.out).unwrap();
            for (i, (desc, part)) in params_parts.iter().enumerate() {
                if let Some(d) = desc {
                    writeln!(self.out, "  @doc(\"{}\")", escape_tsp_string(d),)
                        .unwrap();
                }
                let comma = if i + 1 < params_parts.len() { "," } else { "" };
                writeln!(self.out, "  {}{}", part, comma).unwrap();
            }
            write!(self.out, ")").unwrap();
        }

        // Return type.
        let return_type =
            self.build_return_type(&op.response, op.error_type.as_deref());
        writeln!(self.out, ": {};", return_type).unwrap();
    }

    fn build_return_type(
        &self,
        resp: &ResponseInfo,
        error_type: Option<&str>,
    ) -> String {
        // Freeform responses (hand-rolled Response<Body>) get the
        // FreeformResponse model — no typed body or status code.
        if resp.freeform {
            return match error_type {
                Some(e) => format!("FreeformResponse | {}", e),
                None => "FreeformResponse".to_string(),
            };
        }

        let body_type = match resp.ts_type.as_deref() {
            // "null" comes from `()`, "never" from `Schema::Bool(false)`
            // (no-content types like HttpResponseDeleted). Both mean void.
            Some("null") | Some("never") | None => "void",
            Some(t) => t,
        };

        let has_headers = !resp.headers.is_empty();
        let needs_block = has_headers
            || !matches!(resp.status_code, Some(200) | Some(204) | None);

        let success_type = if !needs_block {
            body_type.to_string()
        } else {
            let mut parts = Vec::new();
            if let Some(code) = resp.status_code {
                if code != 200 {
                    parts.push(format!("  @statusCode _: {};", code));
                }
            }
            for h in &resp.headers {
                let optional = if h.required { "" } else { "?" };
                parts.push(format!(
                    "  @header {}{}: {};",
                    escape_property_name(&h.name),
                    optional,
                    h.ts_type
                ));
            }
            if body_type != "void" {
                parts.push(format!("  @body body: {};", body_type));
            }

            format!("{{\n{}\n}}", parts.join("\n"))
        };

        match error_type {
            Some(e) => format!("{} | {}", success_type, e),
            None => success_type,
        }
    }

    /// Refine a default value literal using type context.
    ///
    /// - String defaults on enum types → `EnumName.member`
    /// - Object defaults on discriminated union types → dropped (returns
    ///   `None`) because they reference variant-specific fields
    fn refine_default(
        &self,
        ts_type: &str,
        default_literal: String,
    ) -> Option<String> {
        // Enum member defaults: "v4" → IpVersion.v4
        if let Some(members) = self.known_enums.get(ts_type) {
            // default_literal is e.g. `"v4"` — strip quotes to get the
            // member name.
            if default_literal.starts_with('"')
                && default_literal.ends_with('"')
            {
                let inner = &default_literal[1..default_literal.len() - 1];
                if members.iter().any(|m| m == inner) {
                    return Some(format!("{}.{}", ts_type, inner));
                }
            }
        }

        // Object defaults on discriminated unions are invalid because
        // they reference variant-specific fields not on the base model.
        if self.discriminated_unions.contains(ts_type)
            && default_literal.starts_with("#{")
        {
            return None;
        }

        Some(default_literal)
    }

    fn emit_doc_from_metadata(
        &mut self,
        metadata: Option<&schemars::schema::Metadata>,
    ) {
        if let Some(meta) = metadata {
            if let Some(desc) = &meta.description {
                writeln!(self.out, "@doc(\"{}\")", escape_tsp_string(desc))
                    .unwrap();
            }
        }
    }
}

#[derive(Default)]
struct ResponseInfo {
    ts_type: Option<String>,
    status_code: Option<u16>,
    description: Option<String>,
    headers: Vec<HeaderInfo>,
    /// True when the endpoint returns a hand-rolled `Response<Body>` — both
    /// schema and status code are unknown. Rendered as `FreeformResponse`.
    freeform: bool,
}

struct HeaderInfo {
    name: String,
    ts_type: String,
    required: bool,
}

struct OpInfo {
    method: String,
    path: String,
    operation_id: String,
    summary: Option<String>,
    description: Option<String>,
    tags: Vec<String>,
    deprecated: bool,
    params: Vec<ParamInfo>,
    body: Option<BodyInfo>,
    response: ResponseInfo,
    error_type: Option<String>,
    extension_mode: ExtensionMode,
}

struct ParamInfo {
    location: ParamLocation,
    name: String,
    ts_type: String,
    required: bool,
    description: Option<String>,
}

struct BodyInfo {
    ts_type: String,
    content_type: ApiEndpointBodyContentType,
}

enum ParamLocation {
    Path,
    Query,
    Header,
}

// ---------------------------------------------------------------------------
// Schema → TypeSpec conversion
// ---------------------------------------------------------------------------

/// Convert an `ApiSchemaGenerator` to a TypeSpec type string, using the
/// schemars generator for dynamic schemas.
fn schema_gen_to_typespec(
    gen: &ApiSchemaGenerator,
    generator: &mut schemars::gen::SchemaGenerator,
    definitions: &mut IndexMap<String, Schema>,
) -> String {
    match gen {
        ApiSchemaGenerator::Gen { name, schema } => {
            let name_str = name();
            let js = schema(generator);
            // If it produced a reference, the type lives in definitions —
            // just emit the name.
            if is_ref_schema(&js) {
                name_str
            } else if is_empty_schema(&js) {
                "void".to_string()
            } else {
                // Inline / anonymous schema — convert directly.
                schemars_to_typespec(&js)
            }
        }
        ApiSchemaGenerator::Static { schema, dependencies } => {
            definitions.extend(dependencies.clone());
            schemars_to_typespec(schema)
        }
    }
}

/// Convert a static `ApiSchemaGenerator` to a TypeSpec type string (for
/// parameters which are always Static).
fn schema_to_typespec_inline(
    gen: &ApiSchemaGenerator,
    definitions: &mut IndexMap<String, Schema>,
) -> String {
    match gen {
        ApiSchemaGenerator::Static { schema, dependencies } => {
            definitions.extend(dependencies.clone());
            schemars_to_typespec(schema)
        }
        ApiSchemaGenerator::Gen { name, .. } => {
            // Parameters shouldn't normally be Gen, but handle gracefully.
            name()
        }
    }
}

/// Convert a schemars `Schema` to a TypeSpec type string.
fn schemars_to_typespec(schema: &Schema) -> String {
    match schema {
        Schema::Bool(true) => "unknown".to_string(),
        Schema::Bool(false) => "never".to_string(),
        Schema::Object(obj) => schemars_obj_to_typespec(obj),
    }
}

/// Convert a schemars `SchemaObject` to a TypeSpec type string.
fn schemars_obj_to_typespec(obj: &SchemaObject) -> String {
    let nullable = obj
        .extensions
        .get("nullable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let base = schemars_obj_to_typespec_inner(obj);

    if nullable {
        format!("{} | null", base)
    } else {
        base
    }
}

fn schemars_obj_to_typespec_inner(obj: &SchemaObject) -> String {
    // Reference → model name.
    if let Some(reference) = &obj.reference {
        return ref_to_type_name(reference);
    }

    // Enum values (inline).
    if let Some(enum_values) = &obj.enum_values {
        let parts: Vec<String> = enum_values
            .iter()
            .map(|v| match v {
                serde_json::Value::String(s) => {
                    format!("\"{}\"", escape_tsp_string(s))
                }
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Null => "null".to_string(),
                _ => "unknown".to_string(),
            })
            .collect();
        return parts.join(" | ");
    }

    // oneOf → union.
    if let Some(subschemas) = &obj.subschemas {
        if let Some(one_of) = &subschemas.one_of {
            let parts: Vec<String> =
                one_of.iter().map(|s| schemars_to_typespec(s)).collect();
            return parts.join(" | ");
        }
        if let Some(any_of) = &subschemas.any_of {
            let parts: Vec<String> =
                any_of.iter().map(|s| schemars_to_typespec(s)).collect();
            return parts.join(" | ");
        }
        if let Some(all_of) = &subschemas.all_of {
            let parts: Vec<String> =
                all_of.iter().map(|s| schemars_to_typespec(s)).collect();
            return parts.join(" & ");
        }
    }

    let instance_type = match &obj.instance_type {
        Some(SingleOrVec::Single(t)) => Some(t.as_ref()),
        Some(SingleOrVec::Vec(types)) => {
            // Nullable pattern: [Type, "null"]
            let non_null: Vec<&InstanceType> =
                types.iter().filter(|t| **t != InstanceType::Null).collect();
            if non_null.len() == 1 {
                // Reconstruct a simpler object for the non-null type and
                // append "| null".
                let inner = instance_type_to_typespec(
                    non_null[0],
                    &obj.format,
                    &obj.array,
                    &obj.object,
                );
                return format!("{} | null", inner);
            }
            // Multiple types → union.
            let parts: Vec<String> = types
                .iter()
                .map(|t| {
                    instance_type_to_typespec(
                        t,
                        &obj.format,
                        &obj.array,
                        &obj.object,
                    )
                })
                .collect();
            return parts.join(" | ");
        }
        None => None,
    };

    match instance_type {
        Some(it) => {
            instance_type_to_typespec(it, &obj.format, &obj.array, &obj.object)
        }
        None => "unknown".to_string(),
    }
}

fn instance_type_to_typespec(
    it: &InstanceType,
    format: &Option<String>,
    array: &Option<Box<schemars::schema::ArrayValidation>>,
    object: &Option<Box<schemars::schema::ObjectValidation>>,
) -> String {
    match it {
        InstanceType::Null => "null".to_string(),
        InstanceType::Boolean => "boolean".to_string(),
        InstanceType::String => match format.as_deref() {
            Some("date") => "plainDate".to_string(),
            Some("date-time") => "utcDateTime".to_string(),
            Some("uuid") => "uuid".to_string(),
            Some("ip") | Some("ipv4") | Some("ipv6") => "string".to_string(),
            Some("uri") => "url".to_string(),
            _ => "string".to_string(),
        },
        InstanceType::Number => match format.as_deref() {
            Some("float") => "float32".to_string(),
            Some("double") | None => "float64".to_string(),
            Some(_) => "float64".to_string(),
        },
        InstanceType::Integer => match format.as_deref() {
            Some("int8") => "int8".to_string(),
            Some("int16") => "int16".to_string(),
            Some("int32") => "int32".to_string(),
            Some("int64") => "int64".to_string(),
            Some("uint") => "uint32".to_string(),
            Some("uint8") => "uint8".to_string(),
            Some("uint16") => "uint16".to_string(),
            Some("uint32") => "uint32".to_string(),
            Some("uint64") => "uint64".to_string(),
            None => "integer".to_string(),
            Some(_) => "integer".to_string(),
        },
        InstanceType::Array => {
            if let Some(arr) = array {
                if let Some(SingleOrVec::Single(item_schema)) = &arr.items {
                    let inner = schemars_to_typespec(item_schema);
                    // Parenthesize union types so that
                    // `(int64 | null)[]` isn't mis-parsed as
                    // `int64 | Array<null>`.
                    if inner.contains(" | ") {
                        return format!("({})[]", inner);
                    }
                    return format!("{}[]", inner);
                }
            }
            "unknown[]".to_string()
        }
        InstanceType::Object => {
            if let Some(obj_val) = object {
                if let Some(ap) = &obj_val.additional_properties {
                    // Map type.
                    let value_type = schemars_to_typespec(ap);
                    return format!("Record<{}>", value_type);
                }
                // Inline object with named properties.
                if !obj_val.properties.is_empty() {
                    let required: std::collections::HashSet<&str> =
                        obj_val.required.iter().map(|s| s.as_str()).collect();
                    let fields: Vec<String> = obj_val
                        .properties
                        .iter()
                        .map(|(name, schema)| {
                            let ts_type = schemars_to_typespec(schema);
                            let optional = if required.contains(name.as_str()) {
                                ""
                            } else {
                                "?"
                            };
                            format!(
                                "{}{}: {}",
                                escape_property_name(name),
                                optional,
                                ts_type,
                            )
                        })
                        .collect();
                    return format!("{{ {} }}", fields.join("; "));
                }
            }
            "Record<unknown>".to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// ResultsPage detection
// ---------------------------------------------------------------------------

/// Detect definitions that are ResultsPage instantiations.
///
/// Returns a map from concrete name (e.g. "WidgetResultsPage") to the item
/// type name (e.g. "Widget"). Detection is heuristic: the name must end in
/// "ResultsPage" and the schema must have `items` (array) and `next_page`
/// (nullable string) properties.
/// Returns true if a schema uses format: "uuid" anywhere within it.
/// We need to recurse because uuid can appear inside inline property schemas
/// or anyOf/oneOf variants, not just at the top level. Cross-type references
/// use $ref and become separate definitions, so this doesn't recurse across
/// named types.
fn schema_uses_uuid_format(schema: &Schema) -> bool {
    match schema {
        Schema::Bool(_) => false,
        Schema::Object(obj) => {
            if obj.format.as_deref() == Some("uuid") {
                return true;
            }
            if let Some(object) = &obj.object {
                for prop in object.properties.values() {
                    if schema_uses_uuid_format(prop) {
                        return true;
                    }
                }
            }
            if let Some(subschemas) = &obj.subschemas {
                let lists = [
                    &subschemas.one_of,
                    &subschemas.any_of,
                    &subschemas.all_of,
                ];
                for list in lists.into_iter().flatten() {
                    for s in list {
                        if schema_uses_uuid_format(s) {
                            return true;
                        }
                    }
                }
            }
            if let Some(arr) = &obj.array {
                if let Some(SingleOrVec::Single(item)) = &arr.items {
                    if schema_uses_uuid_format(item) {
                        return true;
                    }
                }
            }
            false
        }
    }
}

fn detect_results_pages(
    definitions: &IndexMap<String, Schema>,
) -> std::collections::HashMap<String, String> {
    let mut results = std::collections::HashMap::new();

    for (name, schema) in definitions {
        if !name.ends_with("ResultsPage") {
            continue;
        }

        // Verify the schema shape and extract the item type from the
        // `items` array property's $ref.
        let item_type = match schema {
            Schema::Object(SchemaObject { object: Some(obj), .. })
                if obj.properties.contains_key("items")
                    && obj.properties.contains_key("next_page") =>
            {
                extract_results_page_item_type(
                    obj.properties.get("items").unwrap(),
                )
            }
            _ => None,
        };

        if let Some(item_type) = item_type {
            results.insert(name.clone(), item_type);
        }
    }

    results
}

/// Extract the item type name from a ResultsPage `items` property schema.
/// The schema should be `{type: "array", items: {$ref: "..."}}`; we pull
/// the type name from the $ref.
fn extract_results_page_item_type(items_schema: &Schema) -> Option<String> {
    let obj = match items_schema {
        Schema::Object(obj) => obj,
        _ => return None,
    };
    let arr = obj.array.as_ref()?;
    let item = match &arr.items {
        Some(SingleOrVec::Single(item)) => item.as_ref(),
        _ => return None,
    };
    match item {
        Schema::Object(item_obj) => {
            item_obj.reference.as_ref().map(|r| ref_to_type_name(r))
        }
        _ => None,
    }
}

/// Scan definitions for string enum types.
/// Returns a map from enum name to its member values.
fn detect_enum_types(
    definitions: &IndexMap<String, Schema>,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut enums = std::collections::HashMap::new();
    for (name, schema) in definitions {
        if let Schema::Object(obj) = schema {
            if let Some(values) = &obj.enum_values {
                let members: Vec<String> = values
                    .iter()
                    .filter_map(|v| match v {
                        serde_json::Value::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                if !members.is_empty() {
                    enums.insert(name.clone(), members);
                }
            }
            // Also detect described string enums (oneOf of single-value
            // string enums).
            if let Some(sub) = &obj.subschemas {
                if let Some(one_of) = &sub.one_of {
                    if let Some(members) = detect_described_string_enum(one_of)
                    {
                        let vals =
                            members.iter().map(|m| m.value.clone()).collect();
                        enums.insert(name.clone(), vals);
                    }
                }
            }
        }
    }
    enums
}

/// Scan definitions for discriminated union types.
fn detect_discriminated_union_types(
    definitions: &IndexMap<String, Schema>,
) -> std::collections::HashSet<String> {
    let mut unions = std::collections::HashSet::new();
    for (name, schema) in definitions {
        if let Schema::Object(obj) = schema {
            if let Some(sub) = &obj.subschemas {
                if let Some(one_of) = &sub.one_of {
                    if detect_discriminator(one_of).is_some() {
                        unions.insert(name.clone());
                    }
                }
            }
        }
    }
    unions
}

// ---------------------------------------------------------------------------
// Described string enum detection
// ---------------------------------------------------------------------------

struct DescribedEnumMember {
    value: String,
    description: Option<String>,
}

/// Detect a oneOf where every variant is a single-value string enum,
/// optionally with a description. This is schemars' encoding of a Rust
/// enum where each variant has a doc comment.
fn detect_described_string_enum(
    variants: &[Schema],
) -> Option<Vec<DescribedEnumMember>> {
    let mut members = Vec::new();
    for variant in variants {
        match variant {
            Schema::Object(SchemaObject {
                metadata,
                instance_type: Some(SingleOrVec::Single(instance_type)),
                enum_values: Some(values),
                ..
            }) if **instance_type == InstanceType::String
                && !values.is_empty() =>
            {
                let description =
                    metadata.as_ref().and_then(|m| m.description.clone());
                for val in values {
                    let value = match val {
                        serde_json::Value::String(s) => s.clone(),
                        _ => return None,
                    };
                    // Only attach description to single-value variants;
                    // multi-value variants are bulk-encoded with no
                    // per-value descriptions.
                    let member_desc = if values.len() == 1 {
                        description.clone()
                    } else {
                        None
                    };
                    members.push(DescribedEnumMember {
                        value,
                        description: member_desc,
                    });
                }
            }
            _ => return None,
        }
    }
    if members.is_empty() {
        None
    } else {
        Some(members)
    }
}

// ---------------------------------------------------------------------------
// Discriminated union detection
// ---------------------------------------------------------------------------

struct DiscriminatedUnion {
    tag: String,
    variants: Vec<DiscriminatedVariant>,
}

struct DiscriminatedVariant {
    tag_value: String,
    model_name: String,
    description: Option<String>,
    properties: Vec<VariantProperty>,
}

struct VariantProperty {
    name: String,
    ts_type: String,
    required: bool,
    description: Option<String>,
    schema: Schema,
}

/// Detect whether a oneOf schema represents a discriminated union.
///
/// Returns `Some` if every variant is an object with a common property
/// whose schema is a single-value string enum (the discriminator tag).
fn detect_discriminator(variants: &[Schema]) -> Option<DiscriminatedUnion> {
    if variants.is_empty() {
        return None;
    }

    // Find candidate tag properties: present in every variant as a
    // single-value string enum.
    let mut candidate_tag: Option<String> = None;

    for (i, variant) in variants.iter().enumerate() {
        let obj = match variant {
            Schema::Object(obj) => obj,
            _ => return None,
        };
        let object = obj.object.as_ref()?;

        // Find properties that are single-value string enums.
        let tag_props: Vec<&String> = object
            .properties
            .iter()
            .filter(|(_, schema)| is_single_value_string_enum(schema))
            .map(|(name, _)| name)
            .collect();

        if i == 0 {
            // First variant: any single-value-enum property is a candidate.
            if tag_props.is_empty() {
                return None;
            }
            candidate_tag = Some(tag_props[0].clone());
        } else {
            // Subsequent variants must have the same tag property.
            let tag = candidate_tag.as_ref()?;
            if !tag_props.contains(&tag) {
                return None;
            }
        }
    }

    let tag = candidate_tag?;
    let mut result_variants = Vec::new();

    for variant in variants {
        let obj = match variant {
            Schema::Object(obj) => obj,
            _ => return None,
        };
        let object = obj.object.as_ref()?;
        let required: std::collections::HashSet<&str> =
            object.required.iter().map(|s| s.as_str()).collect();

        // Extract the tag value.
        let tag_schema = object.properties.get(&tag)?;
        let tag_value = extract_single_enum_value(tag_schema)?;

        // Collect non-tag properties.
        let properties = object
            .properties
            .iter()
            .filter(|(name, _)| *name != &tag)
            .map(|(name, schema)| VariantProperty {
                name: name.clone(),
                ts_type: schemars_to_typespec(schema),
                required: required.contains(name.as_str()),
                description: schema_description(schema),
                schema: schema.clone(),
            })
            .collect();

        let description =
            obj.metadata.as_deref().and_then(|m| m.description.clone());

        result_variants.push(DiscriminatedVariant {
            tag_value,
            model_name: String::new(), // filled in below
            description,
            properties,
        });
    }

    // Variant model names are not yet known — the parent name is needed
    // to qualify them. Leave model_name empty; the caller will fill it in.

    Some(DiscriminatedUnion { tag, variants: result_variants })
}

// ---------------------------------------------------------------------------
// Externally-tagged union detection
// ---------------------------------------------------------------------------

struct ExternallyTaggedUnion {
    variants: Vec<ExternallyTaggedVariant>,
}

struct ExternallyTaggedVariant {
    key: String,
    model_name: String,
    value_type: String,
}

/// Detect whether a oneOf schema represents an externally-tagged union
/// (Rust's default enum encoding). Each variant is an object with exactly
/// one required property and `additionalProperties: false`.
fn detect_externally_tagged_union(
    variants: &[Schema],
) -> Option<ExternallyTaggedUnion> {
    if variants.is_empty() {
        return None;
    }

    let mut result_variants = Vec::new();

    for variant in variants {
        let obj = match variant {
            Schema::Object(obj) => obj,
            _ => return None,
        };
        let object = obj.object.as_ref()?;

        // Must have exactly one property that is also required.
        if object.properties.len() != 1 || object.required.len() != 1 {
            return None;
        }

        // additionalProperties must be false (or absent).
        if let Some(ap) = &object.additional_properties {
            if !matches!(ap.as_ref(), Schema::Bool(false)) {
                return None;
            }
        }

        let (key, schema) = object.properties.iter().next().unwrap();
        if !object.required.contains(key) {
            return None;
        }

        result_variants.push(ExternallyTaggedVariant {
            key: key.clone(),
            model_name: String::new(), // filled by caller
            value_type: schemars_to_typespec(schema),
        });
    }

    Some(ExternallyTaggedUnion { variants: result_variants })
}

fn is_single_value_string_enum(schema: &Schema) -> bool {
    extract_single_enum_value(schema).is_some()
}

fn extract_single_enum_value(schema: &Schema) -> Option<String> {
    match schema {
        Schema::Object(SchemaObject { enum_values: Some(values), .. })
            if values.len() == 1 =>
        {
            match &values[0] {
                serde_json::Value::String(s) => Some(s.clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Emit TypeSpec validation decorators for a schema's constraints.
fn emit_validation_decorators(out: &mut String, schema: &Schema, indent: &str) {
    let obj = match schema {
        Schema::Object(obj) => obj,
        _ => return,
    };

    // String format hints (uuid, ip, etc.) that aren't captured by the
    // type mapping. Integer/float formats (int32, uint64, float, double)
    // and date-time/uri are already reflected in the TypeSpec type.
    // Only emit @format for string types — TypeSpec rejects it on others.
    let is_string_type = matches!(
        &obj.instance_type,
        Some(SingleOrVec::Single(t)) if **t == InstanceType::String
    );
    if is_string_type {
        if let Some(fmt) = &obj.format {
            match fmt.as_str() {
                "date-time" | "date" | "uri" | "uuid" => {}
                f => {
                    writeln!(out, "{}@format(\"{}\")", indent, f).unwrap();
                }
            }
        }
    }

    // Number validation: @minValue, @maxValue.
    if let Some(number) = &obj.number {
        if let Some(min) = number.minimum {
            writeln!(out, "{}@minValue({})", indent, format_f64(min)).unwrap();
        }
        if let Some(max) = number.maximum {
            writeln!(out, "{}@maxValue({})", indent, format_f64(max)).unwrap();
        }
        if let Some(min) = number.exclusive_minimum {
            writeln!(out, "{}@minValueExclusive({})", indent, format_f64(min))
                .unwrap();
        }
        if let Some(max) = number.exclusive_maximum {
            writeln!(out, "{}@maxValueExclusive({})", indent, format_f64(max))
                .unwrap();
        }
    }

    // String validation: @minLength, @maxLength, @pattern.
    if let Some(string) = &obj.string {
        if let Some(min) = string.min_length {
            writeln!(out, "{}@minLength({})", indent, min).unwrap();
        }
        if let Some(max) = string.max_length {
            writeln!(out, "{}@maxLength({})", indent, max).unwrap();
        }
        if let Some(pat) = &string.pattern {
            writeln!(out, "{}@pattern(\"{}\")", indent, escape_tsp_string(pat))
                .unwrap();
        }
    }

    // Array validation: @minItems, @maxItems.
    if let Some(array) = &obj.array {
        if let Some(min) = array.min_items {
            writeln!(out, "{}@minItems({})", indent, min).unwrap();
        }
        if let Some(max) = array.max_items {
            writeln!(out, "{}@maxItems({})", indent, max).unwrap();
        }
    }
}

/// Format an f64 as an integer string if it has no fractional part.
fn format_f64(v: f64) -> String {
    if v.fract() == 0.0 && v.is_finite() {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

fn ref_to_type_name(reference: &str) -> String {
    // schemars refs look like "#/definitions/TypeName"
    const DEFS_PREFIX: &str = "#/definitions/";
    if let Some(name) = reference.strip_prefix(DEFS_PREFIX) {
        return name.to_string();
    }
    const COMPONENTS_PREFIX: &str = "#/components/schemas/";
    if let Some(name) = reference.strip_prefix(COMPONENTS_PREFIX) {
        return name.to_string();
    }
    reference.to_string()
}

fn is_ref_schema(schema: &Schema) -> bool {
    matches!(schema, Schema::Object(SchemaObject { reference: Some(_), .. }))
}

fn is_empty_schema(schema: &Schema) -> bool {
    match schema {
        Schema::Bool(false) => true,
        Schema::Object(SchemaObject {
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
            reference: None,
            extensions: _,
        }) => true,
        _ => false,
    }
}

/// Extract a default value from a schema and convert to TypeSpec literal.
fn schema_default(schema: &Schema) -> Option<String> {
    match schema {
        Schema::Object(obj) => {
            let val = obj.metadata.as_ref()?.default.as_ref()?;
            Some(json_to_tsp_literal(val))
        }
        _ => None,
    }
}

/// Convert a serde_json::Value to a TypeSpec value literal.
fn json_to_tsp_literal(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            format!("\"{}\"", escape_tsp_string(s))
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                "#[]".to_string()
            } else {
                let items: Vec<String> =
                    arr.iter().map(json_to_tsp_literal).collect();
                format!("#[{}]", items.join(", "))
            }
        }
        serde_json::Value::Object(obj) => {
            if obj.is_empty() {
                "#{}".to_string()
            } else {
                let fields: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, json_to_tsp_literal(v)))
                    .collect();
                format!("#{{{}}}", fields.join(", "))
            }
        }
    }
}

fn schema_description(schema: &Schema) -> Option<String> {
    match schema {
        Schema::Object(obj) => {
            obj.metadata.as_ref().and_then(|m| m.description.clone())
        }
        _ => None,
    }
}

fn escape_tsp_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn escape_property_name(name: &str) -> String {
    if is_valid_tsp_ident(name) {
        name.to_string()
    } else {
        format!("`{}`", name)
    }
}

/// TypeSpec reserved keywords that cannot be used as bare identifiers.
const TYPESPEC_KEYWORDS: &[&str] = &[
    "alias",
    "const",
    "dec",
    "else",
    "enum",
    "extends",
    "extern",
    "false",
    "fn",
    "if",
    "import",
    "init",
    "interface",
    "is",
    "model",
    "namespace",
    "never",
    "null",
    "op",
    "projection",
    "return",
    "scalar",
    "true",
    "typeof",
    "union",
    "unknown",
    "using",
    "valueof",
    "void",
];

fn is_valid_tsp_ident(s: &str) -> bool {
    if TYPESPEC_KEYWORDS.contains(&s) {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn to_pascal_case(s: &str) -> String {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            let first = chars.next().unwrap().to_ascii_uppercase();
            let rest: String = chars.collect();
            format!("{}{}", first, rest)
        })
        .collect::<String>()
}

fn to_namespace_id(title: &str) -> String {
    to_pascal_case(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_namespace_id() {
        assert_eq!(to_namespace_id("my cool api"), "MyCoolApi");
        assert_eq!(to_namespace_id("test"), "Test");
        assert_eq!(to_namespace_id("hello-world"), "HelloWorld");
    }

    #[test]
    fn test_escape_tsp_string() {
        assert_eq!(escape_tsp_string("hello"), "hello");
        assert_eq!(escape_tsp_string("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(escape_tsp_string("line1\nline2"), "line1\\nline2");
    }

    #[test]
    fn test_is_valid_tsp_ident() {
        assert!(is_valid_tsp_ident("hello"));
        assert!(is_valid_tsp_ident("_foo"));
        assert!(!is_valid_tsp_ident("123"));
        assert!(!is_valid_tsp_ident("foo-bar"));
    }

    // -- Helper functions for constructing test schemas --

    fn string_schema() -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::String,
            ))),
            ..Default::default()
        })
    }

    fn string_schema_with_format(fmt: &str) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::String,
            ))),
            format: Some(fmt.to_string()),
            ..Default::default()
        })
    }

    fn uint32_schema() -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::Integer,
            ))),
            format: Some("uint32".to_string()),
            number: Some(Box::new(schemars::schema::NumberValidation {
                minimum: Some(0.0),
                ..Default::default()
            })),
            ..Default::default()
        })
    }

    fn ref_schema(name: &str) -> Schema {
        Schema::Object(SchemaObject {
            reference: Some(format!("#/definitions/{}", name)),
            ..Default::default()
        })
    }

    fn single_enum_schema(value: &str) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::String,
            ))),
            enum_values: Some(vec![serde_json::Value::String(
                value.to_string(),
            )]),
            ..Default::default()
        })
    }

    fn results_page_schema(item_ref: &str) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::Object,
            ))),
            object: Some(Box::new(schemars::schema::ObjectValidation {
                properties: {
                    let mut props = schemars::Map::new();
                    props.insert(
                        "items".to_string(),
                        Schema::Object(SchemaObject {
                            instance_type: Some(SingleOrVec::Single(Box::new(
                                InstanceType::Array,
                            ))),
                            array: Some(Box::new(
                                schemars::schema::ArrayValidation {
                                    items: Some(SingleOrVec::Single(Box::new(
                                        ref_schema(item_ref),
                                    ))),
                                    ..Default::default()
                                },
                            )),
                            ..Default::default()
                        }),
                    );
                    props.insert(
                        "next_page".to_string(),
                        Schema::Object(SchemaObject {
                            instance_type: Some(SingleOrVec::Single(Box::new(
                                InstanceType::String,
                            ))),
                            extensions: {
                                let mut m = schemars::Map::new();
                                m.insert(
                                    "nullable".to_string(),
                                    serde_json::Value::Bool(true),
                                );
                                m
                            },
                            ..Default::default()
                        }),
                    );
                    props
                },
                required: {
                    let mut req = std::collections::BTreeSet::new();
                    req.insert("items".to_string());
                    req
                },
                ..Default::default()
            })),
            ..Default::default()
        })
    }

    // -- Bug 1: ResultsPage name collisions --

    #[test]
    fn test_results_page_item_type_from_ref() {
        // detect_results_pages must extract the item type from the $ref
        // in the items array schema, not by parsing the definition name.
        let mut defs = IndexMap::new();
        defs.insert(
            "Disk".to_string(),
            string_schema(), // placeholder
        );
        defs.insert(
            "PhysicalDisk".to_string(),
            string_schema(), // placeholder
        );
        defs.insert("DiskResultsPage".to_string(), results_page_schema("Disk"));
        defs.insert(
            "PhysicalDiskResultsPage".to_string(),
            results_page_schema("PhysicalDisk"),
        );

        let pages = detect_results_pages(&defs);
        assert_eq!(pages.get("DiskResultsPage").unwrap(), "Disk");
        assert_eq!(
            pages.get("PhysicalDiskResultsPage").unwrap(),
            "PhysicalDisk",
        );
    }

    // -- Bug 2: Externally-tagged unions rendered as Record<never> --

    #[test]
    fn test_externally_tagged_union() {
        // An externally-tagged enum like Rust's default enum encoding:
        //   { "oneOf": [
        //     { "type": "object", "properties": { "unknown": uint32 },
        //       "required": ["unknown"], "additionalProperties": false },
        //     ...
        //   ] }
        // Must NOT produce Record<never> for each variant.
        let variants: Vec<Schema> = vec![
            ("unknown", uint32_schema()),
            ("if_index", uint32_schema()),
            ("port_number", uint32_schema()),
        ]
        .into_iter()
        .map(|(key, val)| {
            Schema::Object(SchemaObject {
                instance_type: Some(SingleOrVec::Single(Box::new(
                    InstanceType::Object,
                ))),
                object: Some(Box::new(schemars::schema::ObjectValidation {
                    properties: {
                        let mut p = schemars::Map::new();
                        p.insert(key.to_string(), val);
                        p
                    },
                    required: {
                        let mut r = std::collections::BTreeSet::new();
                        r.insert(key.to_string());
                        r
                    },
                    additional_properties: Some(Box::new(Schema::Bool(false))),
                    ..Default::default()
                })),
                ..Default::default()
            })
        })
        .collect();

        let obj = SchemaObject {
            subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
                one_of: Some(variants),
                ..Default::default()
            })),
            ..Default::default()
        };

        let mut ctx = TypeSpecContext::new();
        ctx.emit_model_from_object("InterfaceNum", &obj);
        let output = ctx.out;

        // Must contain named variant models, not Record<never>.
        assert!(
            !output.contains("Record<never>"),
            "externally-tagged union produced Record<never>:\n{}",
            output,
        );
        assert!(
            output.contains("`unknown`: uint32"),
            "missing variant property '`unknown`: uint32':\n{}",
            output,
        );
        assert!(
            output.contains("if_index: uint32"),
            "missing variant property 'if_index: uint32':\n{}",
            output,
        );
        assert!(
            output.contains("union InterfaceNum"),
            "missing union declaration:\n{}",
            output,
        );
    }

    // -- Bug 3: Nullable array items operator precedence --

    #[test]
    fn test_nullable_array_items_precedence() {
        // { "type": "array", "items": { "nullable": true, "type":
        //   "integer", "format": "int64" } }
        // Must produce "(int64 | null)[]", NOT "int64 | null[]".
        let schema = Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::Array,
            ))),
            array: Some(Box::new(schemars::schema::ArrayValidation {
                items: Some(SingleOrVec::Single(Box::new(Schema::Object(
                    SchemaObject {
                        instance_type: Some(SingleOrVec::Single(Box::new(
                            InstanceType::Integer,
                        ))),
                        format: Some("int64".to_string()),
                        extensions: {
                            let mut m = schemars::Map::new();
                            m.insert(
                                "nullable".to_string(),
                                serde_json::Value::Bool(true),
                            );
                            m
                        },
                        ..Default::default()
                    },
                )))),
                ..Default::default()
            })),
            ..Default::default()
        });

        let ts = schemars_to_typespec(&schema);
        assert_eq!(ts, "(int64 | null)[]");
    }

    // -- Bug 4: Missing @format("ip") on discriminated union variant
    //    fields --

    #[test]
    fn test_discriminated_variant_format_and_doc() {
        // A discriminated union variant with a property that has
        // format: "ip" and a description. Both must appear in output.
        let variants = vec![Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::Object,
            ))),
            object: Some(Box::new(schemars::schema::ObjectValidation {
                properties: {
                    let mut p = schemars::Map::new();
                    p.insert("kind".to_string(), single_enum_schema("snat"));
                    p.insert(
                        "ip".to_string(),
                        Schema::Object(SchemaObject {
                            instance_type: Some(SingleOrVec::Single(Box::new(
                                InstanceType::String,
                            ))),
                            format: Some("ip".to_string()),
                            metadata: Some(Box::new(
                                schemars::schema::Metadata {
                                    description: Some(
                                        "The IP address.".to_string(),
                                    ),
                                    ..Default::default()
                                },
                            )),
                            ..Default::default()
                        }),
                    );
                    p
                },
                required: {
                    let mut r = std::collections::BTreeSet::new();
                    r.insert("kind".to_string());
                    r.insert("ip".to_string());
                    r
                },
                ..Default::default()
            })),
            ..Default::default()
        })];

        let obj = SchemaObject {
            subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
                one_of: Some(variants),
                ..Default::default()
            })),
            ..Default::default()
        };

        let mut ctx = TypeSpecContext::new();
        ctx.emit_model_from_object("ExternalIp", &obj);
        let output = ctx.out;

        assert!(
            output.contains("@format(\"ip\")"),
            "missing @format(\"ip\") on variant property:\n{}",
            output,
        );
        assert!(
            output.contains("@doc(\"The IP address.\")"),
            "missing @doc on variant property:\n{}",
            output,
        );
    }

    // -- Bug 5: Multi-value described string enum --

    #[test]
    fn test_described_string_enum_multi_value() {
        // oneOf where the first variant has multiple enum values
        // (undocumented) and later variants have single values with
        // descriptions. All values must be extracted individually.
        let variants = vec![
            // Multi-value variant, no description
            Schema::Object(SchemaObject {
                instance_type: Some(SingleOrVec::Single(Box::new(
                    InstanceType::String,
                ))),
                enum_values: Some(vec![
                    serde_json::Value::String("count".to_string()),
                    serde_json::Value::String("bytes".to_string()),
                    serde_json::Value::String("seconds".to_string()),
                ]),
                ..Default::default()
            }),
            // Single-value variant with description
            Schema::Object(SchemaObject {
                instance_type: Some(SingleOrVec::Single(Box::new(
                    InstanceType::String,
                ))),
                enum_values: Some(vec![serde_json::Value::String(
                    "none".to_string(),
                )]),
                metadata: Some(Box::new(schemars::schema::Metadata {
                    description: Some("No meaningful units.".to_string()),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            // Single-value variant with description
            Schema::Object(SchemaObject {
                instance_type: Some(SingleOrVec::Single(Box::new(
                    InstanceType::String,
                ))),
                enum_values: Some(vec![serde_json::Value::String(
                    "rpm".to_string(),
                )]),
                metadata: Some(Box::new(schemars::schema::Metadata {
                    description: Some("Rotations per minute.".to_string()),
                    ..Default::default()
                })),
                ..Default::default()
            }),
        ];

        let members = detect_described_string_enum(&variants);
        let members = members.expect(
            "should detect as described string enum with multi-value \
             variants",
        );
        assert_eq!(members.len(), 5);
        assert_eq!(members[0].value, "count");
        assert!(members[0].description.is_none());
        assert_eq!(members[1].value, "bytes");
        assert!(members[1].description.is_none());
        assert_eq!(members[2].value, "seconds");
        assert!(members[2].description.is_none());
        assert_eq!(members[3].value, "none");
        assert_eq!(
            members[3].description.as_deref(),
            Some("No meaningful units."),
        );
        assert_eq!(members[4].value, "rpm");
        assert_eq!(
            members[4].description.as_deref(),
            Some("Rotations per minute."),
        );
    }

    // -- Bug 6: Missing default values on discriminated union variant
    //    properties --

    #[test]
    fn test_discriminated_variant_default_values() {
        // A discriminated union variant with an optional property that
        // has a default value. The default must appear in the output.
        let variants = vec![Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::Object,
            ))),
            metadata: Some(Box::new(schemars::schema::Metadata {
                description: Some("Create a disk from a snapshot".to_string()),
                ..Default::default()
            })),
            object: Some(Box::new(schemars::schema::ObjectValidation {
                properties: {
                    let mut p = schemars::Map::new();
                    p.insert(
                        "type".to_string(),
                        single_enum_schema("snapshot"),
                    );
                    p.insert(
                        "read_only".to_string(),
                        Schema::Object(SchemaObject {
                            instance_type: Some(SingleOrVec::Single(Box::new(
                                InstanceType::Boolean,
                            ))),
                            metadata: Some(Box::new(
                                schemars::schema::Metadata {
                                    description: Some(
                                        "If true, read-only.".to_string(),
                                    ),
                                    default: Some(serde_json::Value::Bool(
                                        false,
                                    )),
                                    ..Default::default()
                                },
                            )),
                            ..Default::default()
                        }),
                    );
                    p.insert(
                        "snapshot_id".to_string(),
                        string_schema_with_format("uuid"),
                    );
                    p
                },
                required: {
                    let mut r = std::collections::BTreeSet::new();
                    r.insert("type".to_string());
                    r.insert("snapshot_id".to_string());
                    r
                },
                ..Default::default()
            })),
            ..Default::default()
        })];

        let obj = SchemaObject {
            subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
                one_of: Some(variants),
                ..Default::default()
            })),
            ..Default::default()
        };

        let mut ctx = TypeSpecContext::new();
        ctx.emit_model_from_object("DiskSource", &obj);
        let output = ctx.out;

        assert!(
            output.contains("read_only?: boolean = false"),
            "missing default value on variant property:\n{}",
            output,
        );
        assert!(
            output.contains("@doc(\"If true, read-only.\")"),
            "missing @doc on variant property:\n{}",
            output,
        );
    }

    // -- Bug 7: Union member titles lost --

    #[test]
    fn test_union_member_titles() {
        // An untagged union where each oneOf member has a title.
        // The title must become a named union member.
        let variants = vec![
            Schema::Object(SchemaObject {
                metadata: Some(Box::new(schemars::schema::Metadata {
                    title: Some("v4".to_string()),
                    ..Default::default()
                })),
                subschemas: Some(Box::new(
                    schemars::schema::SubschemaValidation {
                        all_of: Some(vec![ref_schema("Ipv4Range")]),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            }),
            Schema::Object(SchemaObject {
                metadata: Some(Box::new(schemars::schema::Metadata {
                    title: Some("v6".to_string()),
                    ..Default::default()
                })),
                subschemas: Some(Box::new(
                    schemars::schema::SubschemaValidation {
                        all_of: Some(vec![ref_schema("Ipv6Range")]),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            }),
        ];

        let obj = SchemaObject {
            subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
                one_of: Some(variants),
                ..Default::default()
            })),
            ..Default::default()
        };

        let mut ctx = TypeSpecContext::new();
        ctx.emit_model_from_object("IpRange", &obj);
        let output = ctx.out;

        assert!(
            output.contains("v4: Ipv4Range"),
            "missing named union member 'v4: Ipv4Range':\n{}",
            output,
        );
        assert!(
            output.contains("v6: Ipv6Range"),
            "missing named union member 'v6: Ipv6Range':\n{}",
            output,
        );
    }

    // -- TypeSpec keyword escaping --

    #[test]
    fn test_keyword_property_names_escaped() {
        // Property names that are TypeSpec keywords must be
        // backtick-escaped.
        let schema = SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::Object,
            ))),
            object: Some(Box::new(schemars::schema::ObjectValidation {
                properties: {
                    let mut p = schemars::Map::new();
                    p.insert("model".to_string(), string_schema());
                    p.insert("interface".to_string(), string_schema());
                    p.insert("unknown".to_string(), uint32_schema());
                    p.insert("name".to_string(), string_schema());
                    p
                },
                required: {
                    let mut r = std::collections::BTreeSet::new();
                    r.insert("model".to_string());
                    r.insert("interface".to_string());
                    r.insert("unknown".to_string());
                    r.insert("name".to_string());
                    r
                },
                ..Default::default()
            })),
            ..Default::default()
        };

        let mut ctx = TypeSpecContext::new();
        ctx.emit_model_from_object("PhysicalDisk", &schema);
        let output = ctx.out;

        assert!(
            output.contains("`model`: string"),
            "'model' must be backtick-escaped:\n{}",
            output,
        );
        assert!(
            output.contains("`interface`: string"),
            "'interface' must be backtick-escaped:\n{}",
            output,
        );
        assert!(
            output.contains("`unknown`: uint32"),
            "'unknown' must be backtick-escaped:\n{}",
            output,
        );
        // "name" is not a keyword, should NOT be escaped.
        assert!(
            output.contains("  name: string"),
            "'name' should not be escaped:\n{}",
            output,
        );
    }

    #[test]
    fn test_keyword_enum_members_escaped() {
        // Enum members that are TypeSpec keywords must be
        // backtick-escaped.
        let schema = SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::String,
            ))),
            enum_values: Some(vec![
                serde_json::Value::String("init".to_string()),
                serde_json::Value::String("never".to_string()),
                serde_json::Value::String("up".to_string()),
            ]),
            ..Default::default()
        };

        let mut ctx = TypeSpecContext::new();
        ctx.emit_model_from_object("BgpPeerState", &schema);
        let output = ctx.out;

        assert!(
            output.contains("`init`: \"init\""),
            "'init' must be backtick-escaped:\n{}",
            output,
        );
        assert!(
            output.contains("`never`: \"never\""),
            "'never' must be backtick-escaped:\n{}",
            output,
        );
        // "up" is not a keyword, should not be escaped.
        assert!(
            output.contains("\n  up"),
            "'up' should not be escaped:\n{}",
            output,
        );
    }

    // -- scalar extends unknown --

    #[test]
    fn test_unknown_type_not_scalar_extends() {
        // A schema with no type info should not produce
        // `scalar X extends unknown` since `unknown` is a keyword.
        let schema = SchemaObject::default();

        let mut ctx = TypeSpecContext::new();
        ctx.emit_model_from_object("BgpMessageHistory", &schema);
        let output = ctx.out;

        assert!(
            !output.contains("scalar"),
            "should not emit 'scalar ... extends unknown':\n{}",
            output,
        );
        assert!(
            output.contains("BgpMessageHistory"),
            "should still define the type:\n{}",
            output,
        );
    }

    // -- @format on non-string types --

    #[test]
    fn test_format_not_emitted_for_non_string() {
        // @format("uint") on an integer type should not be emitted
        // since TypeSpec's @format only applies to strings.
        let schema = Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::Integer,
            ))),
            format: Some("uint".to_string()),
            number: Some(Box::new(schemars::schema::NumberValidation {
                minimum: Some(0.0),
                ..Default::default()
            })),
            ..Default::default()
        });

        let mut out = String::new();
        emit_validation_decorators(&mut out, &schema, "");
        assert!(
            !out.contains("@format"),
            "@format should not be emitted for integers:\n{}",
            out,
        );
        // @minValue should still be emitted.
        assert!(
            out.contains("@minValue(0)"),
            "@minValue should still be emitted:\n{}",
            out,
        );
    }

    // -- Empty object literal --

    #[test]
    fn test_empty_object_literal() {
        let val = serde_json::Value::Object(serde_json::Map::new());
        assert_eq!(
            json_to_tsp_literal(&val),
            "#{}",
            "empty object must be '#{{}}' not '#{{}}{{}}'",
        );
    }

    // -- Enum defaults --

    #[test]
    fn test_enum_default_uses_member_syntax() {
        // A property with type IpVersion (an enum) and default "v4"
        // should emit `IpVersion.v4`, not `"v4"`.
        let schema = SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::Object,
            ))),
            object: Some(Box::new(schemars::schema::ObjectValidation {
                properties: {
                    let mut p = schemars::Map::new();
                    p.insert(
                        "ip_version".to_string(),
                        Schema::Object(SchemaObject {
                            reference: Some(
                                "#/definitions/IpVersion".to_string(),
                            ),
                            metadata: Some(Box::new(
                                schemars::schema::Metadata {
                                    default: Some(serde_json::Value::String(
                                        "v4".to_string(),
                                    )),
                                    ..Default::default()
                                },
                            )),
                            ..Default::default()
                        }),
                    );
                    p
                },
                ..Default::default()
            })),
            ..Default::default()
        };

        let mut ctx = TypeSpecContext::new();
        ctx.known_enums.insert(
            "IpVersion".to_string(),
            vec!["v4".to_string(), "v6".to_string()],
        );
        ctx.emit_model_from_object("IpPoolCreate", &schema);
        let output = ctx.out;

        assert!(
            output.contains("IpVersion.v4"),
            "enum default should use member syntax:\n{}",
            output,
        );
        assert!(
            !output.contains("\"v4\""),
            "enum default should not be a string literal:\n{}",
            output,
        );
    }

    // -- Complex object defaults on discriminated unions --

    // -- Bug: Inline nested objects in union variants → Record<unknown> --

    #[test]
    fn test_inline_object_with_properties_not_record_unknown() {
        // An inline object with named properties (no additional_properties)
        // must produce an inline model type, not Record<unknown>.
        let obj_val = schemars::schema::ObjectValidation {
            properties: {
                let mut p = schemars::Map::new();
                p.insert("id".to_string(), string_schema_with_format("uuid"));
                p.insert("name".to_string(), ref_schema("Name"));
                p
            },
            required: {
                let mut r = std::collections::BTreeSet::new();
                r.insert("id".to_string());
                r.insert("name".to_string());
                r
            },
            ..Default::default()
        };

        let result = instance_type_to_typespec(
            &InstanceType::Object,
            &None,
            &None,
            &Some(Box::new(obj_val)),
        );

        assert!(
            !result.contains("Record<unknown>"),
            "inline object with properties should not be Record<unknown>: {}",
            result,
        );
        assert!(
            result.contains("id") && result.contains("uuid"),
            "should contain 'id: uuid': {}",
            result,
        );
        assert!(
            result.contains("name") && result.contains("Name"),
            "should contain 'name: Name': {}",
            result,
        );
    }

    #[test]
    fn test_discriminated_variant_inline_object_property() {
        // A discriminated union variant with a property whose schema is
        // an inline object (properties, not a $ref). Must not produce
        // Record<unknown>.
        let variants = vec![Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::Object,
            ))),
            object: Some(Box::new(schemars::schema::ObjectValidation {
                properties: {
                    let mut p = schemars::Map::new();
                    p.insert(
                        "type".to_string(),
                        single_enum_schema("instance"),
                    );
                    p.insert(
                        "value".to_string(),
                        Schema::Object(SchemaObject {
                            instance_type: Some(SingleOrVec::Single(Box::new(
                                InstanceType::Object,
                            ))),
                            object: Some(Box::new(
                                schemars::schema::ObjectValidation {
                                    properties: {
                                        let mut ip = schemars::Map::new();
                                        ip.insert(
                                            "id".to_string(),
                                            string_schema_with_format("uuid"),
                                        );
                                        ip.insert(
                                            "name".to_string(),
                                            ref_schema("Name"),
                                        );
                                        ip.insert(
                                            "run_state".to_string(),
                                            ref_schema("InstanceState"),
                                        );
                                        ip
                                    },
                                    required: {
                                        let mut r =
                                            std::collections::BTreeSet::new();
                                        r.insert("id".to_string());
                                        r.insert("name".to_string());
                                        r.insert("run_state".to_string());
                                        r
                                    },
                                    ..Default::default()
                                },
                            )),
                            ..Default::default()
                        }),
                    );
                    p
                },
                required: {
                    let mut r = std::collections::BTreeSet::new();
                    r.insert("type".to_string());
                    r.insert("value".to_string());
                    r
                },
                ..Default::default()
            })),
            ..Default::default()
        })];

        let obj = SchemaObject {
            subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
                one_of: Some(variants),
                ..Default::default()
            })),
            ..Default::default()
        };

        let mut ctx = TypeSpecContext::new();
        ctx.emit_model_from_object("AffinityGroupMember", &obj);
        let output = ctx.out;

        assert!(
            !output.contains("Record<unknown>"),
            "inline object should not become Record<unknown>:\n{}",
            output,
        );
        assert!(
            output.contains("id") && output.contains("uuid"),
            "inline object should reference id/uuid:\n{}",
            output,
        );
        assert!(
            output.contains("run_state") && output.contains("InstanceState"),
            "inline object should reference run_state/InstanceState:\n{}",
            output,
        );
    }

    // -- Bug: Operation descriptions dropped --

    #[test]
    fn test_op_summary_and_description_concatenated() {
        let op = OpInfo {
            method: "post".to_string(),
            path: "/v1/floating-ips".to_string(),
            operation_id: "floating_ip_create".to_string(),
            summary: Some("Create floating IP".to_string()),
            description: Some(
                "A specific IP address can be reserved.".to_string(),
            ),
            tags: vec![],
            deprecated: false,
            params: vec![],
            body: None,
            response: ResponseInfo::default(),
            error_type: None,
            extension_mode: ExtensionMode::None,
        };

        let mut ctx = TypeSpecContext::new();
        ctx.emit_op(&op);
        let output = ctx.out;

        // Both summary and description should appear in a single @doc,
        // separated by \n\n.
        assert!(
            output.contains(
                r#"Create floating IP\n\nA specific IP address can be reserved."#
            ),
            "should concatenate summary and description:\n{}",
            output,
        );
        assert_eq!(
            output.matches("@doc(").count(),
            1,
            "should have exactly one @doc:\n{}",
            output,
        );
    }

    // -- Bug: Parameter descriptions dropped --

    #[test]
    fn test_op_param_descriptions_emitted() {
        let op = OpInfo {
            method: "delete".to_string(),
            path: "/v1/internet-gateways/{gateway}".to_string(),
            operation_id: "internet_gateway_delete".to_string(),
            summary: None,
            description: None,
            tags: vec![],
            deprecated: false,
            params: vec![
                ParamInfo {
                    location: ParamLocation::Path,
                    name: "gateway".to_string(),
                    ts_type: "NameOrId".to_string(),
                    required: true,
                    description: None,
                },
                ParamInfo {
                    location: ParamLocation::Query,
                    name: "cascade".to_string(),
                    ts_type: "boolean".to_string(),
                    required: false,
                    description: Some(
                        "Also delete routes targeting this gateway."
                            .to_string(),
                    ),
                },
                ParamInfo {
                    location: ParamLocation::Query,
                    name: "project".to_string(),
                    ts_type: "NameOrId".to_string(),
                    required: false,
                    description: None,
                },
            ],
            body: None,
            response: ResponseInfo::default(),
            error_type: None,
            extension_mode: ExtensionMode::None,
        };

        let mut ctx = TypeSpecContext::new();
        ctx.emit_op(&op);
        let output = ctx.out;

        assert!(
            output.contains(
                r#"@doc("Also delete routes targeting this gateway.")"#
            ),
            "parameter description should be emitted as @doc:\n{}",
            output,
        );
        // @doc should appear before the @query decorator for cascade.
        let doc_pos = output
            .find(r#"@doc("Also delete routes targeting this gateway.")"#)
            .unwrap();
        let query_cascade_pos = output.find("@query cascade").unwrap();
        assert!(
            doc_pos < query_cascade_pos,
            "@doc should appear before @query:\n{}",
            output,
        );
    }

    // -- Bug: format "uint" maps to integer instead of uint32 --

    #[test]
    fn test_uint_format_maps_to_uint32() {
        let result = instance_type_to_typespec(
            &InstanceType::Integer,
            &Some("uint".to_string()),
            &None,
            &None,
        );
        assert_eq!(result, "uint32", "format 'uint' should map to uint32",);
    }

    #[test]
    fn test_object_default_skipped_for_discriminated_union() {
        // A property whose type is a discriminated union should not
        // have an object default emitted, because the default
        // references variant-specific fields not present on the base.
        let schema = SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(
                InstanceType::Object,
            ))),
            object: Some(Box::new(schemars::schema::ObjectValidation {
                properties: {
                    let mut p = schemars::Map::new();
                    p.insert(
                        "pool_selector".to_string(),
                        Schema::Object(SchemaObject {
                            reference: Some(
                                "#/definitions/PoolSelector".to_string(),
                            ),
                            metadata: Some(Box::new(
                                schemars::schema::Metadata {
                                    default: Some(serde_json::json!({
                                        "ip_version": null,
                                        "type": "auto",
                                    })),
                                    ..Default::default()
                                },
                            )),
                            ..Default::default()
                        }),
                    );
                    p
                },
                ..Default::default()
            })),
            ..Default::default()
        };

        let mut ctx = TypeSpecContext::new();
        ctx.discriminated_unions.insert("PoolSelector".to_string());
        ctx.emit_model_from_object("SomeCreate", &schema);
        let output = ctx.out;

        assert!(
            !output.contains("= #"),
            "object default on discriminated union type should be \
             skipped:\n{}",
            output,
        );
    }
}
