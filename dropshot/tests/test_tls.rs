// Copyright 2022 Oxide Computer Company
/*!
 * Test cases for TLS support. This validates various behaviors of our TLS mode,
 * including certificate loading and supported modes.
 */

use dropshot::{ConfigDropshot, ConfigTls, HttpResponseOk, HttpServerStarter};
use slog::{o, Logger};
use std::convert::TryFrom;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

pub mod common;
use common::create_log_context;

/// See rustls::client::ServerCertVerifier::verify_server_cert for argument
/// meanings
type VerifyCertFn = Box<
    dyn Fn(
            &rustls::Certificate,
            &[rustls::Certificate],
            &rustls::ServerName,
            &mut dyn Iterator<Item = &[u8]>,
            &[u8],
            SystemTime,
        ) -> Result<rustls::client::ServerCertVerified, rustls::Error>
        + Send
        + Sync,
>;

struct CertificateVerifier(VerifyCertFn);

impl rustls::client::ServerCertVerifier for CertificateVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::Certificate,
        intermediates: &[rustls::Certificate],
        server_name: &rustls::ServerName,
        scts: &mut dyn Iterator<Item = &[u8]>,
        ocsp_response: &[u8],
        now: SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        self.0(end_entity, intermediates, server_name, scts, ocsp_response, now)
    }
}

fn make_https_client<
    T: rustls::client::ServerCertVerifier + Send + Sync + 'static,
>(
    verifier: T,
) -> hyper::Client<
    hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>,
> {
    let tls_config = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_only()
        .enable_http1()
        .build();
    hyper::Client::builder().build(https_connector)
}

fn make_server(
    log: &Logger,
    cert_file: &Path,
    key_file: &Path,
) -> HttpServerStarter<i32> {
    let config = ConfigDropshot {
        bind_address: "127.0.0.1:0".parse().unwrap(),
        request_body_max_bytes: 1024,
        tls: Some(ConfigTls::AsFile {
            cert_file: cert_file.to_path_buf(),
            key_file: key_file.to_path_buf(),
        }),
    };
    HttpServerStarter::new(&config, dropshot::ApiDescription::new(), 0, log)
        .unwrap()
}

fn make_pki_verifier(
    certs: &Vec<rustls::Certificate>,
) -> impl rustls::client::ServerCertVerifier {
    let mut root_store = rustls::RootCertStore { roots: vec![] };
    root_store.add(&certs[certs.len() - 1]).expect("adding root cert");
    rustls::client::WebPkiVerifier::new(root_store, None)
}

#[tokio::test]
async fn test_tls_certificate_loading() {
    let logctx = create_log_context("test_tls_certificate_loading");
    let log = logctx.log.new(o!());

    // Generate key for the server
    let (certs, key) = common::generate_tls_key();
    let (cert_file, key_file) = common::tls_key_to_file(&certs, &key);

    let server = make_server(&log, cert_file.path(), key_file.path()).start();
    let port = server.local_addr().port();

    let uri: hyper::Uri =
        format!("https://localhost:{}/", port).parse().unwrap();
    let request = hyper::Request::builder()
        .method(http::method::Method::GET)
        .uri(&uri)
        .body(hyper::Body::empty())
        .unwrap();

    let verifier_called = Arc::new(AtomicUsize::new(0));
    let verifier_called_clone = verifier_called.clone();
    let cert_verifier = move |end_entity: &rustls::Certificate,
                              intermediates: &[rustls::Certificate],
                              server_name: &rustls::ServerName,
                              _scts: &mut dyn Iterator<Item = &[u8]>,
                              _ocsp_response: &[u8],
                              _now: SystemTime|
          -> Result<
        rustls::client::ServerCertVerified,
        rustls::Error,
    > {
        // Tracking to ensure this method was invoked
        verifier_called_clone.fetch_add(1, Ordering::SeqCst);
        // Verify we're seeing the right cert chain from the server
        assert_eq!(*end_entity, certs[0]);
        assert_eq!(intermediates, &certs[1..3]);

        assert_eq!(
            *server_name,
            rustls::ServerName::try_from("localhost").unwrap()
        );
        Ok(rustls::client::ServerCertVerified::assertion())
    };
    let client =
        make_https_client(CertificateVerifier(Box::new(cert_verifier)));
    client.request(request).await.unwrap();
    assert_eq!(verifier_called.load(Ordering::SeqCst), 1);

    server.close().await.unwrap();

    logctx.cleanup_successful();
}

