// Copyright 2020 Oxide Computer Company
/*
 * TODO-doc this won't wind up in the public docs so some of this content should
 * go elsewhere.  Maybe on PaginationParams?
 */
/*!
 *
 * # Support for paginated resources
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
 *
 * ## Example of pagination in HTTP
 *
 * Pagination support in Dropshot implements this common pattern:
 *
 * * This server exposes an **API endpoint** that returns the **items**
 *   contained within a **collection**.
 * * The client is not allowed to list the entire collection in one request.
 *   Instead, they list the collection using a sequence of requests to the one
 *   endpoint.  We call this sequence of requests a **scan** of the collection,
 *   and we sometimes say that the client **pages through** the collection.
 * * The initial request in the scan may specify the **scan mode**, which
 *   typically specifies how the results are to be sorted (i.e., by which
 *   field(s) and whether the sort is ascending or descending).
 * * Each request returns a **page** of results at a time, along with a **page
 *   token** that's provided with the next request as a query parameter.
 * * The scan mode cannot change between requests that are part of the same
 *   scan.
 * * With all requests: there's a default limit (e.g., 100 items returned at a
 *   time).  Clients can request a higher limit using a query parameter (e.g.,
 *   `limit=1000`).  This limit is capped by a hard limit on the server.  If the
 *   client asks for more than the hard limit, the server can use the hard limit
 *   or reject the request.
 *
 * As an example, imagine that we have an API endpoint called "/animals".  Each
 * item returned is an "Animal" object that might look like this:
 *
 * ```json
 * {
 *     "name": "aardvark",
 *     "class": "mammal",
 *     "max_weight": "80", /* kilograms, typical */
 * }
 * ```
 *
 * There are at least 1.5 million known species of animal -- too many to return
 * in one API call!  Our API supports paginating them by "name", which we'll say
 * is a unique field in our data set.
 *
 * The first request to the API fetches "/animals" (with no querystring
 * parameters) and returns:
 *
 * ```json
 * {
 *     "page_token": "abc123...",
 *     "items": [
 *         {
 *             "name": "aardvark",
 *             "class": "mammal",
 *             "max_weight": "80",
 *         },
 *         ...
 *         {
 *             "name": "badger",
 *             "class": "mammal",
 *             "max_weight": "12",
 *         }
 *     ]
 * }
 * ```
 *
 * The subsequent request to the API fetches "/animals?page_token=abc123...".
 * The page token `"abc123..."` is an opaque token to the client, but typically
 * encodes the scan mode ("sorting by name in ascending order") and the value of
 * the last item seen ("name=badger").  The client knows it has completed the
 * scan when it receives a response with no `page_token` in it.
 *
 * Our API endpoint can also support scanning in reverse order.  In this case,
 * when the client makes the first request, it should fetch
 * `"/animals?list_mode=name-descending".  (We use "list_mode" synonymously with
 * "scan_mode".)  Now, the first result might be "zebra".  Again, the page token
 * must include the scan mode so that in subsequent requests, the API endpoint
 * knows that we're scanning backwards, not forwards, from the value we were
 * given.  It's not allowed to change directions or sort order in the middle of
 * a scan.  (You can always start a new scan, but you can't pick up from where
 * you were in the previous scan.)
 *
 * It's also possible to support sorting by multiple fields.  For example, we
 * could support `list_mode=class-name`, which we could define to mean that
 * we'll sort the results first by class, then by name.  Thus we'd get all the
 * amphibians in sorted order, then all the mammals, then all the reptiles.
 * The main requirement is that the combination of fields used for pagination
 * must be unique.  We cannot paginate by the animal's class alone.  (To see
 * why: there are over 6,000 mammals.  If the page size is, say, 1000, then the
 * page_token would say "mammal", but there's not enough information there to
 * see where we are within the list of mammals.  It doesn't matter whether there
 * are 2 mammals or 6,000 because clients can limit the page size to just one
 * item if they want and that ought to work.)
 *
 *
 * ## Dropshot interfaces for pagination
 *
 * We can think of pagination in two parts: the input (handling the pagination
 * query parameters) and the output (emitting a page of results, including the
 * page token).
 *
 * For input, a paginated API endpoint's handler function should accept a
 * `Query<PaginationParams<ScanMode, PageSelector>>`, where `ScanMode` is a
 * consumer-defined type describing the allowed values of `list_mode` (usually
 * represented by an enum with no data) and `PageSelector` is a consumer-defined
 * type describing the page token.  The PageSelector will be serialized to JSON
 * and base64-encoded to construct the page token.
 *
 * For output, a paginated API endpoint's handler function should return
 * `Result<HttpResponseOkPage<ItemType>>`, where `ItemType` is whatever object
 * the API returns a list of.  In our case, `ItemType` would be `Animal`.
 *
 * For the simple case of paginating a collection by a single unique field, we
 * provide the helper type `MarkerPaginator<FieldType>` that you can use in
 * place of `PaginationParams`.  This provides a scan mode with only a single
 * option that corresponds to a scan sorted in increasing order of your field,
 * which has type `FieldType`.  (The client would generally never use
 * "list_mode" in this case because there's only one option and it's the default
 * anyway.)  `MarkerPaginator` also provides a page selector containing the
 * last-seen value of your field.
 *
 * For a complete, documented example of the simpler `MarkerPaginator`
 * interface, see examples/pagination-marker.rs.  For more complex examples, see
 * the other pagination examples in that directory.
 *
 *
 * ## Advanced usage notes
 *
 * It's possible to accept additional query parameters besides the pagination
 * parameters by creating your own struct `MyQueryParams` and embedding a
 * `PaginationParams` in it, like this:
 *
 * ```
 * use dropshot::ExtractedParameter;
 * use dropshot::PaginationParams;
 * use dropshot::Query;
 * use serde::Deserialize;
 * # use serde::Serialize;
 * # #[derive(Debug, Deserialize, ExtractedParameter)]
 * # enum MyScanMode { A };
 * # #[derive(Debug, Deserialize, ExtractedParameter, Serialize)]
 * # enum MyPageSelector { A(String) };
 * ##[derive(Deserialize, ExtractedParameter)]
 * struct MyQueryParams {
 *     do_extra_stuff: bool,
 *
 *     ##[serde(flatten)]
 *     pagination_params: PaginationParams<MyScanMode, MyPageSelector>
 * }
 * ```
 *
 * We use `#[serde(flatten)]` here so that the pagination parameters don't wind
 * up nested under the name "pagination_params".
 *
 *
 * ## Background: patterns for pagination
 *
 * [Google describes an approach similar to the one above in their own API
 * design guidelines][1].  There are many ways to implement the page token with
 * many different tradeoffs.  The one described above has a lot of nice
 * properties:
 *
 * * For APIs backed by a database of some kind, it's usually straightforward to
 *   use an existing primary key or other unique, sortable field (or combination
 *   of fields) as the token.
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
 * * It's easy to support sorting by a single field, and with some care it's
 *   possible to support queries on multiple different fields, even at the same
 *   time.  An API can support listing by any unique, sortable combination of
 *   fields.  For example, say our Projects have a modification time ("mtime")
 *   as well.  We could support listing projects alphabetically by name _or_ in
 *   order of most recently modified.  For the latter, since the modification
 *   time is generally not unique, and the marker must be unique, we'd really be
 *   listing by an ("mtime" descending, "name" ascending) tuple.
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
 * so that you could list animals starting in the middle by just requesting
 * `?animal_name=moose`), but this puts constraints on the server because
 * clients may come to depend on specific fields being supported and sorted in a
 * certain way.  Dropshot takes the approach of using an encoded token that
 * includes information about the whole scan (e.g., the sort order).  This makes
 * it possible to identify cases that might otherwise result in confusing
 * behavior (e.g., a client lists projects in ascending order, but then asks for
 * the next page in descending order).  The token also includes a version number
 * so that it can be evolved in the future.
 *
 *
 * ## Background: Why paginate HTTP APIs in the first place?
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
 * If a service exposes an API that does an unbounded amount of work
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
 * happen if a malicious client finds some way to cause requests to hang or take
 * a very long time.
 *
 * [1]: https://cloud.google.com/apis/design/design_patterns#list_pagination
 * [2]: https://www.citusdata.com/blog/2016/03/30/five-ways-to-paginate/
 */

