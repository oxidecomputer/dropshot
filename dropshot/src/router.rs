// Copyright 2024 Oxide Computer Company
//! Routes incoming HTTP requests to handler functions

use super::error::HttpError;
use super::error_status_code::ClientErrorStatusCode;
use super::handler::RouteHandler;

use crate::api_description::ApiEndpointVersions;
use crate::from_map::MapError;
use crate::from_map::MapValue;
use crate::server::ServerContext;
use crate::ApiEndpoint;
use crate::RequestEndpointMetadata;
use http::Method;
use percent_encoding::percent_decode_str;
use semver::Version;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

/// `HttpRouter` is a simple data structure for routing incoming HTTP requests
/// to specific handler functions based on the request method, URI path, and
/// version.  For examples, see the basic test below.
///
/// Routes are registered and looked up according to a path, like `"/foo/bar"`.
/// Paths are split into segments separated by one or more '/' characters.  When
/// registering a route, a path segment may be either a literal string or a
/// variable.  Variables are specified by wrapping the segment in braces.
///
/// For example, a handler registered for `"/foo/bar"` will match only
/// `"/foo/bar"` (after normalization, that is -- it will also match
/// `"/foo///bar"`).  A handler registered for `"/foo/{bar}"` uses a
/// variable for the second segment, so it will match `"/foo/123"` (with `"bar"`
/// assigned to `"123"`) as well as `"/foo/bar456"` (with `"bar"` mapped to
/// `"bar456"`).  Only one segment is matched per variable, so `"/foo/{bar}"`
/// will not match `"/foo/123/456"`.
///
/// The implementation here is essentially a trie where edges represent segments
/// of the URI path.  ("Segments" here are chunks of the path separated by one or
/// more "/" characters.)  To register or look up the path `"/foo/bar/baz"`, we
/// would start at the root and traverse edges for the literal strings `"foo"`,
/// `"bar"`, and `"baz"`, arriving at a particular node.  Each node has a set of
/// handlers, each associated with one HTTP method.
///
/// We make (and, in some cases, enforce) a number of simplifying assumptions.
/// These could be relaxed, but it's not clear that's useful, and enforcing them
/// makes it easier to catch some types of bugs:
///
/// * A particular resource (node) may have child resources (edges) with either
///   literal path segments or variable path segments, but not both.  For
///   example, you can't register both `"/projects/{id}"` and
///   `"/projects/default"`.
///
/// * If a given resource has an edge with a variable name, all routes through
///   this node for the same API version must use the same name for that
///   variable.  That is, within a single version you can't define routes for
///   `"/projects/{id}"` and `"/projects/{project_id}/info"`.  However,
///   different API versions may use different variable names at the same
///   position (e.g., version 1 uses `{arg}` and version 2 uses `{myarg}`).
///
/// * A given path cannot use the same variable name twice.  For example, you
///   can't register path `"/projects/{id}/instances/{id}"`.
///
/// * A given resource may have at most one handler for a given HTTP method and
///   version.
///
/// * The expectation is that during server initialization,
///   `HttpRouter::insert()` will be invoked to register a number of route
///   handlers.  After that initialization period, the router will be
///   read-only.  This behavior isn't enforced by `HttpRouter`.
#[derive(Debug)]
pub struct HttpRouter<Context: ServerContext> {
    /// root of the trie
    root: Box<HttpRouterNode<Context>>,
    /// indicates whether this router contains any endpoints that are
    /// constrained by version
    has_versioned_routes: bool,
}

/// Each node in the tree represents a group of HTTP resources having the same
/// handler functions.  As described above, these may correspond to exactly one
/// canonical path (e.g., `"/foo/bar"`) or a set of paths that differ by some
/// number of variable assignments (e.g., `"/projects/123/instances"` and
/// `"/projects/456/instances"`).
///
/// Edges of the tree come in one of type types: edges for literal strings and
/// edges for variable strings.  A given node has either literal string edges or
/// variable edges, but not both.  However, we don't necessarily know what type
/// of outgoing edges a node will have when we create it.
#[derive(Debug)]
struct HttpRouterNode<Context: ServerContext> {
    /// Handlers, etc. for each of the HTTP methods defined for this node.
    methods: BTreeMap<String, Vec<ApiEndpoint<Context>>>,
    /// Edges linking to child nodes.
    edges: Option<HttpRouterEdges<Context>>,
}

#[derive(Debug)]
enum HttpRouterEdges<Context: ServerContext> {
    /// Outgoing edges for literal paths.
    Literals(BTreeMap<String, Box<HttpRouterNode<Context>>>),
    /// Outgoing edge for variable-named paths.
    VariableSingle(VersionedVariableNames, Box<HttpRouterNode<Context>>),
    /// Outgoing edge that consumes all remaining components.
    VariableRest(VersionedVariableNames, Box<HttpRouterNode<Context>>),
}

/// Tracks variable names used at a particular path segment, potentially
/// different across API versions.
///
/// Within a single API version, the variable name for a path segment must be
/// consistent. Across versions, different names are allowed: e.g., version 1.x
/// might use `{project_id}` while version 2.x uses `{id}` at the same
/// position.
#[derive(Debug)]
struct VersionedVariableNames {
    /// Each entry associates a variable name with the version range of the
    /// endpoint that registered it. Multiple entries may share the same name
    /// (when multiple endpoints use the same variable name at this position).
    /// Entries with *different* names must have non-overlapping version ranges.
    entries: Vec<(String, ApiEndpointVersions)>,
}

impl VersionedVariableNames {
    /// Create an empty `VersionedVariableNames`.
    fn empty() -> Self {
        VersionedVariableNames { entries: Vec::new() }
    }

    /// Record a variable name and its associated version range. Panics if the
    /// new name has an overlapping version range with an existing name. (We
    /// allow variable renames _across_ versions, but not across endpoint types
    /// like GET or PUT within the same version.)
    fn add(&mut self, path: &str, name: &str, versions: &ApiEndpointVersions) {
        for (existing_name, existing_versions) in &self.entries {
            if existing_name == name && existing_versions == versions {
                // Exact duplicate (e.g., GET and POST at the same path with
                // the same variable name and version range).  Nothing new to
                // record.
                return;
            }
            if existing_name != name
                && existing_versions.overlaps_with(versions)
            {
                panic!(
                    "URI path \"{}\": attempted to use variable name \
                     \"{}\", but a different name (\"{}\") is already \
                     used for the same path segment in an overlapping \
                     API version range",
                    path, name, existing_name
                );
            }
        }
        self.entries.push((name.to_string(), versions.clone()));
    }

    /// Look up the variable name for a given version.
    ///
    /// When `version` is `Some(v)`, returns the name associated with a version
    /// range that includes `v`. When `version` is `None`, returns the first
    /// name (for unversioned/test usage, where any name is acceptable).
    fn name_for_version(&self, version: Option<&Version>) -> Option<&str> {
        self.entries
            .iter()
            .find(|(_, versions)| versions.matches(version))
            .map(|(name, _)| name.as_str())
    }
}

/// `PathSegment` represents a segment in a URI path when the router is being
/// configured.  Each segment may be either a literal string or a variable (the
/// latter indicated by being wrapped in braces). Variables may consume a single
/// /-delimited segment or several as defined by a regex (currently only `.*` is
/// supported).
#[derive(Debug, PartialEq)]
pub enum PathSegment {
    /// a path segment for a literal string
    Literal(String),
    /// a path segment for a variable
    VarnameSegment(String),
    /// a path segment that matches all remaining components for a variable
    VarnameWildcard(String),
}

impl PathSegment {
    /// Given a `&str` representing a path segment from a Uri, return a
    /// PathSegment.  This is used to parse a sequence of path segments to the
    /// corresponding `PathSegment`, which basically means determining whether
    /// it's a variable or a literal.
    pub fn from(segment: &str) -> PathSegment {
        if segment.starts_with('{') || segment.ends_with('}') {
            assert!(
                segment.starts_with('{'),
                "{}",
                "HTTP URI path segment variable missing leading \"{\""
            );
            assert!(
                segment.ends_with('}'),
                "{}",
                "HTTP URI path segment variable missing trailing \"}\""
            );

            let var = &segment[1..segment.len() - 1];

            let (var, pat) = if let Some(index) = var.find(':') {
                (&var[..index], Some(&var[index + 1..]))
            } else {
                (var, None)
            };

            // Note that the only constraint on the variable name is that it is
            // not empty. Consumers may choose odd names like '_' or 'type'
            // that are not valid Rust identifiers and rename them with
            // serde attributes during deserialization.
            assert!(
                !var.is_empty(),
                "HTTP URI path segment variable name must not be empty",
            );

            if let Some(pat) = pat {
                assert!(
                    pat == ".*",
                    "Only the pattern '.*' is currently supported"
                );
                PathSegment::VarnameWildcard(var.to_string())
            } else {
                PathSegment::VarnameSegment(var.to_string())
            }
        } else {
            PathSegment::Literal(segment.to_string())
        }
    }
}

