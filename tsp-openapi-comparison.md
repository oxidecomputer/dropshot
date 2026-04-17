# TypeSpec OpenAPI3 emitter output vs existing dropshot OpenAPI

2025-04-15

Comparison of OpenAPI generated from `/tmp/nexus-external.tsp` via the official
`@typespec/openapi3` emitter (v1.11.0) against the existing dropshot-generated
schema at `../omicron/openapi/nexus/nexus-latest.json`.

## High-level match

All 213 paths, HTTP methods, and operation IDs match exactly between the two
schemas.

## Emitter artifacts (info is in the tsp, emitter makes different choices)

**Discriminated unions.** The emitter produces `discriminator` + `$ref` to
separate variant schemas (e.g., `DiskStateCreating`, `DiskStateAttached`).
Existing uses `oneOf` with inline variant objects. All variant info (fields,
descriptions, discriminator values) is present in the tsp via `@discriminator` +
`extends` pattern.

**ResultsPage.** The emitter inlines the generic `ResultsPage<T>` at each use
site. Existing creates named `FooResultsPage` schemas (e.g.,
`DiskResultsPage`). The type parameter is in the tsp and could be
monomorphized.

**Untagged unions.** `NameOrId` is emitted as `anyOf` with bare `$ref`.
Existing uses `oneOf` with `title` per variant and `allOf` wrapping. The
variant names (`id`, `name`) are in the tsp `union` declaration.

**uuid scalar.** The emitter creates a named `uuid` schema and `$ref`s to it
(often wrapped in `allOf` when combined with `description`). Existing always
inlines `{type: string, format: uuid}` and has no `uuid` in
`components/schemas`.

**Error responses.** The emitter inlines error content in every operation's
4XX/5XX responses. Existing puts `Error` in `components/responses` and uses
`$ref`. The `@error` model has all the needed info.

**Other emitter defaults:**
- 200 response description: "The request has succeeded." vs "successful operation"
- `explode: false` on all query params (absent in existing)
- OpenAPI version 3.0.0 vs 3.0.3
- 577 schemas (variants broken out) vs 457 (variants inlined)

## Verdict

The typespec contains all the structural information needed to generate the
existing OpenAPI schema: discriminated union variants with
fields/descriptions/discriminator values, generic ResultsPage with type
parameters, named union variants, error model, and all operation metadata.

If we needed a byte-compatible OpenAPI output, a custom emitter would need to
handle discriminator inlining, ResultsPage monomorphization, uuid inlining, etc.
But since the goal is for tsp to be the canonical format and OpenAPI to be a
legacy derived artifact, the official `@typespec/openapi3` emitter is likely good
enough.

## Broader considerations: what role should the tsp play?

### The sync problem

Today, tsp and OpenAPI are generated independently from the same Rust source
(the dropshot server definition). This means they could diverge silently — a bug
in one codepath but not the other, or a feature added to one but not reflected in
the other. Generating OpenAPI *from* the tsp would make the tsp the single
source of truth and guarantee they agree.

### Rust tsp parser

If the Rust SDK generator consumes tsp directly (rather than going through
TypeSpec's own emitter infra), it needs a Rust parser. The parser scope is
narrower than "full TypeSpec" since we control the generator — only the subset
the dropshot emitter actually produces needs to be handled.

No Rust tsp parser crate exists today. The closest thing is
`happenslol/tree-sitter-typespec` (a tree-sitter grammar usable via Rust
bindings, gives a CST but not a typed AST). The TypeSpec compiler is a ~2500-line
recursive-descent parser in TypeScript. A formal grammar exists in the TypeSpec
repo at `packages/spec/src/spec.emu.html`.

This may be unnecessary if the Rust SDK is generated via TypeSpec's own client
emitter infra (see below).

### TypeSpec's own client generator infra

TypeSpec has client emitters for several languages (`@typespec/http-client-python`,
`@typespec/http-client-java`, `@typespec/http-client-csharp`, etc.) backed by
heavy Azure investment. Using that infrastructure means SDK generation for
multiple languages without building each one from scratch, and the generated
clients would follow conventions the TypeSpec ecosystem has already worked out.

The downside is the TypeScript/Node dependency in the build chain — an aesthetic
and practical mismatch for an otherwise all-Rust project (CI, reproducibility,
version pinning).

A possible middle ground: use TypeSpec's emitter infra for languages where Oxide
doesn't already have a mature generator, and keep the Rust SDK generator custom
(it already exists and has Oxide-specific opinions). The Rust SDK is the one
where giving up control is least appealing.

However, if other-language SDKs are generated from tsp via TypeSpec emitters and
the Rust SDK is generated from OpenAPI, you're back to needing both outputs in
sync — which is the original problem. This suggests the Rust SDK should also
consume tsp, or at minimum the OpenAPI should be derived from the tsp rather than
generated independently.

### End state

If the goal is for tsp to be the canonical API description format — with all
SDK generation (Rust and other languages) consuming the tsp directly, and
OpenAPI reduced to a derived artifact for backward compatibility — then a few
things follow:

- The tsp generator has to be complete and precise enough to be the sole input
  for SDK generation. Gaps should be addressed as SDK generator work surfaces
  them.
- OpenAPI needs to be derived *from* the tsp rather than generated
  independently, so the two can't drift. The official `@typespec/openapi3`
  emitter is probably fine for this — it just needs to produce a valid document
  describing the same API, not a byte-for-byte match with today's output.
- Given the above, the emitter-artifact differences cataloged earlier
  (discriminator style, ResultsPage naming, uuid inlining, etc.) don't need to
  be fixed.
- SDK generators need a path from tsp to code: either via TypeSpec's own client
  emitter infra, or via a Rust tsp parser if we want to keep the Rust SDK
  generator in-tree and Rust-only.

If instead the goal is more modest — tsp as an additional output for
interoperability, with the dropshot-generated OpenAPI remaining canonical —
then none of the above is required, and the existing independent generation is
fine.
