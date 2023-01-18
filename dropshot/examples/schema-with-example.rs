//! Example use of dropshot to output OpenAPI compatible JSON.  This program
//! specifically illustrates how to add examples to each schema using schemars,
//! and how that will be reflected in the resultant JSON generated when ran.

use dropshot::{
    endpoint, ApiDescription, HttpError, HttpResponseOk, RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// Define 2 structs here - Bar is nested inside Foo and should result in an
// example that looks like:
//
// {
//    "id": 1,
//    "bar": {
//      "id: 2
//    }
// }

#[derive(Deserialize, Serialize, JsonSchema)]
#[schemars(example = "foo_example")]
/// # Foo Object
struct Foo {
    /// Foo ID
    id: u32,

    /// All Foo's have a Bar
    bar: Bar,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[schemars(example = "bar_example")]
/// # Bar Object
struct Bar {
    /// Bar ID
    id: u32,
}

/// Used by schemars to generate the `Foo` example.
fn foo_example() -> Foo {
    Foo { id: 1, bar: bar_example() }
}

/// Used by schemars to generate the `Bar` example.
fn bar_example() -> Bar {
    Bar { id: 2 }
}

fn main() -> Result<(), String> {
    let mut api = ApiDescription::new();
    api.register(get_foo).unwrap();

    api.openapi("Examples", "0.0.0")
        .write(&mut std::io::stdout())
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[endpoint {
    method = GET,
    path = "/foo",
    tags = [ "foo" ],
}]
/// Get a foo
async fn get_foo(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<Foo>, HttpError> {
    let foo = foo_example();
    Ok(HttpResponseOk(foo))
}
