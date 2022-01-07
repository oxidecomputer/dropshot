use std::sync::Arc;

use slog::Drain;
use dropshot::{endpoint, HttpResponseOk, HttpError, RequestContext, ConfigDropshot, ApiDescription, HttpServerStarter};
use serde::Serialize;
use schemars::JsonSchema;
use uuid::Uuid;

// Server context is available to every endpoint
struct ServerContext {
    uuid: Uuid,
}

// Types returned by endpoints should at minimum derive the following
#[derive(Serialize, JsonSchema)]
struct HelloWorld {
    message: String,
    uuid: Uuid,
}

// Endpoints are defined with a method and path, and minimally require the
// request context argument.
#[endpoint {
    method = GET,
    path = "/hello"
}]
async fn hello_world(
    rqctx: Arc<RequestContext<Arc<ServerContext>>>
) -> Result<HttpResponseOk<HelloWorld>, HttpError> {
    let apictx = rqctx.context();

    let return_value = HelloWorld {
        message: "Hello from dropshot!".to_string(),
        uuid: apictx.uuid,
    };

    Ok(HttpResponseOk(return_value))
}

// Endpoint registration is performed in this separate function so that map_err
// (or let Err(s) like below) can be called only once - registration returns a
// String error and this usually needs to be wrapped in something else.
fn register_endpoints(
    api_description: &mut ApiDescription<Arc<ServerContext>>
) -> Result<(), String> {
    api_description.register(hello_world)?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    let log = slog::Logger::root(drain, slog::o!());

    // ConfigDropshot defines general dropshot options.
    let config = ConfigDropshot {
        bind_address: "127.0.0.1:4567".parse()?,
        ..Default::default()
    };

    // Register endpoints
    let mut api_description = ApiDescription::<Arc<ServerContext>>::new();

    if let Err(s) = register_endpoints(&mut api_description) {
        anyhow::bail!("Error from register_endpoints: {}", s);
    }

    // Define the server context in an Arc. In this case our server context is
    // simply a random UUID.
    let ctx = Arc::new(ServerContext { uuid: Uuid::new_v4() });

    slog::info!(&log, "Server uuid is {}", &ctx.uuid);

    // Build the dropshot server
    let http_server = HttpServerStarter::new(
        &config,
        api_description,
        Arc::clone(&ctx),
        &log,
    )?;

    // Run the dropshot server and await termination.
    if let Err(s) = http_server.start().await {
        anyhow::bail!("Error from start(): {}", s);
    }

    Ok(())
}
