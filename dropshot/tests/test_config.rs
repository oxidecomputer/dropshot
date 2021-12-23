// Copyright 2020 Oxide Computer Company
/*!
 * Tests for configuration file.
 */

use dropshot::test_util::read_config;
use dropshot::ConfigDropshot;
use dropshot::HttpServerStarter;
use slog::Logger;
use std::fs;

mod common;

/*
 * Bad values for "bind_address"
 */

#[test]
fn test_config_bad_bind_address_port_too_small() {
    let error = read_config::<ConfigDropshot>(
        "bad_bind_address_port_too_small",
        "bind_address = \"127.0.0.1:-3\"",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.starts_with("invalid IP address syntax for key `bind_address`")
    );
}

#[test]
fn test_config_bad_bind_address_port_too_large() {
    let error = read_config::<ConfigDropshot>(
        "bad_bind_address_port_too_large",
        "bind_address = \"127.0.0.1:65536\"",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.starts_with("invalid IP address syntax for key `bind_address`")
    );
}

#[test]
fn test_config_bad_bind_address_garbage() {
    let error = read_config::<ConfigDropshot>(
        "bad_bind_address_garbage",
        "bind_address = \"garbage\"",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.starts_with("invalid IP address syntax for key `bind_address`")
    );
}

/*
 * Bad values for "request_body_max_bytes"
 */

#[test]
fn test_config_bad_request_body_max_bytes_negative() {
    let error = read_config::<ConfigDropshot>(
        "bad_request_body_max_bytes_negative",
        "request_body_max_bytes = -1024",
    )
    .unwrap_err()
    .to_string();
    assert!(error.starts_with("invalid value: integer"));
}

#[test]
fn test_config_bad_request_body_max_bytes_too_large() {
    let error = read_config::<ConfigDropshot>(
        "bad_request_body_max_bytes_too_large",
        "request_body_max_bytes = 999999999999999999999999999999",
    )
    .unwrap_err()
    .to_string();
    assert!(error.starts_with(""));
}

/*
 * Bad values for "key_file"
 */

#[test]
fn test_config_bad_key_file_garbage() {
    let error =
        read_config::<ConfigDropshot>("bad_key_file_garbage", "key_file = 23")
            .unwrap_err()
            .to_string();
    assert!(error.starts_with("invalid type: integer"));
}

/*
 * Bad values for "cert_file"
 */

#[test]
fn test_config_bad_cert_file_garbage() {
    let error = read_config::<ConfigDropshot>(
        "bad_cert_file_garbage",
        "cert_file = 23",
    )
    .unwrap_err()
    .to_string();
    assert!(error.starts_with("invalid type: integer"));
}

/*
 * Bad values for "https"
 */

#[test]
fn test_config_bad_https_garbage() {
    let error =
        read_config::<ConfigDropshot>("bad_https_garbage", "https = 23")
            .unwrap_err()
            .to_string();
    assert!(error.starts_with("invalid type: integer"));
}

fn make_server(
    config: &ConfigDropshot,
    log: &Logger,
) -> HttpServerStarter<i32> {
    HttpServerStarter::new(&config, dropshot::ApiDescription::new(), 0, log)
        .unwrap()
}

#[derive(Clone, Copy)]
struct TlsConfig<'a> {
    pub cert_file: &'a str,
    pub key_file: &'a str,
}

fn make_config(
    bind_ip_str: &str,
    bind_port: u16,
    tls: Option<TlsConfig>,
) -> ConfigDropshot {
    let (https, cert_file, key_file) = match tls {
        Some(config) => (true, config.cert_file, config.key_file),
        None => (false, "", ""),
    };
    let mut config_text = format!(
        "bind_address = \"{}:{}\"\n\
         request_body_max_bytes = 1024",
        bind_ip_str, bind_port,
    );
    if https {
        config_text = format!(
            "{}\n\
             https = {}\n\
             cert_file = \"{}\"\n\
             key_file = \"{}\"",
            config_text, https, cert_file, key_file
        );
    }
    read_config::<ConfigDropshot>("bind_address", &config_text).unwrap()
}

