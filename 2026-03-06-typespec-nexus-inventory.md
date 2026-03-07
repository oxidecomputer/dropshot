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

**~~oneOf-of-single-string-consts (described enums)~~**: Done. Emitted as
TypeSpec `enum` with `@doc` on each member.

**~~allOf single-$ref wrapper~~**: Not a concern. The 304 occurrences in the
OpenAPI JSON are an OAS 3.0 workaround. Since we generate from schemars
`Schema` objects directly, we never see this wrapper.

**~~Error responses~~**: Done. Error models get `@error` decorator and a
synthetic `@statusCode` field with `@minValue(400) @maxValue(599)`. Operations
return `SuccessType | Error`.

**~~Pagination (`ResultsPage<T>`)~~**: Done. Heuristic detection emits a
generic `model ResultsPage<T>` and rewrites concrete names to generic form.

**~~Validation decorators~~**: Done. Emits `@minValue`, `@maxValue`,
`@minValueExclusive`, `@maxValueExclusive`, `@minLength`, `@maxLength`,
`@pattern`, `@minItems`, `@maxItems`.

**~~Default values~~**: Done. Property defaults from schemars metadata emitted
as TypeSpec value literals (numbers, booleans, strings, `#[]` for arrays,
`#{...}` for objects, `null`).

**~~$ref parameters~~**: Not a concern. The `schema2struct` extraction in
dropshot dereferences `$ref`s when building parameter schemas. The
`ApiSchemaGenerator::Static` for parameters contains inline schemas with
resolved types plus a `dependencies` map. The "$ref parameters" in the nexus
OpenAPI are re-introduced by the OpenAPI code path, not present in the internal
structures we walk.

### Medium priority

**~~Status code 202 (Accepted)~~**: Done. Already handled by the generic
status code logic — any non-200/204 code emits a block with `@statusCode`.

**~~Status code 303 (See Other)~~**: Done. Same mechanism, plus response
headers (`@header location`) are emitted correctly.

**`default` response**: 8 operations use a wildcard `*/*` schema response
(device auth, binary downloads, websocket). May need special handling.

**~~Untagged/externally-tagged unions~~**: Already handled. Non-discriminated
`oneOf`/`anyOf` falls through to TypeSpec `union` or `scalar extends` emission.
Primitive-only untagged enums (like `NameOrId` = `string | uint64`) emit as
`scalar NameOrId extends string | uint64`. Object-variant unions emit as
`union Name { variant1, variant2 }`.

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

1. ~~Described string enums (oneOf-of-string-consts → enum with @doc)~~
2. ~~Error responses (@error model)~~
3. ~~Validation decorators (@minValue, @maxLength, @pattern, etc.)~~
4. ~~Default values~~
5. ~~ResultsPage generic detection~~
6. ~~Additional status codes (202, 303)~~
7. ~~Untagged unions~~
