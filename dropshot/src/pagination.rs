// Copyright 2020 Oxide Computer Company
/*!
 * Parameters related to paginated collection endpoints.
 *
 * XXX flesh out goals (e.g., multiple keys, etc.)
 */

use crate as dropshot; // XXX needed for ExtractedParameter below
use crate::error::HttpError;
use base64::write::EncoderWriter;
use base64::URL_SAFE;
use dropshot_endpoint::ExtractedParameter;
use flate2::Compression;
use flate2::GzBuilder;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::num::NonZeroU64;

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
    pub limit: Option<NonZeroU64>,
}

#[derive(Debug, Deserialize, ExtractedParameter, JsonSchema, Serialize)]
pub struct PaginationMarker<MarkerFields> {
    dropshot_marker_version: DropshotMarkerVersion,
    order: PaginationOrder,
    pub page_start: MarkerFields,
}

impl<MarkerFields> PaginationMarker<MarkerFields>
where
    MarkerFields: Serialize,
{
    pub fn new(order: PaginationOrder, page_start: MarkerFields) -> Self {
        PaginationMarker {
            dropshot_marker_version: DropshotMarkerVersion::V1,
            order,
            page_start,
        }
    }

    pub fn to_serialized(&self) -> Result<String, HttpError> {
        let marker_bytes = {
            let mut v = Vec::new();
            let base64_encoder = EncoderWriter::new(&mut v, URL_SAFE);
            let gz_encoder =
                GzBuilder::new().write(base64_encoder, Compression::fast());
            serde_json::to_writer(gz_encoder, self).map_err(|e| {
                HttpError::for_internal_error(format!(
                    "failed to serialize marker: {}",
                    e
                ))
            })?;
            v
        };

        Ok(String::from_utf8(marker_bytes).unwrap())
    }

    //pub fn serialize<S>(
    //    maybe_marker: &Option<PaginationMarker<MarkerFields>>,
    //    serializer: S,
    //) -> Result<S::Ok, S::Error>
    //where
    //    S: Serializer,
    //{
    //    let marker = match maybe_marker {
    //        None => return serializer.serialize_none(),
    //        Some(m) => m,
    //    };

    //    let marker_bytes = {
    //        let mut v = Vec::new();
    //        let base64_encoder = EncoderWriter::new(&mut v, URL_SAFE);
    //        let gz_encoder =
    //            GzBuilder::new().write(base64_encoder, Compression::fast());
    //        serde_json::to_writer(gz_encoder, marker).map_err(|e| {
    //            S::Error::custom(format!("failed to serialize marker: {}", e))
    //        })?;
    //        v
    //    };

    //    let marker_serialized: Option<&str> =
    //        Some(&std::str::from_utf8(&marker_bytes).unwrap());
    //    serializer.serialize_some(&marker_serialized)
    //}

    // XXX
    // pub fn deserialize<'de, D>(
    //     deserializer: D,
    // ) -> Result<PaginationMarker<MarkerFields>, D::Error>
    // where
    //     D: Deserializer<'de>,
    // {
    // }
}

// #[derive(Debug, JsonSchema, Serialize)]
// pub(crate) struct Page<MarkerFields, ItemType>
// where
//     MarkerFields: Serialize,
// {
//     #[serde(serialize_with = "PaginationMarker::serialize")]
//     pub next_page: Option<PaginationMarker<MarkerFields>>,
//     pub has_next: bool,
//     pub items: Vec<ItemType>,
// }

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
pub struct ClientPage<ItemType> {
    pub next_page: Option<String>,
    pub has_next: bool,
    pub items: Vec<ItemType>,
}
