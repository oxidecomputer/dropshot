// Copyright 2020 Oxide Computer Company
/*!
 * Common facilities for automated testing.
 */

use dropshot::test_util::LogContext;
use dropshot::test_util::TestContext;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingIfExists;
use dropshot::ConfigLoggingLevel;
use slog::o;
use std::io::Write;
use tempfile::NamedTempFile;

pub fn test_setup(
    test_name: &str,
    api: ApiDescription<usize>,
) -> TestContext<usize> {
    /*
     * The IP address to which we bind can be any local IP, but we use
     * 127.0.0.1 because we know it's present, it shouldn't expose this server
     * on any external network, and we don't have to go looking for some other
     * local IP (likely in a platform-specific way).  We specify port 0 to
     * request any available port.  This is important because we may run
     * multiple concurrent tests, so any fixed port could result in spurious
     * failures due to port conflicts.
     */
    let config_dropshot: ConfigDropshot = Default::default();

    let config_logging = ConfigLogging::File {
        level: ConfigLoggingLevel::Debug,
        path: "UNUSED".to_string(),
        if_exists: ConfigLoggingIfExists::Fail,
    };

    let logctx = LogContext::new(test_name, &config_logging);
    let log = logctx.log.new(o!());
    TestContext::new(api, 0 as usize, &config_dropshot, Some(logctx), log)
}

/// Generate a TLS key and a certificate chain containing a certificate for
/// the key, an intermediate cert, and a self-signed root cert.
pub fn generate_tls_key() -> (Vec<rustls::Certificate>, rustls::PrivateKey) {
    let mut root_params = rcgen::CertificateParams::new(vec![]);
    root_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let root_keypair = rcgen::Certificate::from_params(root_params)
        .expect("failed to generate root keys");

    let mut intermediate_params = rcgen::CertificateParams::new(vec![]);
    intermediate_params.is_ca =
        rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let intermediate_keypair =
        rcgen::Certificate::from_params(intermediate_params)
            .expect("failed to generate intermediate keys");

    let end_keypair =
        rcgen::Certificate::from_params(rcgen::CertificateParams::new(vec![
            "localhost".into(),
        ]))
        .expect("failed to generate end-entity keys");

    let root_cert = rustls::Certificate(
        root_keypair.serialize_der().expect("failed to serialize root cert"),
    );
    let intermediate_cert = rustls::Certificate(
        intermediate_keypair
            .serialize_der_with_signer(&root_keypair)
            .expect("failed to serialize intermediate cert"),
    );
    let end_cert = rustls::Certificate(
        end_keypair
            .serialize_der_with_signer(&intermediate_keypair)
            .expect("failed to serialize end-entity cert"),
    );

    let key = rustls::PrivateKey(end_keypair.serialize_private_key_der());
    (vec![end_cert, intermediate_cert, root_cert], key)
}

/// Write keys to a temporary file for passing to the server config
pub fn tls_key_to_file(
    certs: &Vec<rustls::Certificate>,
    key: &rustls::PrivateKey,
) -> (NamedTempFile, NamedTempFile) {
    let mut cert_file =
        NamedTempFile::new().expect("failed to create cert_file");
    for cert in certs {
        let encoded_cert = pem::encode(&pem::Pem {
            tag: "CERTIFICATE".to_string(),
            contents: cert.0.clone(),
        });
        cert_file
            .write(encoded_cert.as_bytes())
            .expect("failed to write cert_file");
    }

    let mut key_file = NamedTempFile::new().expect("failed to create key_file");
    let encoded_key = pem::encode(&pem::Pem {
        tag: "PRIVATE KEY".to_string(),
        contents: key.0.clone(),
    });
    key_file.write(encoded_key.as_bytes()).expect("failed to write key_file");
    (cert_file, key_file)
}