/// Wrapper for a path that's the result of user input i.e. an HTTP query.
/// We use this type to avoid confusion with paths used to define routes.
#[derive(Debug, Clone, Copy)]
pub struct InputPath<'a>(&'a str);

impl<'a> From<&'a str> for InputPath<'a> {
    fn from(s: &'a str) -> Self {
        Self(s)
    }
}

/// A value for a variable which may either be a single value or a list of
/// values in the case of wildcard path matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariableValue {
    String(String),
    Components(Vec<String>),
}

pub type VariableSet = BTreeMap<String, VariableValue>;

impl MapValue for VariableValue {
    fn as_value(&self) -> Result<&str, MapError> {
        match self {
            VariableValue::String(s) => Ok(s.as_str()),
            VariableValue::Components(_) => Err(MapError(
                "cannot deserialize sequence as a single value".to_string(),
            )),
        }
    }

    fn as_seq(&self) -> Result<Box<dyn Iterator<Item = String>>, MapError> {
        match self {
            VariableValue::String(_) => Err(MapError(
                "cannot deserialize a single value as a sequence".to_string(),
            )),
            VariableValue::Components(v) => Ok(Box::new(v.clone().into_iter())),
        }
    }
}

/// The result of invoking `HttpRouter::lookup_route()`.
///
/// A successful route lookup includes the handler and endpoint-related metadata.
#[derive(Debug)]
pub struct RouterLookupResult<Context: ServerContext> {
    pub handler: Arc<dyn RouteHandler<Context>>,
    pub endpoint: RequestEndpointMetadata,
}

impl<Context: ServerContext> HttpRouterNode<Context> {
    pub fn new() -> Self {
        HttpRouterNode { methods: BTreeMap::new(), edges: None }
    }
}

impl<Context: ServerContext> HttpRouter<Context> {
    /// Returns a new `HttpRouter` with no routes configured.
    pub fn new() -> Self {
        HttpRouter {
            root: Box::new(HttpRouterNode::new()),
            has_versioned_routes: false,
        }
    }

    /// Configure a route for HTTP requests based on the HTTP `method` and
    /// URI `path`.  See the `HttpRouter` docs for information about how `path`
    /// is processed.  Requests matching `path` will be resolved to `handler`.
    pub fn insert(&mut self, endpoint: ApiEndpoint<Context>) {
        let method = endpoint.method.clone();
        let path = endpoint.path.clone();

        let all_segments = route_path_to_segments(path.as_str());

        let mut all_segments = all_segments.into_iter();
        let mut varnames: BTreeSet<String> = BTreeSet::new();

        let mut node: &mut Box<HttpRouterNode<Context>> = &mut self.root;
        while let Some(raw_segment) = all_segments.next() {
            let segment = PathSegment::from(raw_segment);

            node = match segment {
                PathSegment::Literal(lit) => {
                    let edges = node.edges.get_or_insert(
                        HttpRouterEdges::Literals(BTreeMap::new()),
                    );
                    match edges {
                        // We do not allow both literal and variable edges from
                        // the same node.  This could be supported (with some
                        // caveats about how matching would work), but it seems
                        // more likely to be a mistake.
                        HttpRouterEdges::VariableSingle(_, _)
                        | HttpRouterEdges::VariableRest(_, _) => {
                            panic!(
                                "URI path \"{}\": attempted to register route \
                                 for literal path segment \"{}\" when a route \
                                 exists for a variable path segment",
                                path, lit,
                            );
                        }
                        HttpRouterEdges::Literals(ref mut literals) => literals
                            .entry(lit)
                            .or_insert_with(|| Box::new(HttpRouterNode::new())),
                    }
                }

                PathSegment::VarnameSegment(new_varname) => {
                    insert_var(&path, &mut varnames, &new_varname);

                    let edges = node.edges.get_or_insert(
                        HttpRouterEdges::VariableSingle(
                            VersionedVariableNames::empty(),
                            Box::new(HttpRouterNode::new()),
                        ),
                    );
                    match edges {
                        // See the analogous check above about combining literal
                        // and variable path segments from the same resource.
                        HttpRouterEdges::Literals(_) => panic!(
                            "URI path \"{}\": attempted to register route for \
                             variable path segment (variable name: \"{}\") \
                             when a route already exists for a literal path \
                             segment",
                            path, new_varname
                        ),

                        HttpRouterEdges::VariableRest(_, _) => panic!(
                            "URI path \"{}\": attempted to register route for \
                             variable path segment (variable name: \"{}\") \
                             when a route already exists for a wildcard path \
                             segment",
                            path, new_varname,
                        ),

                        HttpRouterEdges::VariableSingle(
                            varnames,
                            ref mut node,
                        ) => {
                            // Record the variable name for this endpoint's
                            // version range. This validates that no conflicting
                            // name exists for overlapping versions.
                            varnames.add(
                                &path,
                                &new_varname,
                                &endpoint.versions,
                            );
                            node
                        }
                    }
                }
                PathSegment::VarnameWildcard(new_varname) => {
                    /*
                     * We don't accept further path segments after the .*.
                     */
                    if all_segments.next().is_some() {
                        panic!(
                            "URI path \"{}\": attempted to match segments \
                             after the wildcard variable \"{}\"",
                            path, new_varname,
                        );
                    }

                    insert_var(&path, &mut varnames, &new_varname);

                    let edges = node.edges.get_or_insert(
                        HttpRouterEdges::VariableRest(
                            VersionedVariableNames::empty(),
                            Box::new(HttpRouterNode::new()),
                        ),
                    );
                    match edges {
                        /*
                         * See the analogous check above about combining literal
                         * and variable path segments from the same resource.
                         */
                        HttpRouterEdges::Literals(_) => panic!(
                            "URI path \"{}\": attempted to register route for \
                             variable path regex (variable name: \"{}\") when \
                             a route already exists for a literal path segment",
                            path, new_varname
                        ),

                        HttpRouterEdges::VariableSingle(_, _) => {
                            panic!(
                                "URI path \"{}\": attempted to register route \
                                 for wildcard path segment (variable name: \
                                 \"{}\") when a route already exists for a \
                                 variable path segment",
                                path, new_varname,
                            )
                        }

                        HttpRouterEdges::VariableRest(
                            versioned_names,
                            ref mut node,
                        ) => {
                            // Record the variable name for this endpoint's
                            // version range. This validates that no conflicting
                            // name exists for overlapping versions.
                            versioned_names.add(
                                &path,
                                &new_varname,
                                &endpoint.versions,
                            );
                            node
                        }
                    }
                }
            };
        }

        let methodname = method.as_str().to_uppercase();
        let existing_handlers =
            node.methods.entry(methodname.clone()).or_default();

        for handler in existing_handlers.iter() {
            if handler.versions.overlaps_with(&endpoint.versions) {
                if handler.versions == endpoint.versions {
                    panic!(
                        "URI path \"{}\": attempted to create duplicate route \
                        for method \"{}\"",
                        path, methodname
                    );
                } else {
                    panic!(
                        "URI path \"{}\": attempted to register multiple \
                        handlers for method \"{}\" with overlapping version \
                        ranges",
                        path, methodname
                    );
                }
            }
        }

        if endpoint.versions != ApiEndpointVersions::All {
            self.has_versioned_routes = true;
        }

        existing_handlers.push(endpoint);
    }

    /// Returns whether this router contains any routes that are constrained by
    /// version
    pub fn has_versioned_routes(&self) -> bool {
        self.has_versioned_routes
    }

    #[cfg(test)]
    pub fn lookup_route_unversioned(
        &self,
        method: &Method,
        path: InputPath<'_>,
    ) -> Result<RouterLookupResult<Context>, HttpError> {
        self.lookup_route(method, path, None)
    }

