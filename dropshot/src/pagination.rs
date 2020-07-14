// Copyright 2020 Oxide Computer Company
/*!
 * Support for paginated resources
 *
 * "Pagination" here refers to the interface pattern where HTTP resources (or
 * API endpoints) that provide a list of the items in a collection return a
 * relatively small maximum number of items per request, often called a "page"
 * of results.  Each page includes some metadata that the client can use to make
 * another request for the next page of results.  The client can repeat this
 * until they've gotten all the results.  Limiting the number of results
 * returned per request helps bound the resource utilization and time required
 * for any request, which in turn facilities horizontal scalability, high
 * availability, and protection against some denial of service attacks
 * (intentional or otherwise).
 *
 * For a well-commented example of how to use the interface here, see
 * examples/pagination-basic.rs.  The rest of this comment describes the design
 * and implementation considerations for this interface.
 *
 *
 * ## Patterns for pagination
 *
 * A common pattern for paginating APIs looks something like this:
 *
 * Say we have a resource called a "Project" and a resource `GET /projects` to
 * list all the Projects in the system.  There's a default limit (e.g., 100
 * projects returned at a time).  Clients can request a higher limit using a
 * query parameter (e.g., `?limit=1000`).  This limit is capped by a hard limit
 * on the server.  If the client asks for more than the hard limit, the server
 * can use the hard limit or reject the request.
 *
 * In each response, along with the actual page of results, the server includes
 * some kind of token that can be used to get the next page.  The client usually
 * sends this back in the next request as another query parameter, like
 * `?page_token=abc123`.  [Google describes this approach in their own API
 * design guidelines][1].
 *
 * There are many ways to implement this token with many different tradeoffs.
 * For scalable HTTP APIs, a common approach is to make sure each resource has
 * at least one unique identifier, sort results by that field, and use the id of
 * the last item on the page as the token for the next page.  In our case,
 * suppose each project has a unique user-defined "name" that's a UTF-8 string.
 * Then we'd say that `GET /projects` returns results sorted by name.  When
 * clients pass `token=abc123`, the server returns results starting from the
 * first Project whose name sorts after "abc123".  In the response, the server
 * specifies the name of the last Project on the page as the token for the next
 * request.  This approach has a lot of nice properties:
 *
 * * For APIs backed by a database of some kind, it's usually straightforward to
 *   use an existing primary key or other unique, sortable field as the token.
 *
 * * If the client scans all the way through the collection, they will see every
 *   object that existed both before the scan and after the scan and was not
 *   renamed during the scan.  (This isn't true for schemes that use a simple
 *   numeric offset as the token.)
 *
 * * There's no server-side state associated with the token, so it's no problem
 *   if the server crashes between requests or if subsequent requests are
 *   handled by a different instance.  (This isn't true for schemes that store
 *   the result set on the server.)
 *
 * * It's often straightforward to support a reversed-order scan as well -- this
 *   may just be a matter of flipping the inequality used for a database query.
 *
 * * With some care, it's possible to support queries on multiple different
 *   fields, even at the same time.  An API can support listing by any unique,
 *   sortable combination of fields.  For example, say our Projects have a
 *   modification time ("mtime") as well.  We could support listing projects
 *   alphabetically by name _or_ in order of most recently modified.  For the
 *   latter, since the modification time is generally not unique, and the marker
 *   must be unique, we'd really be listing by an ("mtime" descending, "name"
 *   ascending) tuple.
 *
 * The interfaces here are intended to support this sort of use case.  For APIs
 * backed by traditional RDBMS databases, see [this post for background on
 * various ways to page through a large set of data][2].  (What we're describing
 * here leverages what this post calls "keyset pagination".)
 *
 * Another consideration in designing pagination is whether the token ought to
 * be explicit and meaningful to the user or just an opaque token (likely
 * encoded in some way).  It can be convenient for developers to use APIs where
 * the token is explicitly intended to be one of the fields of the object (e.g.,
 * so that you could list projects by `?project_name=myprojectname`), but it
 * puts constraints on the server because clients may come to depend on specific
 * fields being supported and sorted in a certain way.  Dropshot takes the
 * approach of using an encoded token that includes information about the whole
 * scan (e.g., the sort order).  This makes it possible to identify cases that
 * might otherwise result in confusing behavior (e.g., a client lists projects
 * in ascending order, but then asks for the next page in descending order).
 * The token also includes a version number so that it can be evolved in the
 * future.
 *
 *
 * ## Background: Why paginate HTTP APIs?
 *
 * Pagination helps ensure that the cost of a request in terms of resource
 * utilization remains O(1) -- that is, it can be bounded above by a constant
 * rather than scaling proportionally with any of the request parameters.  This
 * simplifies utilization monitoring, capacity planning, and scale-out
 * activities for the service, since operators can think of the service in terms
 * of one unit that needs to be scaled up.  (It's still a very complex process,
 * but it would be significantly harder if requests weren't O(1).)
 *
 * Similarly, pagination helps ensure that the time required for a request is
 * O(1) under normal conditions.  This makes it easier to define expectations
 * for service latency and to monitor that latency to determine if those
 * expectations are violated.  Generally, if latency increases, then the service
 * is unhealthy, and a crisp definition of "unhealthy" is important to operate a
 * service with high availability.  If requests weren't O(1), an increase in
 * latency might just reflect a changing workload that's still performing within
 * expectations -- e.g., clients listing larger collections than they were
 * before, but still getting results promptly.  That would make it much harder
 * to see when the service really is unhealthy.
 *
 * Finally, bounding requests to O(1) work is a critical mitigation for common
 * (if basic) denial-of-service (DoS) attacks because it requires that clients
 * consume resources commensurate with the server costs that they're imposing.
 * If a service exposed an API that does an unbounded amount of work
 * proportional to some parameter, then it's cheap to launch a DoS on the
 * service by just invoking that API with a large parameter.  By contrast, if
 * the client has to do work that scales linearly with the work the server has
 * to do, then the client's costs go up in order to scale up the attack.
 *
 * Along these lines, connections and requests consume finite server resources
 * like open file descriptors and database connections.  If a service is built
 * so that requests are all supposed to take about the same amount of time (or
 * at least that there's a constant upper bound), then it may be possible to use
 * a simple timeout scheme to cancel requests that are taking too long, as might
 * happen if a malicious client finds some way to do this.
 *
 * [1]: https://cloud.google.com/apis/design/design_patterns#list_pagination
 * [2]: https://www.citusdata.com/blog/2016/03/30/five-ways-to-paginate/
 */

