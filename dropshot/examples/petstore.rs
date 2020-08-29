use dropshot::{
    endpoint, ApiDescription, HttpError, HttpResponseOk, Path, RequestContext,
    TypedBody,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

fn main() -> Result<(), String> {
    /*
     * Build a description of the API.
     */
    let mut api = ApiDescription::new();
    api.register(get_pet_by_id).unwrap();
    api.register(update_pet_with_form).unwrap();

    api.print_openapi(
        &mut std::io::stdout(),
        &"Pet Shop",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &"",
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[allow(dead_code)]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Pet {
    id: Option<i64>,
    category: Option<Category>,
    name: String,
    photo_urls: Vec<String>,
    tags: Option<Vec<Tag>>,

    /// pet status in the store
    status: Option<PetStatus>,
}

#[allow(dead_code)]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Category {
    id: i64,
    name: String,
}

#[allow(dead_code)]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Tag {}

#[allow(dead_code)]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum PetStatus {
    Available,
    Pending,
    Sold,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct PathParams {
    pet_id: i64,
}

#[allow(unused_variables)]
#[endpoint {
    method = GET,
    path = "/pet/{petId}",
    tags = [ "pet" ],
}]
async fn get_pet_by_id(
    rqctx: Arc<RequestContext>,
    path_params: Path<PathParams>,
) -> Result<HttpResponseOk<Pet>, HttpError> {
    let pet = Pet {
        id: None,
        category: None,
        name: "Brickley".to_string(),
        photo_urls: vec![],
        tags: None,
        status: None,
    };

    Ok(HttpResponseOk(pet))
}

#[endpoint {
    method = POST,
    path = "/pet",
    tags = [ "pet" ],
}]
/// Add a new pet to the store
async fn update_pet_with_form(
    _rqctx: Arc<RequestContext>,
    _body: TypedBody<Pet>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}