    /// Look up the route handler for an HTTP request having method `method` and
    /// URI path `path`.  A successful lookup produces a `RouterLookupResult`,
    /// which includes both the handler that can process this request and a map
    /// of variables assigned based on the request path as part of the lookup.
    /// On failure, this returns an `HttpError` appropriate for the failure
    /// mode.
    pub fn lookup_route(
        &self,
        method: &Method,
        path: InputPath<'_>,
        version: Option<&Version>,
    ) -> Result<RouterLookupResult<Context>, HttpError> {
        let all_segments = input_path_to_segments(&path).map_err(|_| {
            HttpError::for_bad_request(
                None,
                String::from("invalid path encoding"),
            )
        })?;
        let mut all_segments = all_segments.into_iter();
        let mut node = &self.root;
        let mut variables = VariableSet::new();

        while let Some(segment) = all_segments.next() {
            let segment_string = segment.to_string();

            node = match &node.edges {
                None => None,

                Some(HttpRouterEdges::Literals(edges)) => {
                    edges.get(&segment_string)
                }
                Some(HttpRouterEdges::VariableSingle(varnames, ref node)) => {
                    match varnames.name_for_version(version) {
                        Some(varname) => {
                            variables.insert(
                                varname.to_string(),
                                VariableValue::String(segment_string),
                            );
                            Some(node)
                        }
                        // No variable name matches this version, so no route
                        // exists through this edge for this version.
                        None => None,
                    }
                }
                Some(HttpRouterEdges::VariableRest(varnames, node)) => {
                    match varnames.name_for_version(version) {
                        Some(varname) => {
                            let mut rest = vec![segment];
                            while let Some(segment) = all_segments.next() {
                                rest.push(segment);
                            }
                            variables.insert(
                                varname.to_string(),
                                VariableValue::Components(rest),
                            );
                            // There should be no outgoing edges since this is
                            // by definition a terminal node.
                            assert!(node.edges.is_none());
                            Some(node)
                        }
                        None => None,
                    }
                }
            }
            .ok_or_else(|| {
                HttpError::for_not_found(
                    None,
                    String::from("no route found (no path in router)"),
                )
            })?
        }

        // The wildcard match consumes the implicit, empty path segment.
        // We must always advance to the wildcard's terminal node so that
        // handler lookup happens on the correct node (where wildcard handlers
        // are registered), not on the parent (which may have unrelated
        // handlers for other methods/paths).
        match &node.edges {
            Some(HttpRouterEdges::VariableRest(varnames, new_node)) => {
                if let Some(varname) = varnames.name_for_version(version) {
                    variables.insert(
                        varname.to_string(),
                        VariableValue::Components(vec![]),
                    );
                } else {
                    // The wildcard edge exists but doesn't cover the requested
                    // version. A wildcard can be version-scoped, so a request
                    // may reach this node (via literal edges, which are
                    // version-agnostic) for a version the wildcard doesn't
                    // cover.
                    //
                    // We still advance to the terminal node below. The in-loop
                    // VariableRest handler (above) returns None in this
                    // situation, producing a 404, but here the path segments
                    // are already exhausted. If we stayed on the parent node,
                    // the handler lookup would see the parent's handlers (which
                    // belong to a different path, e.g. POST /files vs GET
                    // /files/{path:.*}) and could return a spurious 405.
                }
                // Registration rejects path segments after a `.*` wildcard,
                // so the wildcard's terminal node can never have children.
                assert!(new_node.edges.is_none());
                node = new_node;
            }
            Some(HttpRouterEdges::Literals(_)) => {
                // Literal edges don't implicitly consume a trailing segment the
                // way wildcards do, so there's nothing to advance past.
            }
            Some(HttpRouterEdges::VariableSingle(_, _)) => {
                // A single-segment variable edge requires a non-empty segment
                // to match. Since we've exhausted all path segments, there's
                // nothing for it to bind to; the handler lookup below will
                // operate on the current node, which is correct.
            }
            None => {
                // No outgoing edges at all. This is a leaf node.
            }
        }

        // First, look for a matching implementation.
        let methodname = method.as_str().to_uppercase();
        if let Some(handler) = find_handler_matching_version(
            node.methods.get(&methodname).map(|v| v.as_slice()).unwrap_or(&[]),
            version,
        ) {
            return Ok(RouterLookupResult {
                handler: Arc::clone(&handler.handler),
                endpoint: RequestEndpointMetadata {
                    operation_id: handler.operation_id.clone(),
                    variables,
                    body_content_type: handler.body_content_type.clone(),
                    request_body_max_bytes: handler.request_body_max_bytes,
                },
            });
        }

        // We found no handler matching this path, method name, and version.
        // We're going to report a 404 ("Not Found") or 405 ("Method Not
        // Allowed").  It's a 405 if there are any handlers matching this path
        // and version for a different method.  It's a 404 otherwise.
        if node.methods.values().any(|handlers| {
            find_handler_matching_version(handlers, version).is_some()
        }) {
            let mut err = HttpError::for_client_error_with_status(
                None,
                ClientErrorStatusCode::METHOD_NOT_ALLOWED,
            );

            // Add `Allow` headers for the methods that *are* acceptable for
            // this path, as specified in § 15.5.0 RFC9110, which states:
            //
            // > The origin server MUST generate an Allow header field in a
            // > 405 response containing a list of the target resource's
            // > currently supported methods.
            //
            // See: https://httpwg.org/specs/rfc9110.html#status.405
            if let Some(hdrs) = err.headers.as_deref_mut() {
                hdrs.reserve(node.methods.len());
            }
            for allowed in node.methods.keys() {
                err.add_header(http::header::ALLOW, allowed)
                    .expect("method should be a valid allow header");
            }
            Err(err)
        } else {
            Err(HttpError::for_not_found(
                None,
                format!(
                    "route has no handlers for version {}",
                    match version {
                        Some(v) => v.to_string(),
                        None => String::from("<none>"),
                    }
                ),
            ))
        }
    }

    pub fn endpoints<'a>(
        &'a self,
        version: Option<&'a Version>,
    ) -> HttpRouterIter<'a, Context> {
        HttpRouterIter::new(self, version)
    }
}

/// Given a list of handlers, return the first one matching the given semver
///
/// If `version` is `None`, any handler will do.
fn find_handler_matching_version<'a, I, C>(
    handlers: I,
    version: Option<&Version>,
) -> Option<&'a ApiEndpoint<C>>
where
    I: IntoIterator<Item = &'a ApiEndpoint<C>>,
    C: ServerContext,
{
    handlers.into_iter().find(|h| h.versions.matches(version))
}

/// Insert a variable into the set after checking for duplicates.
fn insert_var(
    path: &str,
    varnames: &mut BTreeSet<String>,
    new_varname: &String,
) -> () {
    // Do not allow the same variable name to be used more than
    // once in the path.  Again, this could be supported (with
    // some caveats), but it seems more likely to be a mistake.
    if varnames.contains(new_varname) {
        panic!(
            "URI path \"{}\": variable name \"{}\" is used more than once",
            path, new_varname
        );
    }
    varnames.insert(new_varname.clone());
}

/// Route Interator implementation. We perform a preorder, depth first traversal
/// of the tree starting from the root node. For each node, we enumerate the
/// methods and then descend into its children (or single child in the case of
/// path parameter variables). `method` holds the iterator over the current
/// node's `methods`; `path` is a stack that represents the current collection
/// of path segments and the iterators at each corresponding node. We start with
/// the root node's `methods` iterator and a stack consisting of a
/// blank string and an iterator over the root node's children.
pub struct HttpRouterIter<'a, Context: ServerContext> {
    method:
        Box<dyn Iterator<Item = (&'a String, &'a ApiEndpoint<Context>)> + 'a>,
    path: Vec<(PathSegment, Box<PathIter<'a, Context>>)>,
    version: Option<&'a Version>,
}
type PathIter<'a, Context> =
    dyn Iterator<Item = (PathSegment, &'a Box<HttpRouterNode<Context>>)> + 'a;

fn iter_handlers_from_node<'a, 'b, 'c, C: ServerContext>(
    node: &'a HttpRouterNode<C>,
    version: Option<&'b Version>,
) -> Box<dyn Iterator<Item = (&'a String, &'a ApiEndpoint<C>)> + 'c>
where
    'a: 'c,
    'b: 'c,
{
    Box::new(node.methods.iter().flat_map(move |(m, handlers)| {
        handlers.iter().filter_map(move |h| {
            if h.versions.matches(version) {
                Some((m, h))
            } else {
                None
            }
        })
    }))
}

impl<'a, Context: ServerContext> HttpRouterIter<'a, Context> {
    fn new(
        router: &'a HttpRouter<Context>,
        version: Option<&'a Version>,
    ) -> Self {
        HttpRouterIter {
            method: iter_handlers_from_node(&router.root, version),
            path: vec![(
                PathSegment::Literal("".to_string()),
                HttpRouterIter::iter_node(&router.root, version),
            )],
            version,
        }
    }

