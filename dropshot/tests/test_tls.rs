// Copyright 2023 Oxide Computer Company

//! Test cases for TLS support. This validates various behaviors of our TLS
//! mode, including certificate loading and supported modes.

use dropshot::{
    ConfigDropshot, ConfigTls, HandlerTaskMode, HttpResponseOk,
    HttpServerStarter,
};
use slog::{o, Logger};
use std::convert::TryFrom;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub mod common;
use common::create_log_context;

use crate::common::generate_tls_key;

/// See rustls::client::ServerCertVerifier::verify_server_cert for argument
/// meanings
type VerifyCertFn<'a> = Box<
    dyn Fn(
            &rustls::pki_types::CertificateDer,
            &[rustls::pki_types::CertificateDer],
            &rustls::pki_types::ServerName,
            &[u8],
            rustls::pki_types::UnixTime,
        )
            -> Result<rustls::client::danger::ServerCertVerified, rustls::Error>
        + Send
        + Sync
        + 'a,
>;

struct CertificateVerifier<'a>(VerifyCertFn<'a>);

impl<'a> std::fmt::Debug for CertificateVerifier<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CertificateVerifier... with some function?")
    }
}

impl<'a> rustls::client::danger::ServerCertVerifier
    for CertificateVerifier<'a>
{
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        intermediates: &[rustls::pki_types::CertificateDer<'_>],
        server_name: &rustls::pki_types::ServerName<'_>,
        ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        self.0(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
    {
        // In this test environment, we blithely accept that signature matches
        // the message.
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
    {
        // In this test environment, we blithely accept that signature matches
        // the message.
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        // Default algorithm from rcgen
        vec![rustls::SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

fn make_https_client<
    T: rustls::client::danger::ServerCertVerifier + Send + Sync + 'static,
>(
    verifier: Arc<T>,
) -> hyper_util::client::legacy::Client<
    hyper_rustls::HttpsConnector<
        hyper_util::client::legacy::connect::HttpConnector,
    >,
    dropshot::Body,
> {
    let tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_only()
        .enable_http1()
        .build();
    hyper_util::client::legacy::Client::builder(
        hyper_util::rt::TokioExecutor::new(),
    )
    .build(https_connector)
}

fn make_server(
    log: &Logger,
    cert_file: &Path,
    key_file: &Path,
) -> HttpServerStarter<i32> {
    let config = ConfigDropshot {
        bind_address: "127.0.0.1:0".parse().unwrap(),
        request_body_max_bytes: 1024,
        default_handler_task_mode: HandlerTaskMode::CancelOnDisconnect,
        log_headers: Default::default(),
        ..Default::default()
    };
    let config_tls = Some(ConfigTls::AsFile {
        cert_file: cert_file.to_path_buf(),
        key_file: key_file.to_path_buf(),
    });
    HttpServerStarter::new_with_tls(
        &config,
        dropshot::ApiDescription::new(),
        0,
        log,
        config_tls,
    )
    .unwrap()
}

fn make_pki_verifier(
    certs: &Vec<rustls::pki_types::CertificateDer>,
) -> Arc<impl rustls::client::danger::ServerCertVerifier> {
    let mut root_store = rustls::RootCertStore { roots: vec![] };
    root_store.add(certs[certs.len() - 1].clone()).expect("adding root cert");
    rustls::client::WebPkiServerVerifier::builder(Arc::new(root_store))
        .build()
        .unwrap()
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
        .body(dropshot::Body::empty())
        .unwrap();

    let verifier_called = Arc::new(AtomicUsize::new(0));
    let verifier_called_clone = verifier_called.clone();
    let cert_verifier =
        move |end_entity: &rustls::pki_types::CertificateDer,
              intermediates: &[rustls::pki_types::CertificateDer],
              server_name: &rustls::pki_types::ServerName,
              _ocsp_response: &[u8],
              _now: rustls::pki_types::UnixTime|
              -> Result<
            rustls::client::danger::ServerCertVerified,
            rustls::Error,
        > {
            // Tracking to ensure this method was invoked
            verifier_called_clone.fetch_add(1, Ordering::SeqCst);
            // Verify we're seeing the right cert chain from the server
            assert_eq!(*end_entity, certs[0]);
            assert_eq!(intermediates, &certs[1..3]);

            assert_eq!(
                *server_name,
                rustls::pki_types::ServerName::try_from("localhost").unwrap()
            );
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        };
    let client = make_https_client(Arc::new(CertificateVerifier(Box::new(
        cert_verifier,
    ))));
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
        .body(dropshot::Body::empty())
        .unwrap();
    let http_uri: hyper::Uri =
        format!("http://localhost:{}/", port).parse().unwrap();
    let http_request = hyper::Request::builder()
        .method(http::method::Method::GET)
        .uri(&http_uri)
        .body(dropshot::Body::empty())
        .unwrap();

    let https_client = make_https_client(make_pki_verifier(&certs));
    https_client.request(https_request).await.unwrap();

    // Send an HTTP request, it should fail due to parse error, since
    // the server and client are speaking different protocols
    let http_client = hyper_util::client::legacy::Client::builder(
        hyper_util::rt::TokioExecutor::new(),
    )
    .build(hyper_util::client::legacy::connect::HttpConnector::new());
    let _error = http_client.request(http_request).await.unwrap_err();
    // cannot check if it is a "hyper parse error", but would like to if
    // hyper::Error gains the ability in the future

    // Make an HTTPS request again, to make sure the HTTP client didn't
    // interfere with HTTPS request processing
    let https_request = hyper::Request::builder()
        .method(http::method::Method::GET)
        .uri(&https_uri)
        .body(dropshot::Body::empty())
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
    let (certs, key) = generate_tls_key();
    let (cert_file, key_file) = common::tls_key_to_file(&certs, &key);

    let server = make_server(&log, cert_file.path(), key_file.path()).start();
    let port = server.local_addr().port();

    let https_uri: hyper::Uri =
        format!("https://localhost:{}/", port).parse().unwrap();

    let https_request_maker = || {
        hyper::Request::builder()
            .method(http::method::Method::GET)
            .uri(&https_uri)
            .body(dropshot::Body::empty())
            .unwrap()
    };

    // Make an HTTPS request successfully with the original certificate chain.
    let https_client =
        make_https_client(Arc::new(make_cert_verifier(certs.clone())));
    https_client.request(https_request_maker()).await.unwrap();

    // Create a brand new certificate chain.
    let (new_certs, new_key) = generate_tls_key();
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
    let https_client =
        make_https_client(Arc::new(make_cert_verifier(certs.clone())));
    https_client.request(https_request_maker()).await.unwrap_err();

    // New client requests using the new certificate chain should succeed.
    let https_client =
        make_https_client(Arc::new(make_cert_verifier(new_certs.clone())));
    https_client.request(https_request_maker()).await.unwrap();

    server.close().await.unwrap();
    logctx.cleanup_successful();
}

fn make_cert_verifier(
    certs: Vec<rustls::pki_types::CertificateDer>,
) -> CertificateVerifier {
    CertificateVerifier(Box::new(
        move |end_entity: &rustls::pki_types::CertificateDer,
              intermediates: &[rustls::pki_types::CertificateDer],
              server_name: &rustls::pki_types::ServerName,
              _ocsp_response: &[u8],
              _now: rustls::pki_types::UnixTime|
              -> Result<
            rustls::client::danger::ServerCertVerified,
            rustls::Error,
        > {
            // Verify we're seeing the right cert chain from the server
            if *end_entity != certs[0] {
                return Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::BadEncoding,
                ));
            }
            if intermediates != &certs[1..3] {
                return Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::BadEncoding,
                ));
            }
            if *server_name
                != rustls::pki_types::ServerName::try_from("localhost").unwrap()
            {
                return Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::BadEncoding,
                ));
            }
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        },
    ))
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
    let cert_verifier =
        move |_end_entity: &rustls::pki_types::CertificateDer,
              _intermediates: &[rustls::pki_types::CertificateDer],
              _server_name: &rustls::pki_types::ServerName,
              _ocsp_response: &[u8],
              _now: rustls::pki_types::UnixTime|
              -> Result<
            rustls::client::danger::ServerCertVerified,
            rustls::Error,
        > {
            // Tracking to ensure this method was invoked
            verifier_called_clone.fetch_add(1, Ordering::SeqCst);

            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadEncoding,
            ))
        };
    let client = make_https_client(Arc::new(CertificateVerifier(Box::new(
        cert_verifier,
    ))));
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