use crate as dropshot; // XXX needed for ExtractedParameter below
use crate::error::HttpError;
use crate::handler::ExtractedParameter;
use base64::URL_SAFE;
use dropshot_endpoint::ExtractedParameter;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use std::convert::TryFrom;
use std::fmt::Debug;
use std::num::NonZeroU64;

pub trait PaginatedResource: 'static {
    type ScanMode: Debug
        + DeserializeOwned
        + ExtractedParameter
        + Send
        + Sync
        + 'static;
    type PageSelector: Debug
        + DeserializeOwned
        + ExtractedParameter
        + Serialize
        + Send
        + Sync
        + 'static;
    type Item: Serialize + JsonSchema + Send + Sync + 'static;

    fn page_selector_for(
        i: &Self::Item,
        p: &Self::ScanMode,
    ) -> Self::PageSelector;
}

/**
 * Client's view of a page of results from a paginated API
 */
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
pub struct ClientPage<ItemType> {
    /** token to be used in pagination params for the next page of results */
    pub next_page: Option<String>,
    /** list of items on this page of results */
    pub items: Vec<ItemType>,
}

#[derive(Debug, Deserialize, ExtractedParameter)]
#[serde(untagged)]
// XXX document that the order of these affects deserialization because tagging
// is off
pub enum WhichPage<P: PaginatedResource> {
    NextPage { page_token: PageToken<P::PageSelector> },
    FirstPage { list_mode: Option<P::ScanMode> },
}

#[derive(Debug, Deserialize, ExtractedParameter)]
pub struct PaginationParams<P: PaginatedResource> {
    #[serde(flatten)]
    pub page_params: WhichPage<P>,
    //
    // XXX consider implementing a default() that gives this a value?  It'd be
    // nice if this were runtime-configurable.  Another way to effect this might
    // be to have consumers use a helper function on the requestcontext, like
    // rqctx.pagination_limit(pag_params, Option<...>).  This would prefer the
    // client's requested limit, then the limit passed as an argument (if
    // present), and then the server-wide limit (which would be required).  Or
    // the server-wide limit could be optional and we'd use a compile-time
    // default if that wasn't specified.
    //
    pub limit: Option<NonZeroU64>,
}

/**
 * The order in which the client wants to page through the requested collection
 */
#[derive(Copy, Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PaginationOrder {
    Ascending,
    Descending,
}

/*
 * Token serialization and deserialization
 *
 * Page tokens essentially take the consumer's PageSelector struct, add a
 * version number, serialize that as JSON, and base64-encode the result.  This
 * token is returned in any response from a paginated API, and the client will
 * pass it back as a query parameter for subsequent pagination requests. This
 * approach allows us to rev the serialized form if needed (see
 * `PaginationVersion`) and add other metadata in a backwards-compatiable way.
 * It also emphasizes to clients that the token should be treated as opaque.
 */