#[tokio::test]
async fn test_config_bind_address_http() {
    let log_path =
        dropshot::test_util::log_file_for_test("config_bind_address_http")
            .as_path()
            .display()
            .to_string();
    eprintln!("log file: {}", log_path);

    let log_config = dropshot::ConfigLogging::File {
        level: dropshot::ConfigLoggingLevel::Debug,
        path: log_path.clone(),
        if_exists: dropshot::ConfigLoggingIfExists::Append,
    };
    let log = log_config.to_logger("test_config_bind_address_http").unwrap();

    let client = hyper::Client::new();
    let bind_ip_str = "127.0.0.1";
    let bind_port = 12215;

    /*
     * This helper constructs a GET HTTP request to
     * http://$bind_ip_str:$port/, where $port is the argument to the
     * closure.
     */
    let cons_request = |port: u16| {
        let uri = hyper::Uri::builder()
            .scheme("http")
            .authority(format!("{}:{}", bind_ip_str, port).as_str())
            .path_and_query("/")
            .build()
            .unwrap();
        hyper::Request::builder()
            .method(http::method::Method::GET)
            .uri(&uri)
            .body(hyper::Body::empty())
            .unwrap()
    };

    /*
     * Make sure there is not currently a server running on our expected
     * port so that when we subsequently create a server and run it we know
     * we're getting the one we configured.
     */
    let error = client.request(cons_request(bind_port)).await.unwrap_err();
    assert!(error.is_connect());

    /*
     * Now start a server with our configuration and make the request again.
     * This should succeed in terms of making the request.  (The request
     * itself might fail with a 400-level or 500-level response code -- we
     * don't want to depend on too much from the ApiServer here -- but we
     * should have successfully made the request.)
     */
    let tls = None;
    let config = make_config(bind_ip_str, bind_port, tls);
    let server = make_server(&config, &log).start();
    client.request(cons_request(bind_port)).await.unwrap();
    server.close().await.unwrap();

    /*
     * Make another request to make sure it fails now that we've shut down
     * the server.  We need a new client to make sure our client-side connection
     * starts from a clean slate.  (Otherwise, a race during shutdown could
     * cause us to successfully send a request packet, only to have the TCP
     * stack return with ECONNRESET, which gets in the way of what we're trying
     * to test here.)
     */
    let client = hyper::Client::new();
    let error = client.request(cons_request(bind_port)).await.unwrap_err();
    assert!(error.is_connect());

    /*
     * Start a server on another TCP port and make sure we can reach that
     * one (and NOT the one we just shut down).
     */
    let config = make_config(bind_ip_str, bind_port + 1, tls);
    let server = make_server(&config, &log).start();
    client.request(cons_request(bind_port + 1)).await.unwrap();
    let error = client.request(cons_request(bind_port)).await.unwrap_err();
    assert!(error.is_connect());
    server.close().await.unwrap();

    let error = client.request(cons_request(bind_port)).await.unwrap_err();
    assert!(error.is_connect());
    let error = client.request(cons_request(bind_port + 1)).await.unwrap_err();
    assert!(error.is_connect());

    fs::remove_file(log_path).unwrap();
}

#[tokio::test]
async fn test_config_bind_address_https() {
    let log_path =
        dropshot::test_util::log_file_for_test("config_bind_address_https")
            .as_path()
            .display()
            .to_string();
    eprintln!("log file: {}", log_path);

    let log_config = dropshot::ConfigLogging::File {
        level: dropshot::ConfigLoggingLevel::Debug,
        path: log_path.clone(),
        if_exists: dropshot::ConfigLoggingIfExists::Append,
    };
    let log = log_config.to_logger("test_config_bind_address_https").unwrap();

    // Generate key for the server
    let (cert, key) = common::generate_tls_key();
    let (cert_file, key_file) = common::tls_key_to_file(&cert, &key);

    let make_client = || {
        // Configure TLS to trust the self-signed cert
        let mut root_store = rustls::RootCertStore { roots: vec![] };
        root_store.add(&cert).expect("adding root cert");

        let tls_config = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls_config)
            .https_only()
            .enable_http1()
            .build();
        hyper::Client::builder().build(https_connector)
    };

    let client = make_client();
    let hostname = "localhost";
    let bind_ip_str = "127.0.0.1";
    /* This must be different than the bind_port used in the http test. */
    let bind_port = 12217;

    /*
     * This helper constructs a GET HTTP request to
     * http://$bind_ip_str:$port/, where $port is the argument to the
     * closure.
     */
    let cons_request = |port: u16| {
        let uri = hyper::Uri::builder()
            .scheme("https")
            .authority(format!("{}:{}", hostname, port).as_str())
            .path_and_query("/")
            .build()
            .unwrap();
        hyper::Request::builder()
            .method(http::method::Method::GET)
            .uri(&uri)
            .body(hyper::Body::empty())
            .unwrap()
    };

    /*
     * Make sure there is not currently a server running on our expected
     * port so that when we subsequently create a server and run it we know
     * we're getting the one we configured.
     */
    let error = client.request(cons_request(bind_port)).await.unwrap_err();
    assert!(error.is_connect());

    /*
     * Now start a server with our configuration and make the request again.
     * This should succeed in terms of making the request.  (The request
     * itself might fail with a 400-level or 500-level response code -- we
     * don't want to depend on too much from the ApiServer here -- but we
     * should have successfully made the request.)
     */
    let tls = Some(TlsConfig {
        cert_file: cert_file.path().to_str().unwrap(),
        key_file: key_file.path().to_str().unwrap(),
    });
    let config = make_config(bind_ip_str, bind_port, tls);
    let server = make_server(&config, &log).start();
    client.request(cons_request(bind_port)).await.unwrap();
    server.close().await.unwrap();

    /*
     * Make another request to make sure it fails now that we've shut down
     * the server.  We need a new client to make sure our client-side connection
     * starts from a clean slate.  (Otherwise, a race during shutdown could
     * cause us to successfully send a request packet, only to have the TCP
     * stack return with ECONNRESET, which gets in the way of what we're trying
     * to test here.)
     */
    let client = make_client();
    let error = client.request(cons_request(bind_port)).await.unwrap_err();
    assert!(error.is_connect());

    /*
     * Start a server on another TCP port and make sure we can reach that
     * one (and NOT the one we just shut down).
     */
    let config = make_config(bind_ip_str, bind_port + 1, tls);
    let server = make_server(&config, &log).start();
    client.request(cons_request(bind_port + 1)).await.unwrap();
    let error = client.request(cons_request(bind_port)).await.unwrap_err();
    assert!(error.is_connect());
    server.close().await.unwrap();

    let error = client.request(cons_request(bind_port)).await.unwrap_err();
    assert!(error.is_connect());
    let error = client.request(cons_request(bind_port + 1)).await.unwrap_err();
    assert!(error.is_connect());

    fs::remove_file(log_path).unwrap();
}
