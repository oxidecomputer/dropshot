use dropshot::{
    endpoint, ApiDescription, HttpError, HttpResponseOk, RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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

fn foo_example() -> Foo {
    Foo { id: 1, bar: bar_example() }
}

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
    _rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<Foo>, HttpError> {
    let foo = foo_example();
    Ok(HttpResponseOk(foo))
}
