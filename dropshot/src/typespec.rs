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
                    op.body = Some(BodyInfo {
                        ts_type,
                        content_type: ct.clone(),
                    });
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

    // Emit preamble.
    ctx.emit_preamble(title, version);

    // Emit the generic ResultsPage model if any instantiations were found.
    if !ctx.results_page_rewrites.is_empty() {
        writeln!(ctx.out, "model ResultsPage<T> {{").unwrap();
        writeln!(
            ctx.out,
            "  @doc(\"list of items on this page of results\")"
        )
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

    // Emit the FreeformResponse model if any operation needs it.
    // @defaultResponse tells TypeSpec/OpenAPI this is the catch-all response
    // (rendered as `default` in OpenAPI, matching dropshot's behavior).
    if ctx.needs_freeform_response {
        writeln!(
            ctx.out,
            "@doc(\"Response with untyped body\")"
        )
        .unwrap();
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
    for (concrete, item_type) in &ctx.results_page_rewrites {
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
}

impl TypeSpecContext {
    fn new() -> Self {
        Self {
            out: String::new(),
            results_page_rewrites: std::collections::HashMap::new(),
            error_types: std::collections::HashSet::new(),
            needs_freeform_response: false,
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
                writeln!(self.out, "  @statusCode statusCode: int32;")
                    .unwrap();
            }

            let required_set: std::collections::HashSet<&str> =
                object.required.iter().map(|s| s.as_str()).collect();

            for (prop_name, prop_schema) in &object.properties {
                let ts_type = schemars_to_typespec(prop_schema);
                let optional =
                    if required_set.contains(prop_name.as_str()) { "" } else { "?" };
                let desc = schema_description(prop_schema);
                if let Some(d) = &desc {
                    writeln!(self.out, "  @doc(\"{}\")", escape_tsp_string(d))
                        .unwrap();
                }
                emit_validation_decorators(&mut self.out, prop_schema, "  ");
                let default_suffix = schema_default(prop_schema)
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
        } else {
            emit_validation_decorators(
                &mut self.out,
                &Schema::Object(obj.clone()),
                "",
            );
            writeln!(self.out, "scalar {} extends {};", name, ts_type)
                .unwrap();
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
                    writeln!(self.out, "  value{}: {}{}", i, n, comma)
                        .unwrap();
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
                    writeln!(self.out, "  {}{}", member.value, comma)
                        .unwrap();
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
        if let Some(disc) = detect_discriminator(variants) {
            self.emit_doc_from_metadata(obj.metadata.as_deref());
            writeln!(self.out, "@discriminator(\"{}\")", disc.tag).unwrap();
            writeln!(self.out, "model {} {{", name).unwrap();
            writeln!(self.out, "  {}: string;", disc.tag).unwrap();
            writeln!(self.out, "}}").unwrap();
            writeln!(self.out).unwrap();
            // Emit variant models that extend the base.
            for variant in &disc.variants {
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
                    writeln!(
                        self.out,
                        "  {}{}: {};",
                        escape_property_name(&prop.name),
                        if prop.required { "" } else { "?" },
                        prop.ts_type,
                    )
                    .unwrap();
                }
                writeln!(self.out, "}}").unwrap();
                writeln!(self.out).unwrap();
            }
            return;
        }

        // Non-discriminated union: emit as TypeSpec union with inline types.
        self.emit_doc_from_metadata(obj.metadata.as_deref());
        let variant_types: Vec<String> =
            variants.iter().map(|s| schemars_to_typespec(s)).collect();
        writeln!(self.out, "union {} {{", name).unwrap();
        for vt in &variant_types {
            writeln!(self.out, "  {},", vt).unwrap();
        }
        writeln!(self.out, "}}").unwrap();
        writeln!(self.out).unwrap();
    }

    fn emit_op(&mut self, op: &OpInfo) {
        // Doc comment.
        if let Some(summary) = &op.summary {
            writeln!(self.out, "@doc(\"{}\")", escape_tsp_string(summary))
                .unwrap();
        }
        if let Some(desc) = &op.description {
            if op.summary.is_none() {
                writeln!(self.out, "@doc(\"{}\")", escape_tsp_string(desc))
                    .unwrap();
            }
        }

        // Tags.
        for tag in &op.tags {
            writeln!(self.out, "@tag(\"{}\")", escape_tsp_string(tag))
                .unwrap();
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

        let mut params_parts = Vec::new();

        // Path, query, header params.
        for p in &op.params {
            let decorator = match p.location {
                ParamLocation::Path => "@path",
                ParamLocation::Query => "@query",
                ParamLocation::Header => "@header",
            };
            let optional = if p.required { "" } else { "?" };
            params_parts.push(format!(
                "{} {}{}: {}",
                decorator, p.name, optional, p.ts_type
            ));
        }

        // Body param.
        if let Some(body) = &op.body {
            match body.content_type {
                ApiEndpointBodyContentType::Json => {
                    params_parts
                        .push(format!("@body body: {}", body.ts_type));
                }
                ApiEndpointBodyContentType::Bytes => {
                    params_parts.push(format!(
                        "@header contentType: \"application/octet-stream\", \
                         @body body: {}",
                        body.ts_type
                    ));
                }
                ApiEndpointBodyContentType::UrlEncoded => {
                    params_parts.push(format!(
                        "@header contentType: \
                         \"application/x-www-form-urlencoded\", \
                         @body body: {}",
                        body.ts_type
                    ));
                }
                ApiEndpointBodyContentType::MultipartFormData => {
                    params_parts.push(format!(
                        "@header contentType: \"multipart/form-data\", \
                         @body body: {}",
                        body.ts_type
                    ));
                }
            }
        }

        if params_parts.len() <= 2 {
            // Inline params.
            write!(self.out, "{}", params_parts.join(", ")).unwrap();
            write!(self.out, ")").unwrap();
        } else {
            // Multi-line params.
            writeln!(self.out).unwrap();
            for (i, part) in params_parts.iter().enumerate() {
                let comma =
                    if i + 1 < params_parts.len() { "," } else { "" };
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
    #[allow(dead_code)] // will be used for @doc on params in Phase 2
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
            let non_null: Vec<&InstanceType> = types
                .iter()
                .filter(|t| **t != InstanceType::Null)
                .collect();
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
        InstanceType::String => {
            match format.as_deref() {
                Some("date") => "plainDate".to_string(),
                Some("date-time") => "utcDateTime".to_string(),
                Some("uuid") => "string".to_string(), // no built-in uuid
                Some("ip") | Some("ipv4") | Some("ipv6") => {
                    "string".to_string()
                }
                Some("uri") => "url".to_string(),
                _ => "string".to_string(),
            }
        }
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
fn detect_results_pages(
    definitions: &IndexMap<String, Schema>,
) -> std::collections::HashMap<String, String> {
    let mut results = std::collections::HashMap::new();

    for (name, schema) in definitions {
        if !name.ends_with("ResultsPage") {
            continue;
        }
        let prefix = &name[..name.len() - "ResultsPage".len()];
        if prefix.is_empty() {
            continue;
        }

        // Verify the schema shape: object with `items` array and `next_page`.
        let has_expected_shape = match schema {
            Schema::Object(SchemaObject {
                object: Some(obj), ..
            }) => {
                obj.properties.contains_key("items")
                    && obj.properties.contains_key("next_page")
            }
            _ => false,
        };

        if has_expected_shape {
            results.insert(name.clone(), prefix.to_string());
        }
    }

    results
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
                instance_type:
                    Some(SingleOrVec::Single(instance_type)),
                enum_values: Some(values),
                ..
            }) if **instance_type == InstanceType::String
                && values.len() == 1 =>
            {
                let value = match &values[0] {
                    serde_json::Value::String(s) => s.clone(),
                    _ => return None,
                };
                let description = metadata
                    .as_ref()
                    .and_then(|m| m.description.clone());
                members.push(DescribedEnumMember { value, description });
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
    properties: Vec<VariantProperty>,
}

struct VariantProperty {
    name: String,
    ts_type: String,
    required: bool,
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
            if !tag_props.iter().any(|p| *p == tag) {
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
            })
            .collect();

        result_variants.push(DiscriminatedVariant {
            tag_value,
            model_name: format!("{}{}", tag.chars().next().unwrap().to_ascii_uppercase(), &tag[1..]),
            properties,
        });
    }

    // Use the tag value as the model name (it's typically PascalCase already).
    for v in &mut result_variants {
        v.model_name = v.tag_value.clone();
    }

    Some(DiscriminatedUnion { tag, variants: result_variants })
}

fn is_single_value_string_enum(schema: &Schema) -> bool {
    extract_single_enum_value(schema).is_some()
}

fn extract_single_enum_value(schema: &Schema) -> Option<String> {
    match schema {
        Schema::Object(SchemaObject {
            enum_values: Some(values),
            ..
        }) if values.len() == 1 => match &values[0] {
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Emit TypeSpec validation decorators for a schema's constraints.
fn emit_validation_decorators(
    out: &mut String,
    schema: &Schema,
    indent: &str,
) {
    let obj = match schema {
        Schema::Object(obj) => obj,
        _ => return,
    };

    // String format hints (uuid, ip, etc.) that aren't captured by the
    // type mapping. Integer/float formats (int32, uint64, float, double)
    // and date-time/uri are already reflected in the TypeSpec type.
    if let Some(fmt) = &obj.format {
        match fmt.as_str() {
            "date-time" | "date" | "uri" | "int8" | "int16" | "int32"
            | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
            | "float" | "double" => {}
            f => {
                writeln!(out, "{}@format(\"{}\")", indent, f).unwrap();
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
            writeln!(
                out,
                "{}@pattern(\"{}\")",
                indent,
                escape_tsp_string(pat)
            )
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
    matches!(
        schema,
        Schema::Object(SchemaObject { reference: Some(_), .. })
    )
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
                "#{{}}".to_string()
            } else {
                let fields: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        format!("{}: {}", k, json_to_tsp_literal(v))
                    })
                    .collect();
                format!("#{{{}}}", fields.join(", "))
            }
        }
    }
}

fn schema_description(schema: &Schema) -> Option<String> {
    match schema {
        Schema::Object(obj) => obj
            .metadata
            .as_ref()
            .and_then(|m| m.description.clone()),
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

fn is_valid_tsp_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn to_namespace_id(title: &str) -> String {
    // Convert title to PascalCase identifier.
    title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            let first = chars.next().unwrap().to_ascii_uppercase();
            let rest: String = chars.collect();
            format!("{}{}", first, rest)
        })
        .collect::<String>()
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

}