    /// Produce an iterator over `node`'s children. This is the null (empty)
    /// iterator if there are no children, a single (once) iterator for a
    /// path parameter variable, and a modified iterator in the case of
    /// literal, explicit path segments.
    ///
    /// When the node has a variable edge with versioned names, the `version`
    /// parameter selects which variable name to use. If no name matches,
    /// the subtree is skipped (empty iterator).
    fn iter_node(
        node: &'a HttpRouterNode<Context>,
        version: Option<&'a Version>,
    ) -> Box<PathIter<'a, Context>> {
        match &node.edges {
            Some(HttpRouterEdges::Literals(map)) => Box::new(
                map.iter()
                    .map(|(s, node)| (PathSegment::Literal(s.clone()), node)),
            ),
            Some(HttpRouterEdges::VariableSingle(varnames, node)) => {
                match varnames.name_for_version(version) {
                    Some(name) => Box::new(std::iter::once((
                        PathSegment::VarnameSegment(name.to_string()),
                        node,
                    ))),
                    None => Box::new(std::iter::empty()),
                }
            }
            Some(HttpRouterEdges::VariableRest(varnames, node)) => {
                match varnames.name_for_version(version) {
                    Some(name) => Box::new(std::iter::once((
                        PathSegment::VarnameWildcard(name.to_string()),
                        node,
                    ))),
                    None => Box::new(std::iter::empty()),
                }
            }
            None => Box::new(std::iter::empty()),
        }
    }

    /// Produce a human-readable path from the current vector of path segments.
    fn path(&self) -> String {
        // Ignore the leading element as that's just a placeholder.
        let components: Vec<String> = self.path[1..]
            .iter()
            .map(|(c, _)| match c {
                PathSegment::Literal(s) => s.clone(),
                PathSegment::VarnameSegment(s) => format!("{{{}}}", s),
                PathSegment::VarnameWildcard(s) => format!("{{{}:.*}}", s),
            })
            .collect();

        // Prepend "/" to the "/"-delimited path.
        format!("/{}", components.join("/"))
    }
}

impl<'a, Context: ServerContext> Iterator for HttpRouterIter<'a, Context> {
    type Item = (String, String, &'a ApiEndpoint<Context>);

    fn next(&mut self) -> Option<Self::Item> {
        // If there are no path components left then we've reached the end of
        // our traversal. Making this case explicit isn't strictly required,
        // but is added for clarity.
        if self.path.is_empty() {
            return None;
        }

        loop {
            match self.method.next() {
                Some((m, ref e)) => break Some((self.path(), m.clone(), e)),
                None => {
                    // We've iterated fully through the method in this node so
                    // it's time to find the next node.
                    match self.path.last_mut() {
                        None => break None,
                        Some((_, ref mut last)) => match last.next() {
                            None => {
                                self.path.pop();
                                assert!(self.method.next().is_none());
                            }
                            Some((path_component, node)) => {
                                self.path.push((
                                    path_component,
                                    HttpRouterIter::iter_node(
                                        node,
                                        self.version,
                                    ),
                                ));
                                self.method = iter_handlers_from_node(
                                    &node,
                                    self.version,
                                );
                            }
                        },
                    }
                }
            }
        }
    }
}

/// Helper function for taking a Uri path and producing a `Vec<String>` of
/// URL-decoded strings, each representing one segment of the path. The input is
/// percent-encoded. Empty segments i.e. due to consecutive "/" characters or a
/// leading "/" are omitted.
///
/// Regarding "dot-segments" ("." and ".."), RFC 3986 section 3.3 says this:
///    The path segments "." and "..", also known as dot-segments, are
///    defined for relative reference within the path name hierarchy.  They
///    are intended for use at the beginning of a relative-path reference
///    (Section 4.2) to indicate relative position within the hierarchical
///    tree of names.  This is similar to their role within some operating
///    systems' file directory structures to indicate the current directory
///    and parent directory, respectively.  However, unlike in a file
///    system, these dot-segments are only interpreted within the URI path
///    hierarchy and are removed as part of the resolution process (Section
///    5.2).
///
/// While nothing prohibits APIs from including dot-segments. We see no strong
/// case for allowing them in paths, and plenty of pitfalls if we were to
/// require consumers to consider them (e.g. "GET /../../../etc/passwd"). Note
/// that consumers may be susceptible to other information leaks, for example
/// if a client were able to follow a symlink to the root of the filesystem. As
/// always, it is incumbent on the consumer and *critical* to validate input.
fn input_path_to_segments(path: &InputPath) -> Result<Vec<String>, String> {
    // We're given the "path" portion of a URI and we want to construct an
    // array of the segments of the path.   Relevant references:
    //
    //    RFC 7230 HTTP/1.1 Syntax and Routing
    //             (particularly: 2.7.3 on normalization)
    //    RFC 3986 Uniform Resource Identifier (URI): Generic Syntax
    //             (particularly: 6.2.2 on comparison)
    //
    // TODO-hardening We should revisit this.  We want to consider a couple of
    // things:
    // - what it means (and what we should do) if the path does not begin with
    //   a leading "/"
    // - how to handle paths that end in "/" (in some cases, ought this send a
    //   300-level redirect?)
    //
    // It would seem obvious to reach for the Rust "url" crate. That crate
    // parses complete URLs, which include a scheme and authority section that
    // does not apply here. We could certainly make one up (e.g.,
    // "http://127.0.0.1") and construct a URL whose path matches the path we
    // were given. However, while it seems natural that our internal
    // representation would not be percent-encoded, the "url" crate
    // percent-encodes any path that it's given. Further, we probably want to
    // treat consecutive "/" characters as equivalent to a single "/", but that
    // crate treats them separately (which is not unreasonable, since it's not
    // clear that the above RFCs say anything about whether empty segments
    // should be ignored). The net result is that that crate doesn't buy us
    // much here, but it does create more work, so we'll just split it
    // ourselves.
    path.0
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| match segment {
            "." | ".." => Err("dot-segments are not permitted".to_string()),
            _ => Ok(percent_decode_str(segment)
                .decode_utf8()
                .map_err(|e| e.to_string())?
                .to_string()),
        })
        .collect()
}

/// Whereas in `input_path_to_segments()` we must accommodate any user input, when
/// processing paths specified by the client program we can be more stringent and
/// fail via a panic! rather than an error. We do not percent-decode the path
/// meaning that programs may specify path segments that would require
/// percent-encoding by clients. Paths *must* begin with a "/"; only the final
/// segment may be empty i.e. the path may end with a "/".
pub fn route_path_to_segments(path: &str) -> Vec<&str> {
    if !matches!(path.chars().next(), Some('/')) {
        panic!("route paths must begin with a '/': '{}'", path);
    }
    let mut ret = path.split('/').skip(1).collect::<Vec<_>>();
    for segment in &ret[..ret.len() - 1] {
        if segment.is_empty() {
            panic!("path segments may not be empty: '{}'", path);
        }
    }

    // TODO pop off the last element if it's empty; today we treat a trailing
    // "/" as identical to a path without a trailing "/", but we may want to
    // support the distinction.
    if ret[ret.len() - 1] == "" {
        ret.pop();
    }
    ret
}

#[cfg(test)]
mod test {
    use super::super::error::HttpError;
    use super::super::handler::HttpRouteHandler;
    use super::super::handler::RequestContext;
    use super::super::handler::RouteHandler;
    use super::input_path_to_segments;
    use super::HttpRouter;
    use super::PathSegment;
    use crate::api_description::ApiEndpointBodyContentType;
    use crate::api_description::ApiEndpointVersions;
    use crate::from_map::from_map;
    use crate::router::VariableValue;
    use crate::ApiEndpoint;
    use crate::ApiEndpointResponse;
    use crate::Body;
    use http::Method;
    use http::StatusCode;
    use hyper::Response;
    use semver::Version;
    use serde::Deserialize;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    async fn test_handler(
        _: RequestContext<()>,
    ) -> Result<Response<Body>, HttpError> {
        panic!("test handler is not supposed to run");
    }

    fn new_handler() -> Arc<dyn RouteHandler<()>> {
        HttpRouteHandler::new(test_handler)
    }

    fn new_handler_named(name: &str) -> Arc<dyn RouteHandler<()>> {
        HttpRouteHandler::new_with_name(test_handler, name)
    }

    fn new_endpoint(
        handler: Arc<dyn RouteHandler<()>>,
        method: Method,
        path: &str,
    ) -> ApiEndpoint<()> {
        new_endpoint_versions(handler, method, path, ApiEndpointVersions::All)
    }

    fn new_endpoint_versions(
        handler: Arc<dyn RouteHandler<()>>,
        method: Method,
        path: &str,
        versions: ApiEndpointVersions,
    ) -> ApiEndpoint<()> {
        ApiEndpoint {
            operation_id: "test_handler".to_string(),
            handler,
            method,
            path: path.to_string(),
            parameters: vec![],
            body_content_type: ApiEndpointBodyContentType::default(),
            request_body_max_bytes: None,
            response: ApiEndpointResponse::default(),
            error: None,
            summary: None,
            description: None,
            tags: vec![],
            extension_mode: Default::default(),
            visible: true,
            deprecated: false,
            versions,
        }
    }

