// Copyright 2020 Oxide Computer Company
//! Tests for configuration file.

use dropshot::test_util::read_config;
use dropshot::{ConfigDropshot, ConfigTls};
use dropshot::{HttpServer, HttpServerStarter};
use slog::o;
use slog::Logger;
use std::str::FromStr;
use tempfile::NamedTempFile;

pub mod common;
use common::create_log_context;

// Bad values for "bind_address"

#[test]
fn test_config_bad_bind_address_port_too_small() {
    let error = read_config::<ConfigDropshot>(
        "bad_bind_address_port_too_small",
        "bind_address = \"127.0.0.1:-3\"",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.contains("invalid socket address syntax"));
}

#[test]
fn test_config_bad_bind_address_port_too_large() {
    let error = read_config::<ConfigDropshot>(
        "bad_bind_address_port_too_large",
        "bind_address = \"127.0.0.1:65536\"",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.contains("invalid socket address syntax"));
}

#[test]
fn test_config_bad_bind_address_garbage() {
    let error = read_config::<ConfigDropshot>(
        "bad_bind_address_garbage",
        "bind_address = \"garbage\"",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.contains("invalid socket address syntax"));
}

// Bad values for "request_body_max_bytes"

#[test]
fn test_config_bad_request_body_max_bytes_negative() {
    let error = read_config::<ConfigDropshot>(
        "bad_request_body_max_bytes_negative",
        "request_body_max_bytes = -1024",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.contains("invalid value: integer"));
}

#[test]
fn test_config_bad_request_body_max_bytes_too_large() {
    let error = read_config::<ConfigDropshot>(
        "bad_request_body_max_bytes_too_large",
        "request_body_max_bytes = 999999999999999999999999999999",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.starts_with(""));
}

// Bad values for "key_file"

#[test]
fn test_config_bad_key_file_garbage() {
    let error = read_config::<ConfigDropshot>(
        "bad_key_file_garbage",
        "[tls]\ntype = 'AsFile'\ncert_file = ''\nkey_file = 23",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.contains("invalid type: integer"));
}

// Bad values for "cert_file"

#[test]
fn test_config_bad_cert_file_garbage() {
    let error = read_config::<ConfigDropshot>(
        "bad_cert_file_garbage",
        "[tls]\ntype = 'AsFile'\ncert_file = 23\nkey_file=''",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.contains("invalid type: integer"));
}

// Bad values for "tls"

#[test]
fn test_config_bad_tls_garbage() {
    let error = read_config::<ConfigDropshot>("bad_tls_garbage", "tls = 23")
        .unwrap_err()
        .to_string();
    println!("found error: {}", error);
    assert!(error.contains("invalid type: integer"));
}

#[test]
fn test_config_bad_tls_incomplete() {
    let error = read_config::<ConfigDropshot>(
        "bad_tls_incomplete",
        "[tls]\ntype = 'AsFile'\ncert_file = ''",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.contains("missing field `key_file`"));

    let error = read_config::<ConfigDropshot>(
        "bad_tls_incomplete",
        "[tls]\ntype = 'AsFile'\nkey_file = ''",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.contains("missing field `cert_file`"));
}

fn make_server(
    config: &ConfigDropshot,
    log: &Logger,
) -> HttpServerStarter<i32> {
    HttpServerStarter::new(&config, dropshot::ApiDescription::new(), 0, log)
        .unwrap()
}

fn make_config(
    bind_ip_str: &str,
    bind_port: u16,
    tls: Option<ConfigTls>,
) -> ConfigDropshot {
    ConfigDropshot {
        bind_address: std::net::SocketAddr::new(
            std::net::IpAddr::from_str(bind_ip_str).unwrap(),
            bind_port,
        ),
        request_body_max_bytes: 1024,
        tls,
    }
}

// Trait for abstracting out test case specific properties from the common bind
// test logic
trait TestConfigBindServer<C>
where
    C: hyper::client::connect::Connect + Clone + Send + Sync + 'static,
{
    fn make_client(&self) -> hyper::Client<C>;
    fn make_server(&self, bind_port: u16) -> HttpServer<i32>;
    fn make_uri(&self, bind_port: u16) -> hyper::Uri;

    fn log(&self) -> &slog::Logger;
}

// Validate that we can create a server with the given configuration and that
// it binds to ports as expected.
async fn test_config_bind_server<C, T>(test_config: T, bind_port: u16)
where
    C: hyper::client::connect::Connect + Clone + Send + Sync + 'static,
    T: TestConfigBindServer<C>,
{
    let client = test_config.make_client();

    // Make sure there is not currently a server running on our expected
    // port so that when we subsequently create a server and run it we know
    // we're getting the one we configured.
    let error = client.get(test_config.make_uri(bind_port)).await.unwrap_err();
    assert!(error.is_connect());

    // Now start a server with our configuration and make the request again.
    // This should succeed in terms of making the request.  (The request
    // itself might fail with a 400-level or 500-level response code -- we
    // don't want to depend on too much from the ApiServer here -- but we
    // should have successfully made the request.)
    let server = test_config.make_server(bind_port);
    client.get(test_config.make_uri(bind_port)).await.unwrap();
    server.close().await.unwrap();

    // Make another request to make sure it fails now that we've shut down
    // the server.  We need a new client to make sure our client-side connection
    // starts from a clean slate.  (Otherwise, a race during shutdown could
    // cause us to successfully send a request packet, only to have the TCP
    // stack return with ECONNRESET, which gets in the way of what we're trying
    // to test here.)
    let client = test_config.make_client();
    let error = client.get(test_config.make_uri(bind_port)).await.unwrap_err();
    assert!(error.is_connect());

    // Start a server on another TCP port and make sure we can reach that
    // one (and NOT the one we just shut down).
    let server = test_config.make_server(bind_port + 1);
    client.get(test_config.make_uri(bind_port + 1)).await.unwrap();
    let error = client.get(test_config.make_uri(bind_port)).await.unwrap_err();
    assert!(error.is_connect());
    server.close().await.unwrap();

    let error = client.get(test_config.make_uri(bind_port)).await.unwrap_err();
    assert!(error.is_connect());
    let error =
        client.get(test_config.make_uri(bind_port + 1)).await.unwrap_err();
    assert!(error.is_connect());
}

