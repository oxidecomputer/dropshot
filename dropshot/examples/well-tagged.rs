// Copyright 2022 Oxide Computer Company

//! Example of an API that applies a rigorous tag policy in which each endpoint
//! must use exactly one of the predetermined tags. Tags are often used by
//! documentation generators; Dropshot's tag policies are intended to make
//! proper tagging innate.

use dropshot::{
    ApiDescription, ConfigLogging, ConfigLoggingLevel, EndpointTagPolicy,
    HttpError, HttpResponseOk, RequestContext, ServerBuilder, TagConfig,
    TagDetails, TagExternalDocs, endpoint,
};

#[endpoint {
    method = GET,
    path = "/homerism",
    tags = ["simpsons"],
}]
async fn get_homerism(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<String>, HttpError> {
    unimplemented!()
}

#[endpoint {
    method = GET,
    path = "/barneyism",
    tags = ["simpsons"],
}]
async fn get_barneyism(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<String>, HttpError> {
    unimplemented!()
}

#[endpoint {
    method = GET,
    path = "/get_fryism",
    tags = ["futurama"],
}]
async fn get_fryism(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<String>, HttpError> {
    unimplemented!()
}

#[tokio::main]
async fn main() -> Result<(), String> {
    // See dropshot/examples/basic.rs for more details on most of these pieces.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    // Build a description of the API -- in this case it's not much of an API!
    let mut api = ApiDescription::new().tag_config(TagConfig {
        allow_other_tags: false,
        policy: EndpointTagPolicy::ExactlyOne,
        tags: vec![
            (
                "simpsons".to_string(),
                TagDetails {
                    description: Some(
                        "Important information related to The Simpsons"
                            .to_string(),
                    ),
                    external_docs: Some(TagExternalDocs {
                        description: None,
                        url: "https://frinkiac.com/".to_string(),
                    }),
                },
            ),
            (
                "futurama".to_string(),
                TagDetails {
                    description: Some(
                        "Important information related to Futurama".to_string(),
                    ),
                    external_docs: Some(TagExternalDocs {
                        description: None,
                        url: "https://morbotron.com/".to_string(),
                    }),
                },
            ),
        ]
        .into_iter()
        .collect(),
    });
    api.register(get_homerism).unwrap();
    api.register(get_barneyism).unwrap();
    api.register(get_fryism).unwrap();

    let server = ServerBuilder::new(api, (), log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    server.await
}