// The same handler is used for both an HTTP and HTTPS server.
// Make sure that we can distinguish between the two.
// The intended version is determined by a query parameter
// that varies between both tests.
#[dropshot::endpoint {
    method = GET,
    path = "/",
}]
async fn tls_check_handler(
    rqctx: dropshot::RequestContext<usize>,
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
        default_handler_task_mode: HandlerTaskMode::CancelOnDisconnect,
        log_headers: Default::default(),
        ..Default::default()
    };
    let config_tls = Some(ConfigTls::AsFile {
        cert_file: cert_file.path().to_path_buf(),
        key_file: key_file.path().to_path_buf(),
    });
    let mut api = dropshot::ApiDescription::new();
    api.register(tls_check_handler).unwrap();
    let server =
        HttpServerStarter::new_with_tls(&config, api, 0, &log, config_tls)
            .unwrap()
            .start();
    let port = server.local_addr().port();

    let https_client = make_https_client(make_pki_verifier(&certs));

    // Expect request with tls=true to pass with https server
    let https_request = hyper::Request::builder()
        .method(http::method::Method::GET)
        .uri(format!("https://localhost:{}/?tls=true", port))
        .body(dropshot::Body::empty())
        .unwrap();
    let res = https_client.request(https_request).await.unwrap();
    assert_eq!(res.status(), hyper::StatusCode::OK);

    // Expect request with tls=false to fail with https server
    let https_request = hyper::Request::builder()
        .method(http::method::Method::GET)
        .uri(format!("https://localhost:{}/?tls=false", port))
        .body(dropshot::Body::empty())
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
