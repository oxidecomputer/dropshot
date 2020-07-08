// Copyright 2020 Oxide Computer Company
/*!
 * Parameters related to paginated collection endpoints.
 *
 * XXX flesh out goals (e.g., multiple keys, etc.)
 */

use crate as dropshot; // XXX needed for ExtractedParameter below
use dropshot_endpoint::ExtractedParameter;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

#[derive(Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PaginationOrder {
    Ascending,
    Descending,
}

#[derive(Deserialize, ExtractedParameter, JsonSchema, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DropshotMarkerVersion {
    V1,
}

#[derive(Deserialize, ExtractedParameter)]
pub struct PaginationParams<MarkerFields> {
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
    pub limit: Option<u16>,
}

#[derive(Deserialize, ExtractedParameter, JsonSchema, Serialize)]
pub struct PaginationMarker<MarkerFields> {
    dropshot_marker_version: DropshotMarkerVersion,
    order: PaginationOrder,
    pub page_start: MarkerFields,
}

impl<MarkerFields> PaginationMarker<MarkerFields> {
    pub fn new(order: PaginationOrder, page_start: MarkerFields) -> Self {
        PaginationMarker {
            dropshot_marker_version: DropshotMarkerVersion::V1,
            order,
            page_start,
        }
    }
}

#[derive(Deserialize, JsonSchema, Serialize)]
pub struct Page<MarkerFields, ItemType> {
    pub next_page: Option<PaginationMarker<MarkerFields>>,
    pub has_next: bool,
    pub items: Vec<ItemType>,
}