/**
 * Maximum length of a page token once the consumer-provided type is serialized
 * and the result is base64-encoded
 *
 * We impose a maximum length primarily to prevent a client from making us parse
 * extremely large strings.  We apply this limit when we create tokens to avoid
 * handing out a token that can't be used.
 *
 * Note that these tokens are passed in the HTTP request line (before the
 * headers), and many HTTP implementations impose an implicit limit as low as
 * 8KiB on the size of the request line and headers together, so it's a good
 * idea to keep this as small as we can.
 */
const MAX_TOKEN_LENGTH: usize = 512;

/**
 * Version for the pagination token serialization format
 *
 * This may seem like overkill, but it allows us to rev this in a future version
 * of Dropshot without breaking any ongoing scans when the change is deployed.
 * If we rev this, we might need to provide a way for clients to request at
 * runtime which version of token to generate so that if they do a rolling
 * upgrade of multiple instances, they can configure the instances to generate
 * v1 tokens until the rollout is complete, then switch on the new token
 * version.  Obviously, it would be better to avoid revving this version if
 * possible!
 *
 * Note that consumers still need to consider compatibility if they change their
 * own `ScanMode` or `PageSelector` types.
 */
#[derive(
    Copy,
    Clone,
    Debug,
    Deserialize,
    ExtractedParameter,
    JsonSchema,
    PartialEq,
    Serialize,
)]
#[serde(rename_all = "lowercase")]
enum PaginationVersion {
    V1,
}

/**
 * Parts of the pagination token that actually get serialized.
 */
#[derive(Debug, Deserialize, Serialize)]
struct SerializedToken<PageSelector> {
    v: PaginationVersion,
    page_start: PageSelector,
}

/**
 * Describes the current scan and the client's position within it
 */
#[derive(Debug, Deserialize, ExtractedParameter)]
#[serde(try_from = "String")]
#[serde(bound(deserialize = "PageSelector: DeserializeOwned"))]
pub struct PageToken<PageSelector> {
    pub page_start: PageSelector,
}

impl<PageSelector> PageToken<PageSelector>
where
    PageSelector: Serialize,
{
    pub fn new(page_start: PageSelector) -> Self {
        PageToken {
            page_start,
        }
    }

    pub(crate) fn to_serialized(self) -> Result<String, HttpError> {
        let page_start = self.page_start;

        let marker_bytes = {
            let serialized_marker = SerializedToken {
                v: PaginationVersion::V1,
                page_start: page_start,
            };

            let json_bytes =
                serde_json::to_vec(&serialized_marker).map_err(|e| {
                    HttpError::for_internal_error(format!(
                        "failed to serialize marker: {}",
                        e
                    ))
                })?;

            base64::encode_config(json_bytes, URL_SAFE)
        };

        /*
         * TODO-robustness is there a way for us to know at compile-time that
         * this won't be a problem?  What if we say that MarkerFields has to be
         * Sized?  That won't guarantee that this will work, but wouldn't that
         * mean that if it ever works, then it will always work?  But would that
         * interface be a pain to use, given that variable-length strings are a
         * very common marker?
         */
        if marker_bytes.len() > MAX_TOKEN_LENGTH {
            return Err(HttpError::for_internal_error(format!(
                "serialized token is too large ({} bytes, max is {})",
                marker_bytes.len(),
                MAX_TOKEN_LENGTH
            )));
        }

        Ok(marker_bytes)
    }
}

impl<PageSelector> TryFrom<String> for PageToken<PageSelector>
where
    PageSelector: DeserializeOwned,
{
    type Error = String;

    fn try_from(value: String) -> Result<PageToken<PageSelector>, String> {
        if value.len() > MAX_TOKEN_LENGTH {
            return Err(String::from(
                "failed to parse pagination token: too large",
            ));
        }

        let json_bytes = base64::decode_config(value.as_bytes(), URL_SAFE)
            .map_err(|e| format!("failed to parse pagination token: {}", e))?;

        /*
         * TODO-debugging: we don't want the user to have to know about the
         * internal structure of the token, so the error message here doesn't
         * say anything about that.  However, it would be nice if we could
         * create an internal error message that included the serde_json error,
         * which would have more context for someone looking at the server logs
         * to figure out what happened with this request.  Our own `HttpError`
         * supports this, but it seems like serde only preserves the to_string()
         * output of the error anyway.  It's not clear how else we could
         * propagate this information out.
         */
        let deserialized: SerializedToken<PageSelector> =
            serde_json::from_slice(&json_bytes).map_err(|_| {
                format!("failed to parse pagination token: corrupted token")
            })?;

        if deserialized.v != PaginationVersion::V1 {
            return Err(format!(
                "failed to parse pagination token: unsupported version: {:?}",
                deserialized.v,
            ));
        }

        Ok(PageToken {
            page_start: deserialized.page_start,
        })
    }
}