#[tokio::test]
async fn test_tls_only() {
    let logctx = create_log_context("test_tls_only");
    let log = logctx.log.new(o!());

    // Generate key for the server
    let (certs, key) = common::generate_tls_key();
    let (cert_file, key_file) = common::tls_key_to_file(&certs, &key);

    let server = make_server(&log, cert_file.path(), key_file.path()).start();
    let port = server.local_addr().port();

    let https_uri: hyper::Uri =
        format!("https://localhost:{}/", port).parse().unwrap();
    let https_request = hyper::Request::builder()
        .method(http::method::Method::GET)
        .uri(&https_uri)
        .body(hyper::Body::empty())
        .unwrap();
    let http_uri: hyper::Uri =
        format!("http://localhost:{}/", port).parse().unwrap();
    let http_request = hyper::Request::builder()
        .method(http::method::Method::GET)
        .uri(&http_uri)
        .body(hyper::Body::empty())
        .unwrap();

    let https_client = make_https_client(make_pki_verifier(&certs));
    https_client.request(https_request).await.unwrap();

    // Send an HTTP request, it should fail due to incomplete message, since
    // the server and client are speaking different protocols
    let http_client = hyper::Client::builder().build_http();
    let error = http_client.request(http_request).await.unwrap_err();
    assert!(error.is_incomplete_message());

    // Make an HTTPS request again, to make sure the HTTP client didn't
    // interfere with HTTPS request processing
    let https_request = hyper::Request::builder()
        .method(http::method::Method::GET)
        .uri(&https_uri)
        .body(hyper::Body::empty())
        .unwrap();
    https_client.request(https_request).await.unwrap();

    server.close().await.unwrap();

    logctx.cleanup_successful();
}

#[tokio::test]
async fn test_tls_refresh_certificates() {
    let logctx = create_log_context("test_tls_refresh_certificates");
    let log = logctx.log.new(o!());

    // Generate key for the server
    let ca = common::TestCertificateChain::new();
    let (certs, key) = (ca.cert_chain(), ca.end_cert_private_key());
    let (cert_file, key_file) = common::tls_key_to_file(&certs, &key);

    let server = make_server(&log, cert_file.path(), key_file.path()).start();
    let port = server.local_addr().port();

    let https_uri: hyper::Uri =
        format!("https://localhost:{}/", port).parse().unwrap();

    let https_request_maker = || {
        hyper::Request::builder()
            .method(http::method::Method::GET)
            .uri(&https_uri)
            .body(hyper::Body::empty())
            .unwrap()
    };

    let make_cert_verifier = |certs: Vec<rustls::Certificate>| {
        CertificateVerifier(Box::new(
            move |end_entity: &rustls::Certificate,
                  intermediates: &[rustls::Certificate],
                  server_name: &rustls::ServerName,
                  _scts: &mut dyn Iterator<Item = &[u8]>,
                  _ocsp_response: &[u8],
                  _now: SystemTime|
                  -> Result<
                rustls::client::ServerCertVerified,
                rustls::Error,
            > {
                // Verify we're seeing the right cert chain from the server
                if *end_entity != certs[0] {
                    return Err(rustls::Error::InvalidCertificateData(
                        "Invalid end cert".to_string(),
                    ));
                }
                if intermediates != &certs[1..3] {
                    return Err(rustls::Error::InvalidCertificateData(
                        "Invalid intermediates".to_string(),
                    ));
                }
                if *server_name
                    != rustls::ServerName::try_from("localhost").unwrap()
                {
                    return Err(rustls::Error::InvalidCertificateData(
                        "Invalid name".to_string(),
                    ));
                }
                Ok(rustls::client::ServerCertVerified::assertion())
            },
        ))
    };

    // Make an HTTPS request successfully with the original certificate chain.
    let https_client = make_https_client(make_cert_verifier(certs.clone()));
    https_client.request(https_request_maker()).await.unwrap();

    // Create a brand new certificate chain.
    let ca = common::TestCertificateChain::new();
    let (new_certs, new_key) = (ca.cert_chain(), ca.end_cert_private_key());
    let (cert_file, key_file) = common::tls_key_to_file(&new_certs, &new_key);
    let config = ConfigTls::AsFile {
        cert_file: cert_file.path().to_path_buf(),
        key_file: key_file.path().to_path_buf(),
    };

    // Refresh the server to use the new certificate chain.
    server.refresh_tls(&config).await.unwrap();

    // Client requests which have already been accepted should succeed.
    https_client.request(https_request_maker()).await.unwrap();

    // New client requests using the old certificate chain should fail.
    let https_client = make_https_client(make_cert_verifier(certs.clone()));
    https_client.request(https_request_maker()).await.unwrap_err();

    // New client requests using the new certificate chain should succeed.
    let https_client = make_https_client(make_cert_verifier(new_certs.clone()));
    https_client.request(https_request_maker()).await.unwrap();

    server.close().await.unwrap();
    logctx.cleanup_successful();
}

