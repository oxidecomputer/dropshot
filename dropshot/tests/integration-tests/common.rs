// Copyright 2025 Oxide Computer Company

//! Common facilities for automated testing.

use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingIfExists;
use dropshot::ConfigLoggingLevel;
use dropshot::HandlerTaskMode;
use dropshot::ServerContext;
use dropshot::test_util::LogContext;
use dropshot::test_util::TestContext;
use slog::o;
use std::io::Write;
use tempfile::NamedTempFile;

pub fn test_setup(
    test_name: &str,
    api: ApiDescription<usize>,
) -> TestContext<usize> {
    test_setup_with_context(test_name, api, 0_usize, HandlerTaskMode::Detached)
}

pub fn test_setup_with_context<Context: ServerContext>(
    test_name: &str,
    api: ApiDescription<Context>,
    ctx: Context,
    default_handler_task_mode: HandlerTaskMode,
) -> TestContext<Context> {
    // The IP address to which we bind can be any local IP, but we use
    // 127.0.0.1 because we know it's present, it shouldn't expose this server
    // on any external network, and we don't have to go looking for some other
    // local IP (likely in a platform-specific way).  We specify port 0 to
    // request any available port.  This is important because we may run
    // multiple concurrent tests, so any fixed port could result in spurious
    // failures due to port conflicts.
    let config_dropshot: ConfigDropshot =
        ConfigDropshot { default_handler_task_mode, ..Default::default() };

    let logctx = create_log_context(test_name);
    let log = logctx.log.new(o!());
    TestContext::new(api, ctx, &config_dropshot, Some(logctx), log)
}

pub fn create_log_context(test_name: &str) -> LogContext {
    let log_config = ConfigLogging::File {
        level: ConfigLoggingLevel::Debug,
        path: "UNUSED".into(),
        if_exists: ConfigLoggingIfExists::Fail,
    };
    LogContext::new(test_name, &log_config)
}

struct TestCertificateChain {
    root_cert: rcgen::Certificate,
    intermediate_cert: rcgen::Certificate,
    end_keypair: rcgen::KeyPair,
    end_cert: rcgen::Certificate,
}

impl TestCertificateChain {
    fn new() -> Self {
        let root_keypair =
            rcgen::KeyPair::generate().expect("root keypair generation failed");
        let mut root_params =
            rcgen::CertificateParams::new(vec![]).expect("invalid root params");
        root_params.is_ca =
            rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);

        let root_cert = root_params
            .self_signed(&root_keypair)
            .expect("failed to generate root keys");

        let intermediate_keypair = rcgen::KeyPair::generate()
            .expect("intermediate keypair generation failed");
        let mut intermediate_params = rcgen::CertificateParams::new(vec![])
            .expect("invalid intermediate params");
        intermediate_params.is_ca =
            rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let intermediate_cert = intermediate_params
            .signed_by(
                &intermediate_keypair,
                &rcgen::Issuer::new(root_params, root_keypair),
            )
            .expect("failed to sign intermediate cert");

        let end_keypair =
            rcgen::KeyPair::generate().expect("end keypair generation failed");
        let end_params =
            rcgen::CertificateParams::new(vec!["localhost".into()])
                .expect("invalid end params");
        let end_cert = end_params
            .signed_by(
                &end_keypair,
                &rcgen::Issuer::new(intermediate_params, intermediate_keypair),
            )
            .expect("failed to sign end cert");

        Self { root_cert, intermediate_cert, end_keypair, end_cert }
    }

    pub fn end_cert_private_key(&self) -> rustls::pki_types::PrivateKeyDer<'_> {
        rustls::pki_types::PrivateKeyDer::from(
            rustls::pki_types::PrivatePkcs8KeyDer::from(
                self.end_keypair.serialize_der(),
            ),
        )
    }

    pub fn cert_chain(&self) -> Vec<rustls::pki_types::CertificateDer<'_>> {
        vec![
            self.end_cert.der().clone(),
            self.intermediate_cert.der().clone(),
            self.root_cert.der().clone(),
        ]
    }
}

/// Generate a TLS key and a certificate chain containing a certificate for
/// the key, an intermediate cert, and a self-signed root cert.
pub fn generate_tls_key<'a>() -> (
    Vec<rustls::pki_types::CertificateDer<'a>>,
    rustls::pki_types::PrivateKeyDer<'a>,
) {
    let ca = TestCertificateChain::new();
    let cert_chain =
        ca.cert_chain().into_iter().map(|x| x.into_owned()).collect();
    (cert_chain, ca.end_cert_private_key().clone_key())
}

fn make_temp_file() -> std::io::Result<NamedTempFile> {
    tempfile::Builder::new().prefix("dropshot-test-").rand_bytes(5).tempfile()
}

pub fn tls_key_to_buffer<'a>(
    certs: &Vec<rustls::pki_types::CertificateDer<'a>>,
    key: &rustls::pki_types::PrivateKeyDer<'a>,
) -> (Vec<u8>, Vec<u8>) {
    let mut serialized_certs = vec![];
    let mut cert_writer = std::io::BufWriter::new(&mut serialized_certs);
    for cert in certs {
        let encoded_cert = pem::encode(&pem::Pem::new(
            "CERTIFICATE".to_string(),
            cert.to_vec(),
        ));
        cert_writer
            .write_all(encoded_cert.as_bytes())
            .expect("failed to serialize cert");
    }
    drop(cert_writer);

    let mut serialized_key = vec![];
    let mut key_writer = std::io::BufWriter::new(&mut serialized_key);
    let encoded_key = pem::encode(&pem::Pem::new(
        "PRIVATE KEY".to_string(),
        key.secret_der().to_vec(),
    ));
    key_writer
        .write_all(encoded_key.as_bytes())
        .expect("failed to serialize key");
    drop(key_writer);

    (serialized_certs, serialized_key)
}

/// Write keys to a temporary file for passing to the server config
pub fn tls_key_to_file<'a>(
    certs: &Vec<rustls::pki_types::CertificateDer<'a>>,
    key: &rustls::pki_types::PrivateKeyDer<'a>,
) -> (NamedTempFile, NamedTempFile) {
    let mut cert_file = make_temp_file().expect("failed to create cert_file");
    let mut key_file = make_temp_file().expect("failed to create key_file");

    let (certs, key) = tls_key_to_buffer(certs, key);

    cert_file.write_all(certs.as_slice()).expect("Failed to write certs");
    key_file.write_all(key.as_slice()).expect("Failed to write key");

    (cert_file, key_file)
}
