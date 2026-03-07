# 2026-03-06 — Nexus External API: Feature Inventory for TypeSpec Generation

Source: `omicron/openapi/nexus/nexus-latest.json` (OpenAPI 3.0.3)
Stats: 213 paths, 457 schemas, 311 operations, 29 tags.

## What we already handle

- [x] Basic types: string, integer, number, boolean, array, object
- [x] Integer formats: int32, int64, uint8, uint16, uint32, uint64
- [x] Float formats: float, double
- [x] String formats: date-time, uuid, ip, ipv4, ipv6, uri
- [x] String enums (plain `type: string` + `enum` array) — 22 in nexus
- [x] Internally-tagged discriminated unions (`oneOf` with tag property) — 51 in nexus
- [x] Object models with properties and required fields
- [x] Optional properties (`?` in TypeSpec)
- [x] Nullable properties (`nullable: true` → `Type | null`) — 394 in nexus
- [x] Arrays (`type: array` + `items`) — 172 in nexus
- [x] `@doc` from descriptions
- [x] `@route` + HTTP method decorators
- [x] `@path`, `@query`, `@header`, `@body` parameter decorators
- [x] Response status codes (200, 201, 204)
- [x] Response headers (1 case in nexus: `location` on 303)
- [x] Tags

## What we need to add

### High priority (pervasive in nexus)

**oneOf-of-single-string-consts (described enums)**: 35 schemas where each
variant is `{type: "string", enum: ["Value"]}` with its own description. These
are serde string enums where each variant has a doc comment. We currently emit
these as a union of literal types; should emit as a TypeSpec `enum` with `@doc`
on each member.

**~~allOf single-$ref wrapper~~**: Not actually a concern. The 304 occurrences
of `allOf: [{$ref: "..."}]` in the OpenAPI JSON are an OAS 3.0 workaround
(sibling keywords next to `$ref` are ignored, so schemars wraps in `allOf`).
Since we generate TypeSpec from dropshot's schemars `Schema` objects directly —
not from the OpenAPI JSON — we never see this wrapper. The schemars schema has
the ref and metadata (nullable, description, default) together on the same
`SchemaObject`. TypeSpec can express this cleanly with no wrapper needed.

**Error responses**: Every operation has `4XX` and `5XX` responses referencing
`#/components/responses/Error`. The Error schema is
`{message: string, request_id: string, error_code?: string}`. Should emit as
an `@error` model and include in operation return types or handle via a
convention.

**Pagination (`ResultsPage<T>`)**: 65 `*ResultsPage` schemas, all with
`{items: T[], next_page?: string | null}`. 78 operations carry
`x-dropshot-pagination`. Phase 3 of the plan covers heuristic detection and
generic emission.

**Validation decorators**: Nexus uses:
- `minimum`: 216 uses (134 are `minimum: 0` for unsigned, 82 are `minimum: 1`)
- `maxLength`: 7, `minLength`: 6
- `pattern`: 14 uses across 10 unique patterns
- `maximum`: 1
- `minItems`: 3, `maxItems`: 8

Map to `@minValue`, `@maxValue`, `@minLength`, `@maxLength`, `@pattern`,
`@minItems`, `@maxItems`.

**Default values**: 33 real property defaults including `null`, `[]`, `false`,
`true`, string values, and a few complex object defaults.

**$ref parameters**: Most path and query params reference named schemas like
`NameOrId`, `Name`, sort-mode enums. 164 path params and 260 query params use
`$ref` rather than inline type definitions.

### Medium priority

**Status code 202 (Accepted)**: 12 operations. Need `@statusCode _: 202`.

**Status code 303 (See Other)**: 1 operation (SAML login). Need
`@statusCode _: 303` + `@header location`.

**`default` response**: 8 operations use a wildcard `*/*` schema response
(device auth, binary downloads, websocket). May need special handling.

**Untagged/externally-tagged unions**: 3 schemas (`IpNet`, `IpRange`,
`NameOrId`) use oneOf with `allOf`-wrapped `$ref` variants and `title` fields
instead of a tag property.

**Request body content types**: application/json (90), application/octet-stream
(2), application/x-www-form-urlencoded (2).

**Tag property names**: Not just `kind` — nexus uses `type` (39 schemas),
`kind` (6), `state` (1), `mode` (1), `allow` (1) as discriminator tags.

### Low priority / nice-to-have

**`x-dropshot-pagination` extension**: Could emit as a comment or custom
decorator. Value is `{required: string[]}`.

**`x-dropshot-websocket` extension**: 1 operation. Could emit as a comment.

**`x-rust-type` extension**: 3 schemas (`oxnet::IpNet`, `oxnet::Ipv4Net`,
`oxnet::Ipv6Net`). Informational only.

**Non-standard formats**: `ip`, `uint`, `hex string (32 bytes)`. These have
no TypeSpec built-in; emit as `string` with a comment or `@format` decorator.

**`format: binary`**: 2 request bodies. Maps to `bytes` in TypeSpec.

## Patterns not present in nexus

These features exist in TypeSpec but aren't needed for the nexus API:

- `additionalProperties` / map types (`Record<T>`)
- `readOnly` / `writeOnly`
- `deprecated` operations
- Integer enums (only 1 case: `BlockSize`)
- Deep nesting of $refs
- Array query params
- OpenAPI `discriminator` field (all discrimination is structural)

## Recommended implementation order

1. Described string enums (oneOf-of-string-consts → enum with @doc)
2. Error responses (@error model)
3. Validation decorators (@minValue, @maxLength, @pattern, etc.)
4. Default values
5. Additional status codes (202, 303)
6. ResultsPage generic detection
7. Untagged unions