#[tokio::test]
async fn test_tls_aborted_negotiation() {
    let logctx = create_log_context("test_tls_aborted_negotiation");
    let log = logctx.log.new(o!());

    // Generate key for the server
    let (certs, key) = common::generate_tls_key();
    let (cert_file, key_file) = common::tls_key_to_file(&certs, &key);

    let server = make_server(&log, cert_file.path(), key_file.path()).start();
    let port = server.local_addr().port();

    let uri: hyper::Uri =
        format!("https://localhost:{}/", port).parse().unwrap();

    // Configure a client that will fail to verify the server's cert, therefore
    // aborting the connection partway through negotitation
    let verifier_called = Arc::new(AtomicUsize::new(0));
    let verifier_called_clone = verifier_called.clone();
    let cert_verifier = move |_end_entity: &rustls::Certificate,
                              _intermediates: &[rustls::Certificate],
                              _server_name: &rustls::ServerName,
                              _scts: &mut dyn Iterator<Item = &[u8]>,
                              _ocsp_response: &[u8],
                              _now: SystemTime|
          -> Result<
        rustls::client::ServerCertVerified,
        rustls::Error,
    > {
        // Tracking to ensure this method was invoked
        verifier_called_clone.fetch_add(1, Ordering::SeqCst);

        Err(rustls::Error::InvalidCertificateData("test error".to_string()))
    };
    let client =
        make_https_client(CertificateVerifier(Box::new(cert_verifier)));
    client.get(uri.clone()).await.unwrap_err();
    assert_eq!(verifier_called.load(Ordering::SeqCst), 1);

    // Send a valid request and make sure it still works
    let client = make_https_client(make_pki_verifier(&certs));
    client.get(uri.clone()).await.unwrap();

    server.close().await.unwrap();

    logctx.cleanup_successful();
}

#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct TlsCheckArgs {
    tls: bool,
}

/*
 * The same handler is used for both an HTTP and HTTPS server.
 * Make sure that we can distinguish between the two.
 * The intended version is determined by a query parameter
 * that varies between both tests.
 */
#[dropshot::endpoint {
    method = GET,
    path = "/",
}]
async fn tls_check_handler(
    rqctx: Arc<dropshot::RequestContext<usize>>,
    query: dropshot::Query<TlsCheckArgs>,
) -> Result<HttpResponseOk<()>, dropshot::HttpError> {
    if rqctx.server.using_tls() != query.into_inner().tls {
        return Err(dropshot::HttpError::for_bad_request(
            None,
            "mismatch between expected and actual tls state".to_string(),
        ));
    }
    Ok(HttpResponseOk(()))
}

#[tokio::test]
async fn test_server_is_https() {
    let logctx = create_log_context("test_server_is_https");
    let log = logctx.log.new(o!());

    // Generate key for the server
    let (certs, key) = common::generate_tls_key();
    let (cert_file, key_file) = common::tls_key_to_file(&certs, &key);

    let config = ConfigDropshot {
        bind_address: "127.0.0.1:0".parse().unwrap(),
        request_body_max_bytes: 1024,
        tls: Some(ConfigTls::AsFile {
            cert_file: cert_file.path().to_path_buf(),
            key_file: key_file.path().to_path_buf(),
        }),
    };
    let mut api = dropshot::ApiDescription::new();
    api.register(tls_check_handler).unwrap();
    let server = HttpServerStarter::new(&config, api, 0, &log).unwrap().start();
    let port = server.local_addr().port();

    let https_client = make_https_client(make_pki_verifier(&certs));

    // Expect request with tls=true to pass with https server
    let https_request = hyper::Request::builder()
        .method(http::method::Method::GET)
        .uri(format!("https://localhost:{}/?tls=true", port))
        .body(hyper::Body::empty())
        .unwrap();
    let res = https_client.request(https_request).await.unwrap();
    assert_eq!(res.status(), hyper::StatusCode::OK);

    // Expect request with tls=false to fail with https server
    let https_request = hyper::Request::builder()
        .method(http::method::Method::GET)
        .uri(format!("https://localhost:{}/?tls=false", port))
        .body(hyper::Body::empty())
        .unwrap();
    let res = https_client.request(https_request).await.unwrap();
    assert_eq!(res.status(), hyper::StatusCode::BAD_REQUEST);

    server.close().await.unwrap();

    logctx.cleanup_successful();
}

#[tokio::test]
async fn test_server_is_http() {
    let mut api = dropshot::ApiDescription::new();
    api.register(tls_check_handler).unwrap();

    let testctx = common::test_setup("test_server_is_http", api);

    // Expect request with tls=false to pass with plain http server
    testctx
        .client_testctx
        .make_request(
            hyper::Method::GET,
            "/?tls=false",
            None as Option<()>,
            hyper::StatusCode::OK,
        )
        .await
        .expect("expected success");

    // Expect request with tls=true to fail with plain http server
    testctx
        .client_testctx
        .make_request(
            hyper::Method::GET,
            "/?tls=true",
            None as Option<()>,
            hyper::StatusCode::BAD_REQUEST,
        )
        .await
        .expect_err("expected failure");
}
