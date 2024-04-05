// Copyright 2022 Oxide Computer Company

//! Example of an API that applies a rigorous tag policy in which each endpoint
//! must use exactly one of the predetermined tags. Tags are often used by
//! documentation generators; Dropshot's tag policies are intended to make
//! proper tagging innate.

use dropshot::{
    endpoint, ApiDescription, ConfigLogging, ConfigLoggingLevel,
    EndpointTagPolicy, HttpError, HttpResponseOk, HttpServerStarter,
    RequestContext, TagConfig, TagDetails, TagExternalDocs,
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
    // We must specify a configuration with a bind address.  We'll use 127.0.0.1
    // since it's available and won't expose this server outside the host.  We
    // request port 0, which allows the operating system to pick any available
    // port.
    let config_dropshot = Default::default();

    // For simplicity, we'll configure an "info"-level logger that writes to
    // stderr assuming that it's a terminal.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    // Build a description of the API -- in this case it's not much of an API!.
    let mut api = ApiDescription::new().tag_config(TagConfig {
        allow_other_tags: false,
        endpoint_tag_policy: EndpointTagPolicy::ExactlyOne,
        tag_definitions: vec![
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

    // Set up the server.
    let server = HttpServerStarter::new(config_dropshot, api, (), &log)
        .map_err(|error| format!("failed to create server: {}", error))?
        .start();

    // Wait for the server to stop.  Note that there's not any code to shut down
    // this server, so we should never get past this point.
    server.await
}
