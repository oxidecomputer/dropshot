// Copyright 2022 Oxide Computer Company
/*!
 * Test cases for TLS support. This validates various behaviors of our TLS mode,
 * including certificate loading and supported modes.
 */

use dropshot::ConfigDropshot;
use dropshot::HttpServerStarter;
use slog::Logger;
use std::convert::TryFrom;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

pub mod common;

fn create_log(test_name: &str) -> slog::Logger {
    let log_path = dropshot::test_util::log_file_for_test(test_name)
        .as_path()
        .display()
        .to_string();
    eprintln!("log file: {}", log_path);

    let log_config = dropshot::ConfigLogging::File {
        level: dropshot::ConfigLoggingLevel::Debug,
        path: log_path.clone(),
        if_exists: dropshot::ConfigLoggingIfExists::Append,
    };

    log_config.to_logger(test_name).unwrap()
}

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
        https: true,
        cert_file: cert_file.to_path_buf(),
        key_file: key_file.to_path_buf(),
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
    let log = create_log("test_tls_certificate_loading");

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
}

#[tokio::test]
async fn test_tls_only() {
    let log = create_log("test_tls_only");

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
}
