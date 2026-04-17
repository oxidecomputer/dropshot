# What tsp captures that OpenAPI doesn't

2026-04-17

A comparison of the generated `/tmp/nexus-external.tsp` against the existing
dropshot-emitted OpenAPI at `omicron/openapi/nexus/nexus-latest.json`,
focusing on places where the tsp is a more faithful representation of the
underlying Rust API than the OpenAPI is — either because OpenAPI can't express
the concept at all, or because it can only express it by convention (shape
matching, naming, or dropshot-specific vendor extensions) that client
generators have to recover via implicit knowledge of how dropshot works.

The previous doc (`tsp-openapi-comparison.md`) compared the tsp-derived OpenAPI
to the dropshot-derived OpenAPI. This one is about what the tsp itself carries
that OpenAPI flattens or loses.

## Structural advantages of the tsp

### Discriminated unions with named variants

A Rust `#[serde(tag = "type")]` enum like `BinRange<f64>` has a parent type, a
discriminator field, and named variants that each add their own fields. In the
tsp this round-trips directly:

    @discriminator("type")
    model BinRangedouble {
      type: string;
    }

    model BinRangedoubleRangeTo extends BinRangedouble {
      type: "range_to";
      end: float64;
    }

    model BinRangedoubleRange extends BinRangedouble {
      type: "range";
      end: float64;
      start: float64;
    }

In the OpenAPI the same type becomes a `oneOf` with inline variant objects and
no `discriminator` keyword at all — the discriminator is present only as a
`type` property with a single-value `enum` constraint in each variant. The
relationship between the parent type and its variants is not expressed: the
variants have no names in components, no reference to the parent, and the
discriminator field is something the client has to notice by looking at which
property is a constant across all alternatives. OpenAPI's formal
`discriminator`/`mapping` mechanism would at least name the relationship, but
dropshot doesn't emit it.

The upshot is that every nexus client has to implement the same pattern-match:
"if a `oneOf` has a property present in every branch and that property is a
string constant in each branch, that's the tag." The tsp states this directly.

### Generic templates

`ResultsPage<T>` is a single declaration in the tsp:

    model ResultsPage<T> {
      items: T[];
      next_page?: string | null;
    }

There are 80 uses of this template across operation return types. In the
OpenAPI, every `T` gets its own monomorphized schema —
`DiskResultsPage`, `InstanceResultsPage`, `ProjectResultsPage`, and so on — 64
of them. These schemas are structurally identical modulo the item type.
A client that wants to recognize "this is a page of results" has to match on
the name suffix `ResultsPage` or on the shape `{items: T[], next_page?: string
| null}`. The fact that it's an instantiation of one generic is information
dropshot's OpenAPI drops, and that every client has to recover separately.

The same pattern appears more severely for `BinRange<T>` and `Histogram<T>`
in the oximeter types: the OpenAPI has `BinRangedouble`, `BinRangefloat`,
`BinRangeint8` … through `BinRangeuint64`, each with three variants —
30 near-duplicate variant schemas from what is, in the Rust source, one generic
type. The tsp currently has the same monomorphized shape (see the generation
opportunities section below).

### Named scalars with constraints

Dropshot-level newtypes like `Name`, `Hostname`, `Ipv4Net`, `ByteCount`,
`TimeseriesName`, `Vni`, `MacAddr`, `L4PortRange`, `InstanceCpuCount`, and
`Password` become first-class scalars in the tsp:

    @pattern("^(?![0-9a-fA-F]{8}-…")
    @minLength(1)
    @maxLength(63)
    scalar Name extends string;

This is a nominal type that extends `string`. A client generator can choose to
produce a distinct `Name` type whose identity is preserved across uses.
OpenAPI can express most of the same constraints on a schema, and dropshot
does emit `Name` as a component, but there's no notion of "this is a semantic
scalar type that extends `string`" — just a schema named `Name` that happens to
be of type `string`. Most OpenAPI client generators flatten this kind of alias
to a bare `string` with no type-level identity at call sites. The tsp makes
the identity first-class.

For `uuid` specifically, the dropshot OpenAPI inlines `{type: string, format:
uuid}` at every use site and has no `uuid` component. The tsp declares it
once.

### Named union alternatives

`union NameOrId { id: uuid, name: Name }` in the tsp gives each alternative a
name. The OpenAPI version is a `oneOf` with `title: "id"` and `title: "name"`
on each branch; the `title` is informational text, not a field name, and most
OpenAPI tooling treats it as documentation. Rust and TypeScript clients that
want to generate a proper tagged sum type (or at least a discriminated API
surface) have to read the titles and treat them as identifiers, which is
essentially a dropshot-specific convention.

