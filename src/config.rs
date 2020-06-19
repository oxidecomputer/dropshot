/*!
 * Configuration for Dropshot
 */

use serde::Deserialize;
use serde::Serialize;
use std::net::SocketAddr;

/**
 * Configuration for a Dropshot server.
 *
 * This type implements [`serde::Deserialize`] and [`serde::Serialize`] and it
 * can be composed with the consumer's configuration (whatever format that's
 * in).  For example, consumers could define a custom `MyAppConfig` for an app
 * that contains a Dropshot server:
 *
 * ```
 * use dropshot::ConfigDropshot;
 * use serde::Deserialize;
 *
 * #[derive(Deserialize)]
 * struct MyAppConfig {
 *     http_api_server: ConfigDropshot,
 *     /* ... (other app-specific config) */
 * }
 *
 * fn main() -> Result<(), String> {
 *     let my_config: MyAppConfig = toml::from_str(
 *         r##"
 *             [http_api_server]
 *             bind_address = "127.0.0.1:12345"
 *
 *             ## ... (other app-specific config)
 *         "##
 *     ).map_err(|error| format!("parsing config: {}", error))?;
 *
 *     let dropshot_config: &ConfigDropshot = &my_config.http_api_server;
 *     /* ... (use the config to create a server) */
 *     Ok(())
 * }
 * ```
 */
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ConfigDropshot {
    /** IP address and TCP port to which to bind for accepting connections */
    pub bind_address: SocketAddr,
}
