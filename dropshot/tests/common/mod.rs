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

pub fn generate_tls_key() -> (rustls::Certificate, rustls::PrivateKey) {
    let keypair =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("failed to generate keys");
    let cert = rustls::Certificate(
        keypair.serialize_der().expect("failed to serialize cert"),
    );
    let key = rustls::PrivateKey(keypair.serialize_private_key_der());
    (cert, key)
}

/// Write keys to a temporary file for passing to the server config
pub fn tls_key_to_file(
    cert: &rustls::Certificate,
    key: &rustls::PrivateKey,
) -> (NamedTempFile, NamedTempFile) {
    let encoded_cert = pem::encode(&pem::Pem {
        tag: "CERTIFICATE".to_string(),
        contents: cert.0.clone(),
    });
    let encoded_key = pem::encode(&pem::Pem {
        tag: "PRIVATE KEY".to_string(),
        contents: key.0.clone(),
    });

    let mut cert_file =
        NamedTempFile::new().expect("failed to create cert_file");
    cert_file
        .write(encoded_cert.as_bytes())
        .expect("failed to write cert_file");
    let mut key_file = NamedTempFile::new().expect("failed to create key_file");
    key_file.write(encoded_key.as_bytes()).expect("failed to write key_file");
    (cert_file, key_file)
}
