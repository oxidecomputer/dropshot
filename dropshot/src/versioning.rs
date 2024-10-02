// Copyright 2024 Oxide Computer Company

//! Support for API versioning

use crate::Body;
use crate::HttpError;
use http::HeaderName;
use hyper::Request;
use semver::Version;
use slog::Logger;
use std::str::FromStr;

/// Specifies how a server handles API versioning
#[derive(Debug)]
pub enum VersionPolicy {
    /// This server does not use API versioning.
    ///
    /// All endpoints registered with this server must be specified with
    /// versions = `ApiEndpointVersions::All` (the default).  Dropshot will not
    /// attempt to determine a version for each request.  It will route requests
    /// without considering versions at all.
    Unversioned,

    /// This server uses API versioning and the provided
    /// [`DynamicVersionPolicy`] specifies how to determine the API version to
    /// use for each incoming request.
    ///
    /// With this policy, when a request arrives, Dropshot uses the provided
    /// `DynamicVersionPolicy` impl to determine what API version to use when
    /// handling the request.  Then it routes the request to a handler based on
    /// the HTTP method and path (as usual), filtering out handlers whose
    /// associated `versions` does not include the requested version.
    Dynamic(Box<dyn DynamicVersionPolicy>),
}

impl VersionPolicy {
    /// Given an incoming request, determine the version constraint (if any) to
    /// use when routing the request to a handler
    pub(crate) fn request_version(
        &self,
        request: &Request<Body>,
        request_log: &Logger,
    ) -> Result<Option<Version>, HttpError> {
        match self {
            // If the server is unversioned, then we can ignore versioning
            // altogether when routing.  The result is still ambiguous because
            // we never allow multiple endpoints to have the same HTTP method
            // and path and overlapping version ranges, and unversioned servers
            // only support endpoints whose version range is `All`.
            VersionPolicy::Unversioned => Ok(None),

            // If the server is versioned, use the client-provided impl to
            // determine which version to use.  In this case the impl must
            // return a value or an error -- it's not allowed to decline to
            // provide a version.
            VersionPolicy::Dynamic(vers_impl) => {
                let result =
                    vers_impl.request_extract_version(request, request_log);

                match &result {
                    Ok(version) => {
                        debug!(request_log, "determined request API version";
                            "version" => %version,
                        );
                    }
                    Err(error) => {
                        error!(
                            request_log,
                            "failed to determine request API version";
                            "error" => ?error,
                        );
                    }
                }

                result.map(Some)
            }
        }
    }
}

/// Determines the API version to use for an incoming request
///
/// See [`ClientSpecifiesVersionInHeader`] for a basic implementation that, as
/// the name suggests, requires that the client specify the exact version they
/// want to use in a header and then always uses whatever they provide.
///
/// This trait gives you freedom to implement a very wide range of behavior.
/// For example, you could:
///
/// * Require that the client specify a particular version and always use that
/// * Require that the client specify a particular version but require that it
///   come from a fixed set of supported versions
/// * Allow clients to specify a specific version but supply a default if they
///   don't
/// * Allow clients to specify something else (e.g., a version range, like
///   ">1.0.0") that you then map to a specific version based on the API
///   versions that you know about
///
/// This does mean that if you care about restricting this in any way (e.g.,
/// restricting the allowed API versions to a fixed set), you must implement
/// that yourself by impl'ing this trait.
pub trait DynamicVersionPolicy: std::fmt::Debug + Send + Sync {
    /// Given a request, determine the API version to use to route the request
    /// to the appropriate handler
    ///
    /// This is expected to be a quick, synchronous operation.  Most commonly,
    /// you might parse a semver out of a particular header, maybe match it
    /// against some supported set of versions, and maybe supply a default if
    /// you don't find the header at all.
    fn request_extract_version(
        &self,
        request: &Request<Body>,
        log: &Logger,
    ) -> Result<Version, HttpError>;
}

/// Implementation of `DynamicVersionPolicy` where the client must specify a
/// specific semver in a specific header and we always use whatever they
/// requested
#[derive(Debug)]
pub struct ClientSpecifiesVersionInHeader {
    name: HeaderName,
}

impl ClientSpecifiesVersionInHeader {
    pub fn new(name: HeaderName) -> ClientSpecifiesVersionInHeader {
        ClientSpecifiesVersionInHeader { name }
    }
}

impl DynamicVersionPolicy for ClientSpecifiesVersionInHeader {
    fn request_extract_version(
        &self,
        request: &Request<Body>,
        _log: &Logger,
    ) -> Result<Version, HttpError> {
        parse_header(request.headers(), &self.name)
    }
}

/// Parses a required header out of a request (producing useful error messages
/// for all failure modes)
fn parse_header<T>(
    headers: &http::HeaderMap,
    header_name: &HeaderName,
) -> Result<T, HttpError>
where
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Display,
{
    let v_value = headers.get(header_name).ok_or_else(|| {
        HttpError::for_bad_request(
            None,
            format!("missing expected header {:?}", header_name),
        )
    })?;

    let v_str = v_value.to_str().map_err(|_| {
        HttpError::for_bad_request(
            None,
            format!(
                "bad value for header {:?}: not ASCII: {:?}",
                header_name, v_value
            ),
        )
    })?;

    v_str.parse::<T>().map_err(|e| {
        HttpError::for_bad_request(
            None,
            format!("bad value for header {:?}: {}: {}", header_name, e, v_str),
        )
    })
}