### Enums with per-member documentation

First-class named enums with `@doc` on each member:

    enum NameOrIdSortMode {
      @doc("Sort in increasing order of \"name\"")
      name_ascending,
      @doc("Sort in decreasing order of \"name\"")
      name_descending,
      @doc("Sort in increasing order of \"id\"")
      id_ascending
    }

OpenAPI's `enum` is a flat array of string values. Per-member descriptions
require either vendor extensions (`x-enum-descriptions`) or splitting the enum
into a `oneOf` of single-value schemas. The tsp model is exactly what Rust
has and what most client languages want.

### Model composition

TypeSpec's `extends` and spread (`...Base`) preserve the intent of "this model
includes the shape of that other model, plus these additions." OpenAPI
expresses the same idea via `allOf`, which is structurally equivalent but
semantically opaque — `allOf` means "all of these schemas apply," which
happens to be how inheritance is encoded, but a consumer has to infer intent.
The tsp preserves the distinction between inheritance and general
intersection.

### Result types that pair success and error

A dropshot operation that can return `Disk` or `Error` is one return type in
the tsp:

    op disk_view(...): Disk | Error;

The `Error` model has `@error` on it, and the emitter knows to produce
4XX/5XX responses for the error branch and 2XX for the success branch.
OpenAPI expresses the same thing with a response object keyed by status code,
which is fine for documentation but forces every client generator to re-derive
"this operation has one success path and one error path" by scanning codes.
The tsp expresses it at the type level.

### Namespaces

The tsp organizes the whole API under a single `namespace OxideRegionAPI`.
TypeSpec supports nested namespaces for grouping. OpenAPI has only tags, which
are flat labels on operations; they can't nest and they can't scope types.
This doesn't matter much for a flat API like nexus, but it's a structural
capability OpenAPI lacks.

### Typed decorators vs vendor extensions

TypeSpec decorators (`@route`, `@get`, `@tag`, `@summary`, `@doc`,
`@minValue`, `@pattern`, `@error`, `@discriminator`) carry typed arguments
checked by the compiler. OpenAPI vendor extensions (`x-dropshot-pagination`,
`x-rust-type`, `x-enum-descriptions`) are untyped opaque JSON. Migrating
dropshot conventions from OpenAPI extensions to tsp decorators would make
them machine-checkable and introspectable instead of stringly-typed.

## What this means for client generators

The common thread: nexus client generators today have to bake in knowledge of
dropshot's output patterns. "Schemas ending in `ResultsPage` are paginated
lists." "A `oneOf` where every branch has the same string-constant property is
an internally-tagged enum." "A `oneOf` with `title` strings on each branch is
a named untagged union." "Error response bodies live at the
`components/responses/Error` ref." "Any schema whose `type` is `string` and
whose description mentions 'UUID' is a UUID." Every one of these is a
dropshot-specific convention the client has to recognize.

A tsp-native generator pipeline doesn't need any of that convention. The type
system carries the information directly: `ResultsPage<T>` is a template
instantiation, not a name suffix; a discriminated union has a `@discriminator`
decorator and named variants; `NameOrId` is a named union with named
alternatives; errors are models tagged `@error`; UUIDs are the `uuid` scalar.
Each of these is something a generator inspects structurally rather than
recovers heuristically.

## Opportunities to generate more compact tsp

The Rust source has generic type information that the current tsp generator
drops because it consumes schemars-produced JSON Schema, which is already
monomorphized. Closing this gap would require the generator to track generic
relationships from an earlier stage than schemars.

**`BinRange<T>` and `Histogram<T>` in oximeter.** There are 10 near-identical
copies of `BinRange` (one per numeric type) and 10 of `Histogram`. The shape
of each variant (`RangeTo`, `Range`, `RangeFrom`) differs only in whether
`start` and `end` are `int8`, `uint16`, `float64`, etc. In the Rust source
this is a single `BinRange<T: Bin>` generic with bounded `T`. Emitting the
tsp as one template with a scalar type parameter would take the 40 BinRange*
schemas down to 4 (one parent, three variants, all templated), and similar
for Histogram.

**Common parameter sets.** Every paginated operation has the same three query
parameters (`limit`, `page_token`, plus the pagination-specific
`x-dropshot-pagination` required set). Extracting them into a shared parameter
model that operations spread via `...PaginationParams` would reduce the
operation-level duplication.

