# 2026-03-06 — Plan: TypeSpec Generation from Dropshot

## Goal

Add the ability to generate a TypeSpec schema from `ApiDescription`, walking
dropshot's internal structures directly rather than converting from the built
OpenAPI struct.

## Why walk internal structures

The OpenAPI struct has already lost information that TypeSpec can express:
generics/templates, discriminated unions, semantic decorators. Walking
`ApiEndpoint`, `ApiEndpointParameter`, `ApiSchemaGenerator`, and schemars
`Schema` objects directly preserves more information and gives us a path toward
idiomatic TypeSpec output.

That said, for generics specifically (e.g. `ResultsPage<Project>`), the generic
relationship is already flattened by the time we have an `ApiEndpoint`. The
`ApiSchemaGenerator::Gen { name, .. }` calls `JsonSchema::schema_name()` which
returns e.g. `"ProjectResultsPage"` — a flat string. Recovery of generics will
require either heuristics (detecting `FooResultsPage` pattern and emitting
`ResultsPage<Foo>`) or future changes to carry richer type metadata. Heuristic
recovery for `ResultsPage` is tractable since it's a known dropshot type with
a predictable naming convention.

## Architecture

New module `dropshot/src/typespec.rs` containing:

```rust
/// Generate TypeSpec source from an ApiDescription.
pub fn api_to_typespec<C: ServerContext>(
    api: &ApiDescription<C>,
    title: &str,
    version: &semver::Version,
) -> String
```

This function walks the same structures that `gen_openapi()` walks:
- `api.router.endpoints(Some(version))` to iterate endpoints
- Each `ApiEndpoint`'s parameters, response, error, metadata
- `ApiSchemaGenerator` to get schemars schemas for models
- schemars `SchemaGenerator` to collect referenced type definitions

If iteration helpers are useful for both `gen_openapi` and `api_to_typespec`,
extract them. But don't create a shared intermediate representation — the two
outputs have different enough needs that abstraction would be premature.

## TypeSpec ↔ dropshot mapping

| Dropshot / schemars | TypeSpec |
|---|---|
| `ApiDescription` title | `@service(#{ title: "..." })` |
| top-level | `import "@typespec/http"; using Http;` |
| namespace | `namespace Title;` (sanitized to PascalCase) |
| `ApiEndpoint.method` + `.path` | `@route("/path") @get` (etc.) |
| `ApiEndpoint.operation_id` | `op operationId(...)` |
| `ApiEndpoint.parameters` Path(name) | `@path name: Type` |
| `ApiEndpoint.parameters` Query(name) | `@query name: Type` |
| `ApiEndpoint.parameters` Header(name) | `@header name: Type` |
| `ApiEndpoint.parameters` Body(Json) | `@body body: Type` |
| `ApiEndpoint.response.schema` | return type of op |
| `ApiEndpoint.response.success` (status code) | `@statusCode _: NNN` |
| `ApiEndpoint.response.headers` | `@header name: Type` in response model |
| `ApiEndpoint.error` | `@error model ErrorType { ... }` in union return |
| `ApiEndpoint.summary` / `.description` | `@doc("...")` |
| `ApiEndpoint.tags` | `@tag("name")` |
| `ApiEndpoint.deprecated` | `#deprecated` decorator |
| schemars object schema | `model Name { prop: Type; }` |
| schemars enum | `enum Name { ... }` |
| schemars `type: string` | `string` |
| schemars `type: integer, format: int32` | `int32` |
| schemars `type: integer, format: int64` | `int64` |
| schemars `type: boolean` | `boolean` |
| schemars `type: number, format: float` | `float32` |
| schemars `type: number, format: double` | `float64` |
| schemars `type: array, items: T` | `T[]` |
| schemars `nullable` | `Type \| null` |
| schemars `minLength`, `minimum`, etc. | `@minLength(n)`, `@minValue(n)` |
| optional (not in `required`) | `prop?: Type` |
| `$ref` / definition name | model name reference |
| free-form object | `Record<unknown>` |
| `ResultsPage<T>` (heuristic) | `model ResultsPage<T> { items: T[]; nextPage?: string; }` |

## Implementation plan

### Phase 1: Minimal proof of concept

Target: a tiny server with 1-2 GET endpoints, simple request/response types.
Get the output compiling under `tsp` and looking like what a human would write.

1. **Create `dropshot/src/typespec.rs`**
   - `api_to_typespec()` function
   - Internal helper to convert a schemars `Schema` to a TypeSpec type string
   - Internal helper to convert a schemars object schema to a `model` block

2. **Walk endpoints**: iterate `router.endpoints(Some(version))`, for each:
   - Emit `@route` + HTTP method decorator
   - Emit `op` with parameters and return type
   - Use the same `schemars::gen::SchemaGenerator` approach as `gen_openapi` to
     collect referenced definitions

3. **Walk collected schemas**: emit `model` definitions for each schema in the
   generator's definitions map.

