// Copyright 2020 Oxide Computer Company
//! Common facilities for automated testing.

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
    // The IP address to which we bind can be any local IP, but we use
    // 127.0.0.1 because we know it's present, it shouldn't expose this server
    // on any external network, and we don't have to go looking for some other
    // local IP (likely in a platform-specific way).  We specify port 0 to
    // request any available port.  This is important because we may run
    // multiple concurrent tests, so any fixed port could result in spurious
    // failures due to port conflicts.
    let config_dropshot: ConfigDropshot = Default::default();

    let logctx = create_log_context(test_name);
    let log = logctx.log.new(o!());
    TestContext::new(api, 0_usize, &config_dropshot, Some(logctx), log)
}

pub fn create_log_context(test_name: &str) -> LogContext {
    let log_config = ConfigLogging::File {
        level: ConfigLoggingLevel::Debug,
        path: "UNUSED".into(),
        if_exists: ConfigLoggingIfExists::Fail,
    };
    LogContext::new(test_name, &log_config)
}

pub struct TestCertificateChain {
    root_cert: rustls::Certificate,
    intermediate_cert: rustls::Certificate,
    intermediate_keypair: rcgen::Certificate,
    end_cert: rustls::Certificate,
    end_keypair: rcgen::Certificate,
}

impl TestCertificateChain {
    pub fn new() -> Self {
        let mut root_params = rcgen::CertificateParams::new(vec![]);
        root_params.is_ca =
            rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let root_keypair = rcgen::Certificate::from_params(root_params)
            .expect("failed to generate root keys");

        let mut intermediate_params = rcgen::CertificateParams::new(vec![]);
        intermediate_params.is_ca =
            rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let intermediate_keypair =
            rcgen::Certificate::from_params(intermediate_params)
                .expect("failed to generate intermediate keys");

        let end_keypair = rcgen::Certificate::from_params(
            rcgen::CertificateParams::new(vec!["localhost".into()]),
        )
        .expect("failed to generate end-entity keys");

        let root_cert = rustls::Certificate(
            root_keypair
                .serialize_der()
                .expect("failed to serialize root cert"),
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

        Self {
            root_cert,
            intermediate_cert,
            intermediate_keypair,
            end_cert,
            end_keypair,
        }
    }

    pub fn end_cert_private_key(&self) -> rustls::PrivateKey {
        rustls::PrivateKey(self.end_keypair.serialize_private_key_der())
    }

    pub fn cert_chain(&self) -> Vec<rustls::Certificate> {
        vec![
            self.end_cert.clone(),
            self.intermediate_cert.clone(),
            self.root_cert.clone(),
        ]
    }

    pub fn generate_new_end_cert(&mut self) {
        let end_keypair = rcgen::Certificate::from_params(
            rcgen::CertificateParams::new(vec!["localhost".into()]),
        )
        .expect("failed to generate end-entity keys");
        let end_cert = rustls::Certificate(
            end_keypair
                .serialize_der_with_signer(&self.intermediate_keypair)
                .expect("failed to serialize end-entity cert"),
        );
        self.end_keypair = end_keypair;
        self.end_cert = end_cert;
    }
}

/// Generate a TLS key and a certificate chain containing a certificate for
/// the key, an intermediate cert, and a self-signed root cert.
pub fn generate_tls_key() -> (Vec<rustls::Certificate>, rustls::PrivateKey) {
    let ca = TestCertificateChain::new();
    (ca.cert_chain(), ca.end_cert_private_key())
}

fn make_temp_file() -> std::io::Result<NamedTempFile> {
    tempfile::Builder::new().prefix("dropshot-test-").rand_bytes(5).tempfile()
}

pub fn tls_key_to_buffer(
    certs: &Vec<rustls::Certificate>,
    key: &rustls::PrivateKey,
) -> (Vec<u8>, Vec<u8>) {
    let mut serialized_certs = vec![];
    let mut cert_writer = std::io::BufWriter::new(&mut serialized_certs);
    for cert in certs {
        let encoded_cert = pem::encode(&pem::Pem::new(
            "CERTIFICATE".to_string(),
            cert.0.clone(),
        ));
        cert_writer
            .write_all(encoded_cert.as_bytes())
            .expect("failed to serialize cert");
    }
    drop(cert_writer);

    let mut serialized_key = vec![];
    let mut key_writer = std::io::BufWriter::new(&mut serialized_key);
    let encoded_key =
        pem::encode(&pem::Pem::new("PRIVATE KEY".to_string(), key.0.clone()));
    key_writer
        .write_all(encoded_key.as_bytes())
        .expect("failed to serialize key");
    drop(key_writer);

    (serialized_certs, serialized_key)
}

/// Write keys to a temporary file for passing to the server config
pub fn tls_key_to_file(
    certs: &Vec<rustls::Certificate>,
    key: &rustls::PrivateKey,
) -> (NamedTempFile, NamedTempFile) {
    let mut cert_file = make_temp_file().expect("failed to create cert_file");
    let mut key_file = make_temp_file().expect("failed to create key_file");

    let (certs, key) = tls_key_to_buffer(certs, key);

    cert_file.write_all(certs.as_slice()).expect("Failed to write certs");
    key_file.write_all(key.as_slice()).expect("Failed to write key");

    (cert_file, key_file)
}
