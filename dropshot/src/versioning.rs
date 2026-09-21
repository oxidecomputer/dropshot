// Copyright 2024 Oxide Computer Company

//! Support for API versioning

use crate::Body;
use crate::HttpError;
use http::HeaderName;
use http::HeaderValue;
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

    /// Add `Vary` response headers for request fields that affect routing.
    pub(crate) fn add_vary_headers(&self, headers: &mut http::HeaderMap) {
        if let VersionPolicy::Dynamic(policy) = self {
            for header_name in policy.response_vary_on() {
                add_vary_on_header(headers, header_name);
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

    /// Request header field names that may cause the response to vary.
    ///
    /// These are added to the response `Vary` header so that intermediate
    /// caches store separate entries per value.
    fn response_vary_on(&self) -> &[HeaderName] {
        &[]
    }
}

/// Implementation of `DynamicVersionPolicy` where the client specifies a
/// specific semver in a specific header and we always use whatever they
/// requested.
///
/// An incoming request will be rejected with a 400-level error if:
///
/// - the header value cannot be parsed as a semver, or
/// - the requested version is newer than `max_version` (see
///   [`ClientSpecifiesVersionInHeader::new()`], which implies that the client
///   is trying to use a newer version of the API than this server supports.
///
/// By default, incoming requests will also be rejected with a 400-level error
/// if the header is missing. To override this behavior, supply a default
/// version via [`Self::on_missing`].
///
/// If you need anything more flexible (e.g., validating the provided version
/// against a fixed set of supported versions), you'll want to impl
/// `DynamicVersionPolicy` yourself.
#[derive(Debug)]
pub struct ClientSpecifiesVersionInHeader {
    name: HeaderName,
    max_version: Version,
    on_missing: Option<Version>,
}

impl ClientSpecifiesVersionInHeader {
    /// Make a new `ClientSpecifiesVersionInHeader` policy.
    ///
    /// Arguments:
    ///
    /// * `name`: name of the header that the client will use to specify the
    ///   version
    /// * `max_version`: the maximum version of the API that this server
    ///   supports.  Requests for a version newer than this will be rejected
    ///   with a 400-level error.
    pub fn new(
        name: HeaderName,
        max_version: Version,
    ) -> ClientSpecifiesVersionInHeader {
        ClientSpecifiesVersionInHeader { name, max_version, on_missing: None }
    }

    /// If the header is missing, use the provided version instead.
    ///
    /// By default, the policy will reject requests with a missing header. Call
    /// this function to use the provided version instead.
    ///
    /// Typically, the provided version should either be a fixed supported
    /// version (for backwards compatibility with older clients), or the newest
    /// supported version (in case clients are generally kept up-to-date but not
    /// all clients send the header).
    ///
    /// Using this function is not recommended if you control all clients—in
    /// that case, arrange for clients to send the header instead. In
    /// particular, **at Oxide, do not use this function for internal APIs.**
    pub fn on_missing(mut self, version: Version) -> Self {
        self.on_missing = Some(version);
        self
    }
}

impl DynamicVersionPolicy for ClientSpecifiesVersionInHeader {
    fn response_vary_on(&self) -> &[HeaderName] {
        std::slice::from_ref(&self.name)
    }

    fn request_extract_version(
        &self,
        request: &Request<Body>,
        _log: &Logger,
    ) -> Result<Version, HttpError> {
        let v = parse_header(request.headers(), &self.name)?;
        match (v, &self.on_missing) {
            (Some(v), _) => {
                if v <= self.max_version {
                    Ok(v)
                } else {
                    Err(HttpError::for_bad_request(
                        None,
                        format!(
                            "server does not support this API version: {}",
                            v
                        ),
                    ))
                }
            }
            (None, Some(on_missing)) => Ok(on_missing.clone()),
            (None, None) => Err(HttpError::for_bad_request(
                None,
                format!("missing expected header {:?}", self.name),
            )),
        }
    }
}

/// Parses a required header out of a request (producing useful error messages
/// for all failure modes)
fn parse_header<T>(
    headers: &http::HeaderMap,
    header_name: &HeaderName,
) -> Result<Option<T>, HttpError>
where
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Display,
{
    let Some(v_value) = headers.get(header_name) else { return Ok(None) };

    let v_str = v_value.to_str().map_err(|_| {
        HttpError::for_bad_request(
            None,
            format!(
                "bad value for header {:?}: not ASCII: {:?}",
                header_name, v_value
            ),
        )
    })?;

    let v = v_str.parse::<T>().map_err(|e| {
        HttpError::for_bad_request(
            None,
            format!("bad value for header {:?}: {}: {}", header_name, e, v_str),
        )
    })?;

    Ok(Some(v))
}

fn header_value_contains_field(value: &HeaderValue, field: &str) -> bool {
    value.to_str().is_ok_and(|vary| {
        vary.split(',')
            .any(|v| v.trim().eq_ignore_ascii_case(field))
    })
}

/// Adds a request header field name to the response `Vary` header if not
/// already present.
fn add_vary_on_header(headers: &mut http::HeaderMap, header_name: &HeaderName) {
    let vary_values = headers.get_all(http::header::VARY);

    // can't and shouldn't add anything if we already have "*"
    // https://datatracker.ietf.org/doc/html/rfc7231#section-7.1.4
    if vary_values.iter().any(|v| v == "*") {
        return;
    }

    let field = header_name.as_str();
    if !vary_values.iter().any(|v| header_value_contains_field(v, field)) {
        headers.append(
            http::header::VARY,
            HeaderValue::from_str(field).expect("header name is valid"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::VARY;

    #[test]
    fn test_add_vary_on_header() {
        let header_name: HeaderName = "dropshot-test-version".parse().unwrap();
        let mut headers = http::HeaderMap::new();
        add_vary_on_header(&mut headers, &header_name);

        let vary_values: Vec<_> = headers
            .get_all(VARY)
            .iter()
            .map(|value| value.to_str().unwrap().to_string())
            .collect();
        assert!(
            vary_values.iter().any(|value| value
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case("dropshot-test-version")))
        );
    }

    #[test]
    fn test_add_vary_on_header_avoids_duplicates() {
        let header_name: HeaderName = "dropshot-test-version".parse().unwrap();
        let mut headers = http::HeaderMap::new();
        headers.append(
            VARY,
            HeaderValue::from_static("dropshot-test-version, Accept-Language"),
        );

        add_vary_on_header(&mut headers, &header_name);

        let mut version_header_count = 0;
        for value in headers.get_all(VARY).iter() {
            let text = value.to_str().unwrap();
            version_header_count += text
                .split(',')
                .filter(|v| {
                    v.trim().eq_ignore_ascii_case("dropshot-test-version")
                })
                .count();
        }

        assert_eq!(version_header_count, 1);
    }

    #[test]
    fn test_add_vary_on_header_skips_when_vary_is_star() {
        let header_name: HeaderName = "dropshot-test-version".parse().unwrap();
        let mut headers = http::HeaderMap::new();
        headers.append(VARY, HeaderValue::from_static("*"));

        add_vary_on_header(&mut headers, &header_name);

        assert_eq!(headers.get_all(VARY).iter().count(), 1);
        assert_eq!(headers.get(VARY).unwrap(), "*");
    }
}