4. **Emit preamble**: `import`, `using`, `@service`, `namespace`.

5. **Wire up**: add `typespec()` method to `OpenApiDefinition` (or on
   `ApiDescription` directly), write a test with expectorate against a `.tsp`
   snapshot.

6. **Validate**: `tsp compile` the output, confirm it's valid.

### Phase 2: Expand feature coverage ✓

All done:

- ✓ Enum types → `enum Name { ... }`
- ✓ Request bodies (`TypedBody<T>`) → `@body body: T`
- ✓ Multiple response status codes → union return types
- ✓ Error responses → `@error model` with `@statusCode` range, `| Error` in ops
- ✓ Doc comments → `@doc("...")`
- ✓ Tags → `@tag("name")`
- ✓ Validation decorators (`@minValue`, `@maxValue`, `@minLength`, `@maxLength`,
  `@pattern`, `@minItems`, `@maxItems`, exclusive variants)
- ✓ Nullable → `Type | null`
- ✓ Free-form JSON → `Record<unknown>`
- ✓ Response headers → `@header` in response model
- ✓ Path/query/header parameters with various types
- ✓ Default values → `prop?: Type = value` with full literal support
- ✓ Described string enums → `enum` with `@doc` per member
- ✓ Discriminated unions → `@discriminator` + extends pattern

### Phase 3: Generics and polish

- ✓ Detect `ResultsPage` pattern and emit as `model ResultsPage<T> { ... }`
  with generic usage at call sites
- ~~Consider operation grouping~~ — not doing this, flat ops are fine
- ✓ Dropshot extensions emitted as `@extension` decorators on operations:
  `@extension("x-dropshot-pagination", #{required: #[...]})` and
  `@extension("x-dropshot-websocket", #{})`
- Decide on public API surface (feature flag, method placement)

### Remaining medium-priority items

- ✓ Additional status codes (202, 303) — already handled by generic logic
- ✓ Untagged/externally-tagged unions — handled by union/scalar fallback
- Non-JSON request bodies (`application/octet-stream`, `x-www-form-urlencoded`)
- `format: binary` → `bytes`
- `default` response (wildcard `*/*` content type, 8 nexus operations)

## Open questions

- **Namespace naming**: the caller picks a namespace name that is already a valid
  TypeSpec identifier. No automatic sanitization of the title.
- **Operation grouping**: flat ops in top namespace vs. `interface` blocks.
  Start flat, iterate.
- **`4XX`/`5XX` error pattern**: TypeSpec uses `@error model` + union returns.
  Dropshot has a shared error type per API. Emit once as `@error model`, union
  into each op's return type.
- **Round-trip fidelity**: not a goal. The TypeSpec should be idiomatic and
  valid, not a lossless encoding of the OpenAPI output.

## Secondary goal: TypeSpec as the source of truth

Longer term, it would be worth exploring whether we could generate TypeSpec
*first* and then derive OpenAPI from it (using `tsp compile` with the
`@typespec/openapi3` emitter). This would mean:

- Dropshot generates `.tsp` files
- Anyone who wants OpenAPI runs `tsp compile` on that output
- We stop maintaining the `gen_openapi()` code path entirely

This is attractive because it collapses two code paths into one, and TypeSpec →
OpenAPI is a well-supported, maintained toolchain (it's TypeSpec's primary use
case). The OpenAPI output from `tsp compile` would be standards-compliant by
construction.

**Feasibility considerations:**

- TypeSpec is strictly more expressive than OpenAPI 3.0, so TypeSpec → OpenAPI
  is lossless for everything OpenAPI can represent. This direction works.
- The `tsp` compiler is a Node.js tool. Requiring Node at build time may or may
  not be acceptable. Could be a dev-dependency or CI-only step.
- The generated OpenAPI won't be byte-identical to what `gen_openapi()` produces
  today (different formatting, field ordering, description text). Consumers that
  diff OpenAPI output would see churn. This is a migration concern, not a
  blocking one.
- Round-trip testing: we could validate that the TypeSpec we generate, when
  compiled to OpenAPI, is semantically equivalent to what `gen_openapi()`
  produces. This would give confidence during a transition period.

**Approach:** Build the TypeSpec generation (Phase 1-3 above) first. Once the
output is solid and covers the full API surface, experiment with `tsp compile`
on the output and compare against the existing OpenAPI. If the results are
close enough, we can evaluate deprecating `gen_openapi()`.

## Target application

The flagship dropshot API we care about is the Oxide external API in
`oxidecomputer/omicron` (specifically `nexus/external-api`). The feature set
of the TypeSpec generator needs to cover everything that API uses. Before
declaring any phase complete, validate against the omicron external API to
discover gaps.

## Iteration strategy

Start with the simplest possible case. Get one endpoint looking right. Then
add complexity one feature at a time. Use TypeSpec docs at typespec.io as the
reference for what correct output looks like.