**Discriminated-union variant naming.** The emitter currently produces names
like `BinRangedoubleRangeTo` by concatenating the parent name and the variant
name. TypeSpec's namespace mechanism would allow `BinRange.Double.RangeTo` or
similar, keeping component names readable and scoping variants under their
parent.

These are secondary — the tsp is already strictly more faithful than the
OpenAPI even in its current form — but each of them would take the
representation closer to how the Rust source actually looks.

## Where the tsp is worse

Nothing structural turned up. The cost of tsp over OpenAPI is ecosystem: a
younger spec, fewer validators, no equivalent of Swagger UI or Redoc out of
the box, and a Node-only toolchain. For documentation hosting and third-party
interop, OpenAPI remains the lingua franca. For anything that consumes the
spec to generate code, the tsp is a strictly richer input.

## Verdict

The tsp expresses the Rust API more directly than OpenAPI does. The places
where this matters most are the ones where dropshot's OpenAPI forces client
generators to recognize conventions — discriminated unions without
`discriminator`, generics flattened to name-suffix patterns, named scalars
that lose their identity, untagged unions whose alternative names live in
`title` strings, errors split across status-coded response objects. Each of
these is a case where the tsp type system carries the information the OpenAPI
requires the client to infer.

The remaining gap between the tsp and the Rust source is mostly about
generics — the schemars-derived monomorphization loses information that the
Rust types still carry. Closing that gap (starting with oximeter's BinRange
and Histogram) would leave the tsp a near-lossless projection of the
dropshot-level API.

## Counterargument

> I had Claude make the strongest counterargument it could to the above.

Most of what this doc calls "OpenAPI limitations" are dropshot-emission quality
issues, not spec-level ones. OpenAPI 3.0+ has `discriminator.propertyName` and
`discriminator.mapping`, which expresses named-variant discriminated unions as
explicitly as `@discriminator` does; dropshot just doesn't emit it. OpenAPI
supports named component schemas with constraints, and the reason `Name` loses
identity in downstream clients is that most OpenAPI client generators flatten
aliases to `string` — a generator-quality problem, not a spec-expressiveness
problem. If the dropshot OpenAPI emitter produced proper discriminator
mappings, canonical error responses, and richer metadata on scalar components,
most of the "client has to infer" list collapses.

Generics are a source-language convenience, not a wire-level semantic.
`ResultsPage<T>` and `BinRange<T>` look economical in the tsp file, but
TypeSpec templates are compile-time substitution: by the time anything reaches
the wire, or even the openapi3 emitter, the templates have been monomorphized.
The "64 duplicate schemas vs. one generic" framing overstates the win — the
tsp source is shorter, but a generator consuming it still instantiates 64
types. The honest shape of the JSON API is 64 types.

Client generators for OpenAPI already handle the dropshot conventions.
Progenitor, the TypeScript client, and openapi-generator all know how to
recognize `ResultsPage`, internally-tagged `oneOf`, UUID format, and so on.
The "every client has to bake in dropshot-isms" complaint describes work that
has already been done. Moving to tsp means redoing it in a new toolchain and
taking on a Node-only build dependency in a codebase that's otherwise
all-Rust.

TypeSpec has its own conventions that clients have to learn, so the
"convention-load" argument isn't a reduction — it's a migration. Which
`@error` union branches map to which status codes, how `extends` and spread
behave at the wire level, how templates get emitted as components, what `@doc`
vs `@summary` mean — these are tsp-specific rules a generator has to
internalize. The heuristics-on-OpenAPI critique applies in some form to any
declarative API spec.

The OpenAPI ecosystem isn't window dressing. Spectral, openapi-diff, Prism
mocking, 42Crunch scanning, contract-testing frameworks — these are
OpenAPI-only and load-bearing for API governance. Swagger UI is the most
visible piece but not the most important. TypeSpec's tooling story is thinner
and largely Microsoft-controlled.

Finally, if fidelity to the Rust source is the real goal, the tsp doesn't
close the gap; it's another intermediate. Both the tsp and the OpenAPI are
strictly less faithful than generating clients directly from the dropshot
`ApiDescription`. The Progenitor-via-OpenAPI pipeline is an existing detour;
a tsp-as-canonical pipeline is a different detour of similar length. Neither
is the Rust source itself.

The doc's thesis still stands if the frame is "given that we want a portable
spec artifact, tsp expresses more of what the Rust source carries." But the
skeptic's version is: most of what we're crediting to the spec belongs to the
emitter, and improving the OpenAPI emitter would get us most of the way there
for a fraction of the ecosystem cost.
