// Copyright 2020 Oxide Computer Company
/*!
 * Parameters related to paginated collection endpoints.
 *
 * XXX flesh out goals (e.g., multiple keys, etc.)
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
use std::num::NonZeroU64;

/**
 * Maximum length of a pagination token once the consumer-provided type is
 * serialized and the result is base64-encoded.
 *
 * We impose a maximum length primarily to avoid a client forcing us to parse
 * extremely large strings.  We apply this limit when we create tokens as well
 * to attempt to catch the error earlier.
 *
 * Note that these tokens are passed in the HTTP request line (before the
 * headers), and many HTTP implementations impose an implicit limit as low as
 * 8KiB on the size of the request line and headers together, so it's a good
 * idea to keep this as small as we can.
 */
const MAX_TOKEN_LENGTH: usize = 512;

#[derive(Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PaginationOrder {
    Ascending,
    Descending,
}

#[derive(Debug, Deserialize, ExtractedParameter, JsonSchema, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DropshotMarkerVersion {
    V1,
}

#[derive(Debug, Deserialize, ExtractedParameter)]
pub struct PaginationParams<MarkerFields: DeserializeOwned> {
    /**
     * If present, this is the value of the sort field for the last object seen
     */
    pub marker: Option<PaginationMarker<MarkerFields>>,

    /**
     * If present, this is the order of results to return.
     * XXX consider implementing a default() that makes this ascending?
     */
    pub order: Option<PaginationOrder>,

    /**
     * If present, this is an upper bound on how many objects the client wants
     * in this page of results.  The server may choose to use a lower limit.
     * XXX consider implementing a default() that gives this a value?  It'd be
     * nice if this were runtime-configurable.
     */
    pub limit: Option<NonZeroU64>,
}

#[derive(Debug, Deserialize, ExtractedParameter, JsonSchema, Serialize)]
#[serde(try_from = "String")]
pub struct PaginationMarker<MarkerFields>
{
    version: DropshotMarkerVersion,
    order: PaginationOrder,
    #[serde(bound(deserialize = "MarkerFields: DeserializeOwned"))]
    pub page_start: MarkerFields,
}

impl<MarkerFields> PaginationMarker<MarkerFields>
where
    MarkerFields: Serialize,
{
    pub fn new(order: PaginationOrder, page_start: MarkerFields) -> Self {
        PaginationMarker {
            version: DropshotMarkerVersion::V1,
            order,
            page_start,
        }
    }

    pub fn to_serialized(&self) -> Result<String, HttpError> {
        let marker_bytes = {
            let json_bytes = serde_json::to_vec(self).map_err(|e| {
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

impl<MarkerFields> TryFrom<String> for PaginationMarker<MarkerFields>
where
    MarkerFields: DeserializeOwned,
{
    type Error = String;

    fn try_from(
        value: String,
    ) -> Result<PaginationMarker<MarkerFields>, String> {
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
        let rv: PaginationMarker<MarkerFields> =
            serde_json::from_slice(&json_bytes).map_err(|e| {
                format!("failed to parse pagination token: corrupted token")
            })?;
        Ok(rv)
    }
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
pub struct ClientPage<ItemType> {
    pub next_page: Option<String>,
    pub has_next: bool,
    pub items: Vec<ItemType>,
}