use crate as dropshot; // XXX needed for ExtractedParameter below
use crate::error::HttpError;
use base64::URL_SAFE;
use dropshot_endpoint::ExtractedParameter;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use std::convert::TryFrom;
use std::fmt::Debug;
use std::num::NonZeroU64;

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

/**
 * Pagination interface for simple cases where a collection is sorted in one
 * direction by one Serializable field (called the marker)
 *
 * This is simpler to use than `PaginationParams` directly, but it's less
 * flexible.  See examples/pagination-marker.rs for details.
 */
pub type MarkerPaginator<FieldType> =
    PaginationParams<MarkerScanParams, MarkerPageSelector<FieldType>>;

/**
 * Internal type used as the `ScanMode` type for `MarkerPaginator`.
 */
#[derive(Debug, Deserialize, ExtractedParameter)]
pub struct MarkerScanParams {}

/**
 * Internal type used as the `PageSelector` type for `MarkerPaginator`.
 */
#[derive(Debug, Deserialize, ExtractedParameter, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkerPageSelector<F> {
    Marker(F),
}

/*
 * Generic pagination infrastructure
 * XXX need to better understand the trait bounds / serde bounds here.
 */

/**
 * Query parameters provided by clients when scanning a paginated collection
 *
 * See `PaginatedResource` for more information.
 */
