// Copyright 2020 Oxide Computer Company
//! Configuration for Dropshot

use serde::Deserialize;
use serde::Serialize;
use std::net::SocketAddr;
use std::path::PathBuf;

/// Raw [`rustls::ServerConfig`] TLS configuration for use with
/// [`ConfigTls::Dynamic`]
pub type RawTlsConfig = rustls::ServerConfig;

/// Configuration for a Dropshot server.
///
/// This type implements [`serde::Deserialize`] and [`serde::Serialize`] and it
/// can be composed with the consumer's configuration (whatever format that's
/// in).  For example, consumers could define a custom `MyAppConfig` for an app
/// that contains a Dropshot server:
///
/// ```
/// use dropshot::ConfigDropshot;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct MyAppConfig {
///     http_api_server: ConfigDropshot,
///     /* ... (other app-specific config) */
/// }
///
/// fn main() -> Result<(), String> {
///     let my_config: MyAppConfig = toml::from_str(
///         r##"
///             [http_api_server]
///             bind_address = "127.0.0.1:12345"
///             request_body_max_bytes = 1024
///             ## ... (other app-specific config)
///         "##
///     ).map_err(|error| format!("parsing config: {}", error))?;
///
///     let dropshot_config: &ConfigDropshot = &my_config.http_api_server;
///     /* ... (use the config to create a server) */
///     Ok(())
/// }
/// ```
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ConfigDropshot {
    /// IP address and TCP port to which to bind for accepting connections
    pub bind_address: SocketAddr,
    /// maximum allowed size of a request body, defaults to 1024
    pub request_body_max_bytes: usize,
}

#[derive(Clone, Debug)]
pub enum ConfigTls {
    /// The server will read the certificate chain and private key from the
    /// specified file.
    AsFile {
        /// Path to a PEM file containing a certificate chain for the
        ///  server to identify itself with. The first certificate is the
        ///  end-entity certificate, and the remaining are intermediate
        ///  certificates on the way to a trusted CA.
        cert_file: PathBuf,
        /// Path to a PEM-encoded PKCS #8 file containing the private key the
        ///  server will use.
        key_file: PathBuf,
    },
    /// The server will use the certificate chain and private key from the
    /// specified bytes.
    AsBytes { certs: Vec<u8>, key: Vec<u8> },
    /// The dropshot consumer will provide TLS configuration dynamically (that
    /// is not expressible in a static config file)
    Dynamic(RawTlsConfig),
}

impl Default for ConfigDropshot {
    fn default() -> Self {
        ConfigDropshot {
            bind_address: "127.0.0.1:0".parse().unwrap(),
            request_body_max_bytes: 1024,
        }
    }
}
