// Copyright 2023 Oxide Computer Company
//! DTrace probes and support

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct RequestInfo {
    id: String,
    local_addr: std::net::SocketAddr,
    remote_addr: std::net::SocketAddr,
    method: String,
    path: String,
    query: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ResponseInfo {
    id: String,
    local_addr: std::net::SocketAddr,
    remote_addr: std::net::SocketAddr,
    status_code: u16,
    message: String,
}

#[cfg(feature = "usdt-probes")]
#[usdt::provider(provider = "dropshot")]
mod probes {
    use super::{RequestInfo, ResponseInfo};
    fn request__start(_: &RequestInfo) {}
    fn request__done(_: &ResponseInfo) {}
}

/// The result of registering a server's DTrace USDT probes.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeRegistration {
    /// The probes are explicitly disabled at compile time.
    Disabled,

    /// Probes were successfully registered.
    Succeeded,

    /// Registration failed, with an error message explaining the cause.
    Failed(String),
}
