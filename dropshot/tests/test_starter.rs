// Copyright 2024 Oxide Computer Company

//! Quick check that the "legacy" HttpServerStarter::new() and
//! HttpServerStarter::new_with_tls() interfaces work.

pub mod common;

use common::create_log_context;
use dropshot::endpoint;
use dropshot::test_util::read_json;
use dropshot::test_util::ClientTestContext;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigTls;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::HttpServerStarter;
use dropshot::RequestContext;

extern crate slog;

/// Test starting a server with `HttpServerStarter::new()`.
#[tokio::test]
async fn test_no_tls() {
    let api = demo_api();
    let logctx = create_log_context("test_no_tls");
    let starter =
        HttpServerStarter::new(&ConfigDropshot::default(), api, 0, &logctx.log)
            .unwrap();
    let server = starter.start();
    let server_addr = server.local_addr();
    let client = ClientTestContext::new(server_addr, logctx.log.clone());
    let mut response = client
        .make_request_no_body(http::Method::GET, "/demo", http::StatusCode::OK)
        .await
        .unwrap();
    let json: String = read_json(&mut response).await;
    assert_eq!(json, "demo");

    logctx.cleanup_successful();
}

/// Test starting a server with `HttpServerStarter::new_with_tls()`.
#[tokio::test]
async fn test_with_tls() {
    let logctx = create_log_context("test_with_tls");

    // Generate key for the server
    let (certs, key) = common::generate_tls_key();
    let (serialized_certs, serialized_key) =
        common::tls_key_to_buffer(&certs, &key);
    let config_tls = Some(ConfigTls::AsBytes {
        certs: serialized_certs.clone(),
        key: serialized_key.clone(),
    });

    let mut builder = reqwest::Client::builder();
    let certs =
        reqwest::Certificate::from_pem_bundle(&serialized_certs).unwrap();
    for c in certs {
        builder = builder.add_root_certificate(c);
    }
    let client = builder.build().unwrap();
    let api = demo_api();
    let starter = HttpServerStarter::new_with_tls(
        &ConfigDropshot::default(),
        api,
        0,
        &logctx.log,
        config_tls,
    )
    .unwrap();
    let server = starter.start();
    let server_addr = server.local_addr();
    // It would be nice to just write the whole sockaddr into the URL.  But
    // for TLS, we're required to use a server name and not an IP address here.
    let url = format!("https://localhost:{}/demo", server_addr.port());
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.json::<String>().await.unwrap(), "demo");

    logctx.cleanup_successful();
}

#[endpoint {
    method = GET,
    path = "/demo",
}]
async fn demo_handler(
    _rqctx: RequestContext<usize>,
) -> Result<HttpResponseOk<String>, HttpError> {
    Ok(HttpResponseOk(String::from("demo")))
}

fn demo_api() -> ApiDescription<usize> {
    let mut api = ApiDescription::<usize>::new();
    api.register(demo_handler).unwrap();
    api
}