    #[test]
    #[should_panic(
        expected = "HTTP URI path segment variable name must not be empty"
    )]
    fn test_variable_name_empty() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(new_handler(), Method::GET, "/foo/{}"));
    }

    #[test]
    #[should_panic(
        expected = "HTTP URI path segment variable missing trailing \"}\""
    )]
    fn test_variable_name_bad_end() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/foo/{asdf/foo",
        ));
    }

    #[test]
    #[should_panic(
        expected = "HTTP URI path segment variable missing leading \"{\""
    )]
    fn test_variable_name_bad_start() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/foo/asdf}/foo",
        ));
    }

    #[test]
    #[should_panic(expected = "URI path \"/boo\": attempted to create \
                               duplicate route for method \"GET\"")]
    fn test_duplicate_route1() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(new_handler(), Method::GET, "/boo"));
        router.insert(new_endpoint(new_handler(), Method::GET, "/boo"));
    }

    #[test]
    #[should_panic(expected = "URI path \"/foo/bar/\": attempted to create \
                               duplicate route for method \"GET\"")]
    fn test_duplicate_route2() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(new_handler(), Method::GET, "/foo/bar"));
        router.insert(new_endpoint(new_handler(), Method::GET, "/foo/bar/"));
    }

    #[test]
    #[should_panic(expected = "path segments may not be empty: '//'")]
    fn test_duplicate_route3() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(new_handler(), Method::GET, "/"));
        router.insert(new_endpoint(new_handler(), Method::GET, "//"));
    }

    #[test]
    #[should_panic(expected = "URI path \"/boo\": attempted to create \
                               duplicate route for method \"GET\"")]
    fn test_duplicate_route_same_version() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint_versions(
            new_handler(),
            Method::GET,
            "/boo",
            ApiEndpointVersions::From(Version::new(1, 2, 3)),
        ));
        router.insert(new_endpoint_versions(
            new_handler(),
            Method::GET,
            "/boo",
            ApiEndpointVersions::From(Version::new(1, 2, 3)),
        ));
    }

    #[test]
    #[should_panic(expected = "URI path \"/boo\": attempted to register \
                               multiple handlers for method \"GET\" with \
                               overlapping version ranges")]
    fn test_duplicate_route_overlapping_version() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint_versions(
            new_handler(),
            Method::GET,
            "/boo",
            ApiEndpointVersions::From(Version::new(1, 2, 3)),
        ));
        router.insert(new_endpoint_versions(
            new_handler(),
            Method::GET,
            "/boo",
            ApiEndpointVersions::From(Version::new(4, 5, 6)),
        ));
    }

    #[test]
    #[should_panic(expected = "URI path \"/projects/{id}/insts/{id}\": \
                               variable name \"id\" is used more than once")]
    fn test_duplicate_varname() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/projects/{id}/insts/{id}",
        ));
    }

    #[test]
    #[should_panic(expected = "URI path \"/projects/{id}\": attempted to use \
                               variable name \"id\", but a different name \
                               (\"project_id\") is already used for the same \
                               path segment in an overlapping API version \
                               range")]
    fn test_inconsistent_varname() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/projects/{project_id}",
        ));
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/projects/{id}",
        ));
    }

    /// Different variable names at the same path segment are allowed when
    /// their version ranges don't overlap.
    #[test]
    fn test_versioned_varname_allowed() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint_versions(
            new_handler_named("h1"),
            Method::GET,
            "/thing/{arg}",
            // Note this is "until", so versions < 2.0.0.
            ApiEndpointVersions::until(Version::new(2, 0, 0)),
        ));
        // Different variable name for a non-overlapping version range.
        router.insert(new_endpoint_versions(
            new_handler_named("h2"),
            Method::GET,
            "/thing/{myarg}",
            // Note this is "from", so versions >= 2.0.0.
            ApiEndpointVersions::from(Version::new(2, 0, 0)),
        ));

        // Version 1.x uses "arg".
        let result = router
            .lookup_route(
                &Method::GET,
                "/thing/hello".into(),
                Some(&Version::new(1, 0, 0)),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h1");
        assert_eq!(
            *result.endpoint.variables.get("arg").unwrap(),
            VariableValue::String("hello".to_string())
        );
        assert!(!result.endpoint.variables.contains_key("myarg"));

        // Version 2.x uses "myarg".
        let result = router
            .lookup_route(
                &Method::GET,
                "/thing/hello".into(),
                Some(&Version::new(2, 0, 0)),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h2");
        assert_eq!(
            *result.endpoint.variables.get("myarg").unwrap(),
            VariableValue::String("hello".to_string())
        );
        assert!(!result.endpoint.variables.contains_key("arg"));
    }

    /// Different variable names at the same path segment should still be
    /// rejected when their version ranges overlap.
    #[test]
    #[should_panic(expected = "URI path \"/thing/{myarg}\": attempted to use \
                               variable name \"myarg\", but a different name \
                               (\"arg\") is already used for the same path \
                               segment in an overlapping API version range")]
    fn test_versioned_varname_conflict() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint_versions(
            new_handler_named("h1"),
            Method::GET,
            "/thing/{arg}",
            ApiEndpointVersions::from(Version::new(1, 0, 0)),
        ));
        // Overlapping version range with a different variable name.
        router.insert(new_endpoint_versions(
            new_handler_named("h2"),
            Method::GET,
            "/thing/{myarg}",
            ApiEndpointVersions::from(Version::new(2, 0, 0)),
        ));
    }

    /// Multiple endpoints can share a variable name at the same path segment,
    /// even when a different variable name is used for other versions.
    #[test]
    fn test_versioned_varname_shared_subtree() {
        let mut router = HttpRouter::new();
        // Two endpoints sharing "arg" in version 1.x, at different sub-paths.
        router.insert(new_endpoint_versions(
            new_handler_named("h1"),
            Method::GET,
            "/thing/{arg}/info",
            ApiEndpointVersions::until(Version::new(2, 0, 0)),
        ));
        router.insert(new_endpoint_versions(
            new_handler_named("h2"),
            Method::POST,
            "/thing/{arg}/info",
            ApiEndpointVersions::until(Version::new(2, 0, 0)),
        ));
        // Version 2.x uses a different name at the same position.
        router.insert(new_endpoint_versions(
            new_handler_named("h3"),
            Method::GET,
            "/thing/{myarg}/info",
            ApiEndpointVersions::from(Version::new(2, 0, 0)),
        ));

        let result = router
            .lookup_route(
                &Method::GET,
                "/thing/x/info".into(),
                Some(&Version::new(1, 0, 0)),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h1");
        assert_eq!(
            *result.endpoint.variables.get("arg").unwrap(),
            VariableValue::String("x".to_string())
        );

        let result = router
            .lookup_route(
                &Method::POST,
                "/thing/x/info".into(),
                Some(&Version::new(1, 0, 0)),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h2");

        let result = router
            .lookup_route(
                &Method::GET,
                "/thing/x/info".into(),
                Some(&Version::new(2, 0, 0)),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h3");
        assert_eq!(
            *result.endpoint.variables.get("myarg").unwrap(),
            VariableValue::String("x".to_string())
        );

        // Version 2.x has no POST handler.
        let error = router
            .lookup_route(
                &Method::POST,
                "/thing/x/info".into(),
                Some(&Version::new(2, 0, 0)),
            )
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::METHOD_NOT_ALLOWED);
    }

    /// Versioned variable names work for wildcard (rest) path segments too.
    #[test]
    fn test_versioned_varname_wildcard() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint_versions(
            new_handler_named("h1"),
            Method::GET,
            "/files/{path:.*}",
            ApiEndpointVersions::until(Version::new(2, 0, 0)),
        ));
        router.insert(new_endpoint_versions(
            new_handler_named("h2"),
            Method::GET,
            "/files/{filepath:.*}",
            ApiEndpointVersions::from(Version::new(2, 0, 0)),
        ));

        let result = router
            .lookup_route(
                &Method::GET,
                "/files/a/b/c".into(),
                Some(&Version::new(1, 0, 0)),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h1");
        assert_eq!(
            result.endpoint.variables.get("path"),
            Some(&VariableValue::Components(vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
            ]))
        );
        assert!(!result.endpoint.variables.contains_key("filepath"));

        let result = router
            .lookup_route(
                &Method::GET,
                "/files/a/b/c".into(),
                Some(&Version::new(2, 0, 0)),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h2");
        assert_eq!(
            result.endpoint.variables.get("filepath"),
            Some(&VariableValue::Components(vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
            ]))
        );
        assert!(!result.endpoint.variables.contains_key("path"));
    }

    /// When a version falls in a gap between two non-overlapping version
    /// ranges at a variable edge, lookup should return 404.
    #[test]
    fn test_versioned_varname_version_gap() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint_versions(
            new_handler_named("h1"),
            Method::GET,
            "/thing/{arg}/info",
            ApiEndpointVersions::until(Version::new(2, 0, 0)),
        ));
        router.insert(new_endpoint_versions(
            new_handler_named("h2"),
            Method::GET,
            "/thing/{myarg}/info",
            ApiEndpointVersions::from(Version::new(3, 0, 0)),
        ));

        // Version 1.x: matches the first endpoint.
        let result = router
            .lookup_route(
                &Method::GET,
                "/thing/x/info".into(),
                Some(&Version::new(1, 0, 0)),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h1");

        // Version 3.x: matches the second endpoint.
        let result = router
            .lookup_route(
                &Method::GET,
                "/thing/x/info".into(),
                Some(&Version::new(3, 0, 0)),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h2");

        // Version 2.x: falls in the gap — no variable name matches, so the
        // variable edge returns None and we get a 404.
        let error = router
            .lookup_route(
                &Method::GET,
                "/thing/x/info".into(),
                Some(&Version::new(2, 0, 0)),
            )
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
    }

    /// Same as above but for wildcard (rest) path segments, including the
    /// post-loop empty-path case where the wildcard matches zero segments.
    #[test]
    fn test_versioned_varname_wildcard_version_gap() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint_versions(
            new_handler_named("h1"),
            Method::GET,
            "/files/{path:.*}",
            ApiEndpointVersions::until(Version::new(2, 0, 0)),
        ));
        router.insert(new_endpoint_versions(
            new_handler_named("h2"),
            Method::GET,
            "/files/{filepath:.*}",
            ApiEndpointVersions::from(Version::new(3, 0, 0)),
        ));

        // Version 2.5 with path segments: the in-loop VariableRest branch
        // returns None.
        let error = router
            .lookup_route(
                &Method::GET,
                "/files/a/b".into(),
                Some(&Version::new(2, 0, 0)),
            )
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);

        // Version 2.5 with no trailing segments: the post-loop wildcard
        // rest-match hits the gap.
        let error = router
            .lookup_route(
                &Method::GET,
                "/files".into(),
                Some(&Version::new(2, 0, 0)),
            )
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
    }

    /// The iterator should produce the correct variable name for each version.
    #[test]
    fn test_versioned_varname_iter() {
        let mut router = HttpRouter::<()>::new();
        router.insert(new_endpoint_versions(
            new_handler_named("h1"),
            Method::GET,
            "/thing/{arg}",
            ApiEndpointVersions::until(Version::new(2, 0, 0)),
        ));
        router.insert(new_endpoint_versions(
            new_handler_named("h2"),
            Method::GET,
            "/thing/{myarg}",
            ApiEndpointVersions::from(Version::new(2, 0, 0)),
        ));

        let v1 = Version::new(1, 0, 0);
        let v1_endpoints: Vec<_> =
            router.endpoints(Some(&v1)).map(|x| (x.0, x.1)).collect();
        assert_eq!(
            v1_endpoints,
            vec![("/thing/{arg}".to_string(), "GET".to_string())]
        );

        let v2 = Version::new(2, 0, 0);
        let v2_endpoints: Vec<_> =
            router.endpoints(Some(&v2)).map(|x| (x.0, x.1)).collect();
        assert_eq!(
            v2_endpoints,
            vec![("/thing/{myarg}".to_string(), "GET".to_string())]
        );
    }

    /// The iterator should produce the correct variable name for each
    /// version, including the `:.*` suffix for wildcard path segments.
    #[test]
    fn test_versioned_varname_wildcard_iter() {
        let mut router = HttpRouter::<()>::new();
        router.insert(new_endpoint_versions(
            new_handler_named("h1"),
            Method::GET,
            "/files/{path:.*}",
            ApiEndpointVersions::until(Version::new(2, 0, 0)),
        ));
        router.insert(new_endpoint_versions(
            new_handler_named("h2"),
            Method::GET,
            "/files/{filepath:.*}",
            ApiEndpointVersions::from(Version::new(2, 0, 0)),
        ));

        let v1 = Version::new(1, 0, 0);
        let v1_endpoints: Vec<_> =
            router.endpoints(Some(&v1)).map(|x| (x.0, x.1)).collect();
        assert_eq!(
            v1_endpoints,
            vec![("/files/{path:.*}".to_string(), "GET".to_string())]
        );

        let v2 = Version::new(2, 0, 0);
        let v2_endpoints: Vec<_> =
            router.endpoints(Some(&v2)).map(|x| (x.0, x.1)).collect();
        assert_eq!(
            v2_endpoints,
            vec![("/files/{filepath:.*}".to_string(), "GET".to_string())]
        );
    }

    /// Regression test: when the wildcard edge's variable name doesn't match
    /// the requested version, the router must still advance to the wildcard's
    /// terminal node.  Otherwise, handler lookup would fall through to the
    /// parent node (which has a POST handler for all versions), producing a
    /// spurious 405 instead of 404.
    #[test]
    fn test_versioned_wildcard_rest_no_fallthrough() {
        let mut router = HttpRouter::new();

        // Wildcard GET endpoint, only for version 1.x.
        router.insert(new_endpoint_versions(
            new_handler_named("wildcard_get"),
            Method::GET,
            "/files/{path:.*}",
            ApiEndpointVersions::until(Version::new(2, 0, 0)),
        ));

        // Non-wildcard POST endpoint at /files, all versions.  This handler
        // lives on the parent node (the "files" literal node), NOT on the
        // wildcard's terminal node.
        router.insert(new_endpoint_versions(
            new_handler_named("files_post"),
            Method::POST,
            "/files",
            ApiEndpointVersions::All,
        ));

        // Version 1.x: GET /files should match the wildcard with empty path
        // components.
        let result = router
            .lookup_route(
                &Method::GET,
                "/files".into(),
                Some(&Version::new(1, 0, 0)),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "wildcard_get");
        assert_eq!(
            result.endpoint.variables.get("path"),
            Some(&VariableValue::Components(vec![]))
        );

        // Version 3.x: GET /files should be 404 (Not Found).  The wildcard
        // GET endpoint only covers versions < 2.0.0, and there is no
        // GET /files endpoint.  Without the unconditional advance to the
        // wildcard's terminal node, the router would stay on the parent
        // (which has the POST handler) and return 405 instead.
        let error = router
            .lookup_route(
                &Method::GET,
                "/files".into(),
                Some(&Version::new(3, 0, 0)),
            )
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
    }

    #[test]
    #[should_panic(expected = "URI path \"/projects/{id}\": attempted to \
                               register route for variable path segment \
                               (variable name: \"id\") when a route already \
                               exists for a literal path segment")]
    fn test_variable_after_literal() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/projects/default",
        ));
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/projects/{id}",
        ));
    }

    #[test]
    #[should_panic(expected = "URI path \"/projects/default\": attempted to \
                               register route for literal path segment \
                               \"default\" when a route exists for a variable \
                               path segment")]
    fn test_literal_after_variable() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/projects/{id}",
        ));
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/projects/default",
        ));
    }

    #[test]
    #[should_panic(expected = "URI path \"/projects/default\": attempted to \
                               register route for literal path segment \
                               \"default\" when a route exists for a variable \
                               path segment")]
    fn test_literal_after_regex() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/projects/{rest:.*}",
        ));
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/projects/default",
        ));
    }

    #[test]
    #[should_panic(expected = "Only the pattern '.*' is currently supported")]
    fn test_bogus_regex() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/word/{rest:[a-z]*}",
        ));
    }

    #[test]
    #[should_panic(expected = "URI path \"/some/{more:.*}/{stuff}\": \
                               attempted to match segments after the \
                               wildcard variable \"more\"")]
    fn test_more_after_regex() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/some/{more:.*}/{stuff}",
        ));
    }

    // TODO: We allow a trailing slash after the wildcard specifier, but we may
    // reconsider this if we decided to distinguish between the presence or
    // absence of the trailing slash.
    #[test]
    fn test_slash_after_wildcard_is_fine_dot_dot_dot_for_now() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler(),
            Method::GET,
            "/some/{more:.*}/",
        ));
    }

    #[test]
    fn test_error_cases() {
        let mut router = HttpRouter::new();

        // Check a few initial conditions.
        let error = router
            .lookup_route_unversioned(&Method::GET, "/".into())
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
        let error = router
            .lookup_route_unversioned(&Method::GET, "////".into())
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
        let error = router
            .lookup_route_unversioned(&Method::GET, "/foo/bar".into())
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
        let error = router
            .lookup_route_unversioned(&Method::GET, "//foo///bar".into())
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);

        // Insert a route into the middle of the tree.  This will let us look at
        // parent nodes, sibling nodes, and child nodes.
        router.insert(new_endpoint(new_handler(), Method::GET, "/foo/bar"));
        assert!(router
            .lookup_route_unversioned(&Method::GET, "/foo/bar".into())
            .is_ok());
        assert!(router
            .lookup_route_unversioned(&Method::GET, "/foo/bar/".into())
            .is_ok());
        assert!(router
            .lookup_route_unversioned(&Method::GET, "//foo/bar".into())
            .is_ok());
        assert!(router
            .lookup_route_unversioned(&Method::GET, "//foo//bar".into())
            .is_ok());
        assert!(router
            .lookup_route_unversioned(&Method::GET, "//foo//bar//".into())
            .is_ok());
        assert!(router
            .lookup_route_unversioned(&Method::GET, "///foo///bar///".into())
            .is_ok());

        // TODO-cleanup: consider having a "build" step that constructs a
        // read-only router and does validation like making sure that there's a
        // GET route on all nodes?
        let error = router
            .lookup_route_unversioned(&Method::GET, "/".into())
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
        let error = router
            .lookup_route_unversioned(&Method::GET, "/foo".into())
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
        let error = router
            .lookup_route_unversioned(&Method::GET, "//foo".into())
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
        let error = router
            .lookup_route_unversioned(&Method::GET, "/foo/bar/baz".into())
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);

        let error = router
            .lookup_route_unversioned(&Method::PUT, "/foo/bar".into())
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::METHOD_NOT_ALLOWED);
        let error = router
            .lookup_route_unversioned(&Method::PUT, "/foo/bar/".into())
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::METHOD_NOT_ALLOWED);

        // Check error cases that are specific (or handled differently) when
        // routes are versioned.
        let mut router = HttpRouter::new();
        router.insert(new_endpoint_versions(
            new_handler(),
            Method::GET,
            "/foo",
            ApiEndpointVersions::from(Version::new(1, 0, 0)),
        ));
        let error = router
            .lookup_route(
                &Method::GET,
                "/foo".into(),
                Some(&Version::new(0, 9, 0)),
            )
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
        let error = router
            .lookup_route(
                &Method::PUT,
                "/foo".into(),
                Some(&Version::new(1, 1, 0)),
            )
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn test_router_basic() {
        let mut router = HttpRouter::new();

        // Insert a handler at the root and verify that we get that handler
        // back, even if we use different names that normalize to "/".
        // Before we start, sanity-check that there's nothing at the root
        // already.  Other test cases examine the errors in more detail.
        assert!(router
            .lookup_route_unversioned(&Method::GET, "/".into())
            .is_err());
        router.insert(new_endpoint(new_handler_named("h1"), Method::GET, "/"));
        let result =
            router.lookup_route_unversioned(&Method::GET, "/".into()).unwrap();
        assert_eq!(result.handler.label(), "h1");
        assert!(result.endpoint.variables.is_empty());
        let result =
            router.lookup_route_unversioned(&Method::GET, "//".into()).unwrap();
        assert_eq!(result.handler.label(), "h1");
        assert!(result.endpoint.variables.is_empty());
        let result = router
            .lookup_route_unversioned(&Method::GET, "///".into())
            .unwrap();
        assert_eq!(result.handler.label(), "h1");
        assert!(result.endpoint.variables.is_empty());

        // Now insert a handler for a different method at the root.  Verify that
        // we get both this handler and the previous one if we ask for the
        // corresponding method and that we get no handler for a different,
        // third method.
        assert!(router
            .lookup_route_unversioned(&Method::PUT, "/".into())
            .is_err());
        router.insert(new_endpoint(new_handler_named("h2"), Method::PUT, "/"));
        let result =
            router.lookup_route_unversioned(&Method::PUT, "/".into()).unwrap();
        assert_eq!(result.handler.label(), "h2");
        assert!(result.endpoint.variables.is_empty());
        let result =
            router.lookup_route_unversioned(&Method::GET, "/".into()).unwrap();
        assert_eq!(result.handler.label(), "h1");
        assert!(result.endpoint.variables.is_empty());
        assert!(router
            .lookup_route_unversioned(&Method::DELETE, "/".into())
            .is_err());

        // Now insert a handler one level deeper.  Verify that all the previous
        // handlers behave as we expect, and that we have one handler at the new
        // path, whichever name we use for it.
        assert!(router
            .lookup_route_unversioned(&Method::GET, "/foo".into())
            .is_err());
        router.insert(new_endpoint(
            new_handler_named("h3"),
            Method::GET,
            "/foo",
        ));
        let result =
            router.lookup_route_unversioned(&Method::PUT, "/".into()).unwrap();
        assert_eq!(result.handler.label(), "h2");
        assert!(result.endpoint.variables.is_empty());
        let result =
            router.lookup_route_unversioned(&Method::GET, "/".into()).unwrap();
        assert_eq!(result.handler.label(), "h1");
        assert!(result.endpoint.variables.is_empty());
        let result = router
            .lookup_route_unversioned(&Method::GET, "/foo".into())
            .unwrap();
        assert_eq!(result.handler.label(), "h3");
        assert!(result.endpoint.variables.is_empty());
        let result = router
            .lookup_route_unversioned(&Method::GET, "/foo/".into())
            .unwrap();
        assert_eq!(result.handler.label(), "h3");
        assert!(result.endpoint.variables.is_empty());
        let result = router
            .lookup_route_unversioned(&Method::GET, "//foo//".into())
            .unwrap();
        assert_eq!(result.handler.label(), "h3");
        assert!(result.endpoint.variables.is_empty());
        let result = router
            .lookup_route_unversioned(&Method::GET, "/foo//".into())
            .unwrap();
        assert_eq!(result.handler.label(), "h3");
        assert!(result.endpoint.variables.is_empty());
        assert!(router
            .lookup_route_unversioned(&Method::PUT, "/foo".into())
            .is_err());
        assert!(router
            .lookup_route_unversioned(&Method::PUT, "/foo/".into())
            .is_err());
        assert!(router
            .lookup_route_unversioned(&Method::PUT, "//foo//".into())
            .is_err());
        assert!(router
            .lookup_route_unversioned(&Method::PUT, "/foo//".into())
            .is_err());
    }

    #[test]
    fn test_router_versioned() {
        // Install handlers for a particular route for a bunch of different
        // versions.
        //
        // This is not exhaustive because the matching logic is tested
        // exhaustively elsewhere.
        let method = Method::GET;
        let path: super::InputPath<'static> = "/foo".into();
        let mut router = HttpRouter::new();
        router.insert(new_endpoint_versions(
            new_handler_named("h1"),
            method.clone(),
            "/foo",
            ApiEndpointVersions::until(Version::new(1, 2, 3)),
        ));
        router.insert(new_endpoint_versions(
            new_handler_named("h2"),
            method.clone(),
            "/foo",
            ApiEndpointVersions::from_until(
                Version::new(2, 0, 0),
                Version::new(3, 0, 0),
            )
            .unwrap(),
        ));
        router.insert(new_endpoint_versions(
            new_handler_named("h3"),
            method.clone(),
            "/foo",
            ApiEndpointVersions::From(Version::new(5, 0, 0)),
        ));

        // Check what happens for a representative range of versions.
        let result = router
            .lookup_route(&method, path, Some(&Version::new(1, 2, 2)))
            .unwrap();
        assert_eq!(result.handler.label(), "h1");
        let error = router
            .lookup_route(&method, path, Some(&Version::new(1, 2, 3)))
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);

        let result = router
            .lookup_route(&method, path, Some(&Version::new(2, 0, 0)))
            .unwrap();
        assert_eq!(result.handler.label(), "h2");
        let result = router
            .lookup_route(&method, path, Some(&Version::new(2, 1, 0)))
            .unwrap();
        assert_eq!(result.handler.label(), "h2");

        let error = router
            .lookup_route(&method, path, Some(&Version::new(3, 0, 0)))
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
        let error = router
            .lookup_route(&method, path, Some(&Version::new(3, 0, 1)))
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);

        let error = router
            .lookup_route(&method, path, Some(&Version::new(4, 99, 99)))
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);

        let result = router
            .lookup_route(&method, path, Some(&Version::new(5, 0, 0)))
            .unwrap();
        assert_eq!(result.handler.label(), "h3");
        let result = router
            .lookup_route(&method, path, Some(&Version::new(128313, 0, 0)))
            .unwrap();
        assert_eq!(result.handler.label(), "h3");
    }

    #[test]
    fn test_embedded_non_variable() {
        // This isn't an important use case today, but we'd like to know if we
        // change the behavior, intentionally or otherwise.
        let mut router = HttpRouter::new();
        assert!(router
            .lookup_route_unversioned(&Method::GET, "/not{a}variable".into())
            .is_err());
        router.insert(new_endpoint(
            new_handler_named("h4"),
            Method::GET,
            "/not{a}variable",
        ));
        let result = router
            .lookup_route_unversioned(&Method::GET, "/not{a}variable".into())
            .unwrap();
        assert_eq!(result.handler.label(), "h4");
        assert!(result.endpoint.variables.is_empty());
        assert!(router
            .lookup_route_unversioned(&Method::GET, "/not{b}variable".into())
            .is_err());
        assert!(router
            .lookup_route_unversioned(&Method::GET, "/notnotavariable".into())
            .is_err());
    }

    #[test]
    fn test_variables_basic() {
        // Basic test using a variable.
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler_named("h5"),
            Method::GET,
            "/projects/{project_id}",
        ));
        assert!(router
            .lookup_route_unversioned(&Method::GET, "/projects".into())
            .is_err());
        assert!(router
            .lookup_route_unversioned(&Method::GET, "/projects/".into())
            .is_err());
        let result = router
            .lookup_route_unversioned(&Method::GET, "/projects/p12345".into())
            .unwrap();
        assert_eq!(result.handler.label(), "h5");
        assert_eq!(
            result.endpoint.variables.keys().collect::<Vec<&String>>(),
            vec!["project_id"]
        );
        assert_eq!(
            *result.endpoint.variables.get("project_id").unwrap(),
            VariableValue::String("p12345".to_string())
        );
        assert!(router
            .lookup_route_unversioned(
                &Method::GET,
                "/projects/p12345/child".into()
            )
            .is_err());
        let result = router
            .lookup_route_unversioned(&Method::GET, "/projects/p12345/".into())
            .unwrap();
        assert_eq!(result.handler.label(), "h5");
        assert_eq!(
            *result.endpoint.variables.get("project_id").unwrap(),
            VariableValue::String("p12345".to_string())
        );
        let result = router
            .lookup_route_unversioned(
                &Method::GET,
                "/projects///p12345//".into(),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h5");
        assert_eq!(
            *result.endpoint.variables.get("project_id").unwrap(),
            VariableValue::String("p12345".to_string())
        );
        // Trick question!
        let result = router
            .lookup_route_unversioned(
                &Method::GET,
                "/projects/{project_id}".into(),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h5");
        assert_eq!(
            *result.endpoint.variables.get("project_id").unwrap(),
            VariableValue::String("{project_id}".to_string())
        );
    }

    #[test]
    fn test_variables_multi() {
        // Exercise a case with multiple variables.
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler_named("h6"),
            Method::GET,
            "/projects/{project_id}/instances/{instance_id}/fwrules/\
             {fwrule_id}/info",
        ));
        let result = router
            .lookup_route_unversioned(
                &Method::GET,
                "/projects/p1/instances/i2/fwrules/fw3/info".into(),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h6");
        assert_eq!(
            result.endpoint.variables.keys().collect::<Vec<&String>>(),
            vec!["fwrule_id", "instance_id", "project_id"]
        );
        assert_eq!(
            *result.endpoint.variables.get("project_id").unwrap(),
            VariableValue::String("p1".to_string())
        );
        assert_eq!(
            *result.endpoint.variables.get("instance_id").unwrap(),
            VariableValue::String("i2".to_string())
        );
        assert_eq!(
            *result.endpoint.variables.get("fwrule_id").unwrap(),
            VariableValue::String("fw3".to_string())
        );
    }

    #[test]
    fn test_empty_variable() {
        // Exercise a case where a broken implementation might erroneously
        // assign a variable to the empty string.
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler_named("h7"),
            Method::GET,
            "/projects/{project_id}/instances",
        ));
        assert!(router
            .lookup_route_unversioned(
                &Method::GET,
                "/projects/instances".into()
            )
            .is_err());
        assert!(router
            .lookup_route_unversioned(
                &Method::GET,
                "/projects//instances".into()
            )
            .is_err());
        assert!(router
            .lookup_route_unversioned(
                &Method::GET,
                "/projects///instances".into()
            )
            .is_err());
        let result = router
            .lookup_route_unversioned(
                &Method::GET,
                "/projects/foo/instances".into(),
            )
            .unwrap();
        assert_eq!(result.handler.label(), "h7");
    }

    #[test]
    fn test_variables_glob() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler_named("h8"),
            Method::OPTIONS,
            "/console/{path:.*}",
        ));

        let result = router
            .lookup_route_unversioned(
                &Method::OPTIONS,
                "/console/missiles/launch".into(),
            )
            .unwrap();

        assert_eq!(
            result.endpoint.variables.get("path"),
            Some(&VariableValue::Components(vec![
                "missiles".to_string(),
                "launch".to_string()
            ]))
        );
    }

    #[test]
    fn test_variable_rename() {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct MyPath {
            #[serde(rename = "type")]
            t: String,
            #[serde(rename = "ref")]
            r: String,
            #[serde(rename = "@")]
            at: String,
        }

        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler_named("h8"),
            Method::OPTIONS,
            "/{type}/{ref}/{@}",
        ));

        let result = router
            .lookup_route_unversioned(
                &Method::OPTIONS,
                "/console/missiles/launch".into(),
            )
            .unwrap();

        let path =
            from_map::<MyPath, VariableValue>(&result.endpoint.variables)
                .unwrap();

        assert_eq!(path.t, "console");
        assert_eq!(path.r, "missiles");
        assert_eq!(path.at, "launch");
    }

    #[test]
    fn test_iter_null() {
        let router = HttpRouter::<()>::new();
        let ret: Vec<_> = router.endpoints(None).map(|x| (x.0, x.1)).collect();
        assert_eq!(ret, vec![]);
    }

    #[test]
    fn test_iter() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler_named("root"),
            Method::GET,
            "/",
        ));
        router.insert(new_endpoint(
            new_handler_named("i"),
            Method::GET,
            "/projects/{project_id}/instances",
        ));
        let ret: Vec<_> = router.endpoints(None).map(|x| (x.0, x.1)).collect();
        assert_eq!(
            ret,
            vec![
                ("/".to_string(), "GET".to_string(),),
                (
                    "/projects/{project_id}/instances".to_string(),
                    "GET".to_string(),
                ),
            ]
        );
    }

    #[test]
    fn test_iter2() {
        let mut router = HttpRouter::new();
        router.insert(new_endpoint(
            new_handler_named("root_get"),
            Method::GET,
            "/",
        ));
        router.insert(new_endpoint(
            new_handler_named("root_post"),
            Method::POST,
            "/",
        ));
        let ret: Vec<_> = router.endpoints(None).map(|x| (x.0, x.1)).collect();
        assert_eq!(
            ret,
            vec![
                ("/".to_string(), "GET".to_string(),),
                ("/".to_string(), "POST".to_string(),),
            ]
        );
    }

    #[test]
    fn test_segments() {
        let segs =
            input_path_to_segments(&"//foo/bar/baz%2fbuzz".into()).unwrap();
        assert_eq!(segs, vec!["foo", "bar", "baz/buzz"]);
    }

    #[test]
    fn test_path_segment() {
        let seg = PathSegment::from("abc");
        assert_eq!(seg, PathSegment::Literal("abc".to_string()));

        let seg = PathSegment::from("{words}");
        assert_eq!(seg, PathSegment::VarnameSegment("words".to_string()));

        let seg = PathSegment::from("{rest:.*}");
        assert_eq!(seg, PathSegment::VarnameWildcard("rest".to_string()),);
    }

    #[test]
    #[should_panic]
    fn test_bad_path_segment1() {
        let _ = PathSegment::from("{foo");
    }

    #[test]
    #[should_panic]
    fn test_bad_path_segment2() {
        let _ = PathSegment::from("bar}");
    }

    #[test]
    #[should_panic]
    fn test_bad_path_segment3() {
        let _ = PathSegment::from("{}");
    }

    #[test]
    #[should_panic]
    fn test_bad_path_segment4() {
        let _ = PathSegment::from("{varname:abc+}");
    }

    #[test]
    fn test_map() {
        #[derive(Deserialize)]
        struct A {
            bbb: String,
            ccc: Vec<String>,
        }

        let mut map = BTreeMap::new();
        map.insert(
            "bbb".to_string(),
            VariableValue::String("doggos".to_string()),
        );
        map.insert(
            "ccc".to_string(),
            VariableValue::Components(vec![
                "lizzie".to_string(),
                "brickley".to_string(),
            ]),
        );

        match from_map::<A, VariableValue>(&map) {
            Ok(a) => {
                assert_eq!(a.bbb, "doggos");
                assert_eq!(a.ccc, vec!["lizzie", "brickley"]);
            }
            Err(s) => panic!("unexpected error: {}", s),
        }
    }

    #[test]
    fn test_map_bad_value() {
        #[allow(dead_code)]
        #[derive(Deserialize)]
        struct A {
            bbb: String,
        }

        let mut map = BTreeMap::new();
        map.insert(
            "bbb".to_string(),
            VariableValue::Components(vec![
                "lizzie".to_string(),
                "brickley".to_string(),
            ]),
        );

        match from_map::<A, VariableValue>(&map) {
            Ok(_) => panic!("unexpected success"),
            Err(s) => {
                assert_eq!(s, "cannot deserialize sequence as a single value")
            }
        }
    }

    #[test]
    fn test_map_bad_seq() {
        #[allow(dead_code)]
        #[derive(Deserialize)]
        struct A {
            bbb: Vec<String>,
        }

        let mut map = BTreeMap::new();
        map.insert(
            "bbb".to_string(),
            VariableValue::String("doggos".to_string()),
        );

        match from_map::<A, VariableValue>(&map) {
            Ok(_) => panic!("unexpected success"),
            Err(s) => {
                assert_eq!(s, "cannot deserialize a single value as a sequence")
            }
        }
    }
}