#[derive(Debug, Deserialize, ExtractedParameter)]
#[serde(bound(
    deserialize = "PageSelector: DeserializeOwned, ScanMode: DeserializeOwned"
))]
#[serde(try_from = "RawPaginationParams<ScanMode>")]
pub struct PaginationParams<ScanMode, PageSelector>
where
    ScanMode: Debug + Send + Sync + 'static,
    PageSelector: Serialize + Send + Sync + 'static,
{
    /**
     * Specifies either how the client wants to begin a new scan or how to
     * resume a previous scan.  This field is flattened by serde, so see the
     * variants of [`WhichPage`] to see which fields actually show up in the
     * serialized form.
     */
    #[serde(flatten)]
    pub page_params: WhichPage<ScanMode, PageSelector>,

    /**
     * Optional client-requested limit on page size.  Consumers should use
     * [`dropshot::RequestContext::page_limit()`] to access this value.
     */
    pub(crate) limit: Option<NonZeroU64>,
}

/**
 * Indicates whether the client is beginning a new scan or resuming an existing
 * one and provides the corresponding parameters for each case
 */
/*
 * In REST APIs, callers typically provide either the parameters to resume a
 * scan or the parameters to begin a new one, with no separate field indicating
 * which case they're requesting.  Fortunately, serde(untagged) implements this
 * behavior precisely, with one caveat: the variants below are tried in order
 * until one succeeds.  So for this to work, `NextPage` must be defined before
 * `FirstPage`, since any set of parameters at all (including no parameters at
 * all) are valid for `FirstPage`.
 */
//#[derive(Debug, Deserialize, ExtractedParameter)]
//#[serde(bound(
//    deserialize = "PageSelector: DeserializeOwned, ScanMode: DeserializeOwned"
//))]
#[derive(Debug)]
pub enum WhichPage<ScanMode, PageSelector>
where
    ScanMode: Debug + Send + Sync + 'static,
    PageSelector: Serialize + Send + Sync + 'static,
{
    /**
     * Indicates that the client is resuming a previous scan.  The client
     * provides `page_token`, an opaque value that wraps the consumer's
     * `PageSelector` type.  Critically, this must provide enough information
     * for the consumer to resume a scan.
     */
    Next(PageSelector),

    /**
     * Indicates that the client is beginning a new scan.  The client may
     * provide `list_mode`, a consumer-specified type.
     */
    First(ScanMode),
}

impl<ScanMode, PageSelector> dropshot::ExtractedParameter
    for WhichPage<ScanMode, PageSelector>
where
    ScanMode: Debug + Send + Sync + 'static,
    PageSelector: Serialize + Send + Sync + 'static,
{
    fn metadata(
        inn: dropshot::ApiEndpointParameterLocation,
    ) -> Vec<dropshot::ApiEndpointParameter> {
        // XXX
        todo!()
    }
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
 * Query string deserialization
 */

#[derive(Deserialize, ExtractedParameter)]
#[serde(bound(deserialize = "ScanMode: DeserializeOwned"))]
#[serde(untagged)]
enum RawWhichPage<ScanMode>
where
    ScanMode: Debug + Send + Sync + 'static,
{
    Next { page_token: String },
    First(ScanMode),
}

#[derive(Deserialize, ExtractedParameter)]
#[serde(bound(deserialize = "ScanMode: DeserializeOwned"))]
struct RawPaginationParams<ScanMode>
where
    ScanMode: Debug + Send + Sync + 'static,
{
    #[serde(flatten)]
    page_params: RawWhichPage<ScanMode>,
    limit: Option<NonZeroU64>,
}

impl<ScanMode, PageSelector> TryFrom<RawPaginationParams<ScanMode>>
    for PaginationParams<ScanMode, PageSelector>
where
    ScanMode: Debug + DeserializeOwned + Send + Sync + 'static,
    PageSelector: DeserializeOwned + Serialize + Send + Sync + 'static,
{
    type Error = String;

    fn try_from(
        raw: RawPaginationParams<ScanMode>,
    ) -> Result<PaginationParams<ScanMode, PageSelector>, String> {
        let token_str = match raw.page_params {
            RawWhichPage::First(scan_mode) => {
                return Ok(PaginationParams {
                    page_params: WhichPage::First(scan_mode),
                    limit: raw.limit,
                })
            }
            RawWhichPage::Next {
                page_token,
            } => page_token,
        };

        if token_str.len() > MAX_TOKEN_LENGTH {
            return Err(String::from(
                "failed to parse pagination token: too large",
            ));
        }

        let json_bytes = base64::decode_config(token_str.as_bytes(), URL_SAFE)
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

        Ok(PaginationParams {
            page_params: WhichPage::Next(deserialized.page_start),
            limit: raw.limit,
        })
    }
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
