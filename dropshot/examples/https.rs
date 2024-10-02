// Copyright 2020 Oxide Computer Company

//! Example use of Dropshot with TLS enabled

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::ConfigTls;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use dropshot::TypedBody;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::io::Write;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use tempfile::NamedTempFile;

// This function would not be used in a normal application. It is used to
// generate temporary keys and certificates for the purpose of this demo.
fn generate_keys() -> Result<(NamedTempFile, NamedTempFile), String> {
    let keypair =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .map_err(|e| e.to_string())?;
    let cert = keypair.cert.pem();
    let priv_key = keypair.key_pair.serialize_pem();

    let make_temp = || {
        tempfile::Builder::new()
            .prefix("dropshot-https-example-")
            .rand_bytes(5)
            .tempfile()
    };

    let mut cert_file =
        make_temp().map_err(|_| "failed to create cert_file")?;
    cert_file
        .write(cert.as_bytes())
        .map_err(|_| "failed to write cert_file")?;
    let mut key_file = make_temp().map_err(|_| "failed to create key_file")?;
    key_file
        .write(priv_key.as_bytes())
        .map_err(|_| "failed to write key_file")?;
    Ok((cert_file, key_file))
}

#[tokio::main]
async fn main() -> Result<(), String> {
    // Begin by generating TLS certificates and keys and stuffing them into a
    // TLS configuration.
    let (cert_file, key_file) = generate_keys()?;
    let config_tls = Some(ConfigTls::AsFile {
        cert_file: cert_file.path().to_path_buf(),
        key_file: key_file.path().to_path_buf(),
    });

    // See dropshot/examples/basic.rs for more details on most of these pieces.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    let mut api = ApiDescription::new();
    api.register(example_api_get_counter).unwrap();
    api.register(example_api_put_counter).unwrap();

    let api_context = ExampleContext::new();

    let server = ServerBuilder::new(api, api_context, log)
        // This differs from the basic example: provide the TLS configuration.
        .tls(config_tls)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    server.await
}

/// Application-specific example context (state shared by handler functions)
struct ExampleContext {
    /// counter that can be manipulated by requests to the HTTP API
    counter: AtomicU64,
}

impl ExampleContext {
    /// Return a new ExampleContext.
    pub fn new() -> ExampleContext {
        ExampleContext { counter: AtomicU64::new(0) }
    }
}

// HTTP API interface

/// `CounterValue` represents the value of the API's counter, either as the
/// response to a GET request to fetch the counter or as the body of a PUT
/// request to update the counter.
#[derive(Deserialize, Serialize, JsonSchema)]
struct CounterValue {
    counter: u64,
}

/// Fetch the current value of the counter.
#[endpoint {
    method = GET,
    path = "/counter",
}]
async fn example_api_get_counter(
    rqctx: RequestContext<ExampleContext>,
) -> Result<HttpResponseOk<CounterValue>, HttpError> {
    let api_context = rqctx.context();

    Ok(HttpResponseOk(CounterValue {
        counter: api_context.counter.load(Ordering::SeqCst),
    }))
}

/// Update the current value of the counter.  Note that the special value of 10
/// is not allowed (just to demonstrate how to generate an error).
#[endpoint {
    method = PUT,
    path = "/counter",
}]
async fn example_api_put_counter(
    rqctx: RequestContext<ExampleContext>,
    update: TypedBody<CounterValue>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    let api_context = rqctx.context();
    let updated_value = update.into_inner();

    if updated_value.counter == 10 {
        Err(HttpError::for_bad_request(
            Some(String::from("BadInput")),
            format!("do not like the number {}", updated_value.counter),
        ))
    } else {
        api_context.counter.store(updated_value.counter, Ordering::SeqCst);
        Ok(HttpResponseUpdatedNoContent())
    }
}