#[tokio::test]
async fn test_config_bind_address_http() {
    let logctx = create_log_context("config_bind_address_http");
    let log = logctx.log.new(o!());

    struct ConfigBindServerHttp {
        log: slog::Logger,
    }
    impl TestConfigBindServer<hyper::client::connect::HttpConnector>
        for ConfigBindServerHttp
    {
        fn make_client(
            &self,
        ) -> hyper::Client<hyper::client::connect::HttpConnector> {
            hyper::Client::new()
        }

        fn make_uri(&self, bind_port: u16) -> hyper::Uri {
            format!("http://localhost:{}/", bind_port).parse().unwrap()
        }
        fn make_server(&self, bind_port: u16) -> HttpServer<i32> {
            let tls = None;
            let config = make_config("127.0.0.1", bind_port, tls);
            make_server(&config, &self.log).start()
        }

        fn log(&self) -> &slog::Logger {
            &self.log
        }
    }

    let test_config = ConfigBindServerHttp { log };
    let bind_port = 12215;
    test_config_bind_server::<_, ConfigBindServerHttp>(test_config, bind_port)
        .await;

    logctx.cleanup_successful();
}

#[tokio::test]
async fn test_config_bind_address_https() {
    struct ConfigBindServerHttps {
        log: slog::Logger,
        certs: Vec<rustls::Certificate>,
        cert_file: NamedTempFile,
        key_file: NamedTempFile,
    }

    impl
        TestConfigBindServer<
            hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>,
        > for ConfigBindServerHttps
    {
        fn make_client(
            &self,
        ) -> hyper::Client<
            hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>,
        > {
            // Configure TLS to trust the self-signed cert
            let mut root_store = rustls::RootCertStore { roots: vec![] };
            root_store
                .add(&self.certs[self.certs.len() - 1])
                .expect("adding root cert");

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
        }

        fn make_uri(&self, bind_port: u16) -> hyper::Uri {
            format!("https://localhost:{}/", bind_port).parse().unwrap()
        }

        fn make_server(&self, bind_port: u16) -> HttpServer<i32> {
            let tls = Some(ConfigTls::AsFile {
                cert_file: self.cert_file.path().to_path_buf(),
                key_file: self.key_file.path().to_path_buf(),
            });
            let config = make_config("127.0.0.1", bind_port, tls);
            make_server(&config, &self.log).start()
        }

        fn log(&self) -> &Logger {
            &self.log
        }
    }

    let logctx = create_log_context("config_bind_address_https");
    let log = logctx.log.new(o!());

    // Generate key for the server
    let (certs, key) = common::generate_tls_key();
    let (cert_file, key_file) = common::tls_key_to_file(&certs, &key);
    let test_config = ConfigBindServerHttps { log, certs, cert_file, key_file };

    // This must be different than the bind_port used in the http test.
    let bind_port = 12217;
    test_config_bind_server::<_, ConfigBindServerHttps>(test_config, bind_port)
        .await;

    logctx.cleanup_successful();
}

#[tokio::test]
async fn test_config_bind_address_https_buffer() {
    struct ConfigBindServerHttps {
        log: slog::Logger,
        certs: Vec<rustls::Certificate>,
        serialized_certs: Vec<u8>,
        serialized_key: Vec<u8>,
    }

    impl
        TestConfigBindServer<
            hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>,
        > for ConfigBindServerHttps
    {
        fn make_client(
            &self,
        ) -> hyper::Client<
            hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>,
        > {
            // Configure TLS to trust the self-signed cert
            let mut root_store = rustls::RootCertStore { roots: vec![] };
            root_store
                .add(&self.certs[self.certs.len() - 1])
                .expect("adding root cert");

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
        }

        fn make_uri(&self, bind_port: u16) -> hyper::Uri {
            format!("https://localhost:{}/", bind_port).parse().unwrap()
        }

        fn make_server(&self, bind_port: u16) -> HttpServer<i32> {
            let tls = Some(ConfigTls::AsBytes {
                certs: self.serialized_certs.clone(),
                key: self.serialized_key.clone(),
            });
            let config = make_config("127.0.0.1", bind_port, tls);
            make_server(&config, &self.log).start()
        }

        fn log(&self) -> &Logger {
            &self.log
        }
    }

    let logctx = create_log_context("config_bind_address_https_buffer");
    let log = logctx.log.new(o!());

    // Generate key for the server
    let (certs, key) = common::generate_tls_key();
    let (serialized_certs, serialized_key) =
        common::tls_key_to_buffer(&certs, &key);
    let test_config =
        ConfigBindServerHttps { log, certs, serialized_certs, serialized_key };

    // This must be different than the bind_port used in the http test.
    let bind_port = 12219;
    test_config_bind_server::<_, ConfigBindServerHttps>(test_config, bind_port)
        .await;

    logctx.cleanup_successful();
}
