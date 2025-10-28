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
///             default_request_body_max_bytes = 1024
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
#[serde(
    from = "DeserializedConfigDropshot",
    into = "DeserializedConfigDropshot"
)]
pub struct ConfigDropshot {
    /// IP address and TCP port to which to bind for accepting connections
    pub bind_address: SocketAddr,
    /// maximum allowed size of a request body, defaults to 1024
    pub default_request_body_max_bytes: usize,
    /// Default behavior for HTTP handler functions with respect to clients
    /// disconnecting early.
    pub default_handler_task_mode: HandlerTaskMode,
    /// A list of header names to include as extra properties in the log
    /// messages emitted by the per-request logger.  Each header will, if
    /// present, be included in the output with a "hdr_"-prefixed property name
    /// in lower case that has all hyphens replaced with underscores; e.g.,
    /// "X-Forwarded-For" will be included as "hdr_x_forwarded_for".  No attempt
    /// is made to deal with headers that appear multiple times in a single
    /// request.
    pub log_headers: Vec<String>,
    /// Whether to enable gzip compression for responses when response contents
    /// allow it and clients ask for it through the Accept-Encoding header.
    /// Defaults to true.
    pub compression: bool,
}

/// Enum specifying options for how a Dropshot server should run its handler
/// futures.
///
/// The variants are phrased in terms of how the handler interacts with client
/// disconnection, but they control how the future is run: for
/// `CancelOnDisconnect`, the future is run directly, and it will be dropped
/// (and thus cancelled) if the client disconnects; for `Detach`, handler
/// futures will be `tokio::spawn()`'d, detaching their completion from the
/// behavior of the client.
///
/// If using `CancelOnDisconnect`, one must be careful that all handlers are
/// cancel-safe. If you're unsure, we recommend `Detached`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HandlerTaskMode {
    /// If a client disconnects while the handler is still running, cancel the
    /// future.
    CancelOnDisconnect,

    /// If a client disconnects while the handler is still running, continue
    /// running the handler future to completion (i.e., the handler future is
    /// detached from the client connection).
    Detached,
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
            default_request_body_max_bytes: 1024,
            default_handler_task_mode: HandlerTaskMode::Detached,
            log_headers: Default::default(),
            compression: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
struct DeserializedConfigDropshot {
    bind_address: SocketAddr,
    default_request_body_max_bytes: usize,
    // Previous name for default_request_body_max_bytes, in Dropshot < 0.14.
    // Present only to guide users to the new name.
    #[serde(
        deserialize_with = "deserialize_invalid_request_body_max_bytes",
        skip_serializing
    )]
    request_body_max_bytes: Option<InvalidConfig>,
    default_handler_task_mode: HandlerTaskMode,
    log_headers: Vec<String>,
    compression: bool,
}

impl From<DeserializedConfigDropshot> for ConfigDropshot {
    fn from(v: DeserializedConfigDropshot) -> Self {
        ConfigDropshot {
            bind_address: v.bind_address,
            default_request_body_max_bytes: v.default_request_body_max_bytes,
            default_handler_task_mode: v.default_handler_task_mode,
            log_headers: v.log_headers,
            compression: v.compression,
        }
    }
}

impl From<ConfigDropshot> for DeserializedConfigDropshot {
    fn from(v: ConfigDropshot) -> Self {
        DeserializedConfigDropshot {
            bind_address: v.bind_address,
            default_request_body_max_bytes: v.default_request_body_max_bytes,
            request_body_max_bytes: None,
            default_handler_task_mode: v.default_handler_task_mode,
            log_headers: v.log_headers,
            compression: v.compression,
        }
    }
}

impl Default for DeserializedConfigDropshot {
    fn default() -> Self {
        ConfigDropshot::default().into()
    }
}

/// A marker type to indicate that the configuration is invalid.
///
/// This type can never be constructed, which means that for any valid config,
/// `Option<InvalidConfig>` is always none.
#[derive(Clone, Debug, PartialEq)]
pub enum InvalidConfig {}

// We prefer having a deserialize function over `impl Deserialize for
// InvalidConfig` for two reasons:
//
// 1. This returns an `Option<InvalidConfig>`, not an `InvalidConfig`.
// 2. This way, the deserializer has a custom message associated with it.
fn deserialize_invalid_request_body_max_bytes<'de, D>(
    deserializer: D,
) -> Result<Option<InvalidConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_invalid(
        deserializer,
        "request_body_max_bytes has been renamed to \
         default_request_body_max_bytes",
    )
}

fn deserialize_invalid<'de, D>(
    deserializer: D,
    msg: &'static str,
) -> Result<Option<InvalidConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    struct V {
        msg: &'static str,
    }

    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Option<InvalidConfig>;

        fn expecting(
            &self,
            formatter: &mut std::fmt::Formatter,
        ) -> std::fmt::Result {
            write!(formatter, "the field to be absent ({})", self.msg)
        }

        fn visit_some<D>(self, _: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            Err(D::Error::custom(self.msg))
        }
    }

    deserializer.deserialize_any(V { msg })
}
