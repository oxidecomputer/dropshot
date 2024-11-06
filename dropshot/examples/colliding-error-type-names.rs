// Copyright 2024 Oxide Computer Companyy

//! Example of an API using a user-defined error type to serialize error
//! representations differently from [`dropshot::HttpError`].

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::RequestContext;
use dropshot::ServerBuilder;

mod latex {
    use super::*;

    /// Errors returned by pdflatex.
    ///
    /// Good luck figuring out what these mean!
    #[derive(
        Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
    )]
    pub enum Error {
        /// An hbox is overfull.
        OverfullHbox {
            /// The amount of badness in the overfull hbox.
            badness: usize,
        },
        /// Like an overfull hbox, except the opposite of that.
        UnderfullHbox {
            /// This one also has badness.
            badness: usize,
        },
    }

    impl dropshot::error::IntoErrorResponse for Error {
        fn into_error_response(
            &self,
            ctx: dropshot::error::ErrorContext<'_>,
        ) -> http::Response<dropshot::Body> {
            http::Response::builder()
                .status(http::StatusCode::BAD_REQUEST)
                .header()
                .body(
                    serde_json::to_string(self)
                        .expect("serialization of MyError should never fail")
                        .into(),
                )
                .expect("building response should never fail")
        }
    }

    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::OverfullHbox { badness } => {
                    write!(f, "overfull hbox, badness {}", badness)
                }
                Self::UnderfullHbox { badness } => {
                    write!(f, "underfull hbox, badness {badness}")
                }
            }
        }
    }

    #[endpoint {
    method = GET,
    path = "/pdflatex/{filename}",
}]
    pub(super) async fn get_pdflatex(
        _rqctx: RequestContext<()>,
        _path: dropshot::Path<PdflatexPathParams>,
    ) -> Result<dropshot::HttpResponseCreated<Pdf>, Error> {
        Err(Error::OverfullHbox { badness: 1000 })
    }

    #[derive(
        Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema,
    )]
    struct PdflatexPathParams {
        filename: String,
    }

    #[derive(
        Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema,
    )]
    struct Pdf {
        filename: String,
    }
}

mod flask {
    use super::*;
    #[derive(
        Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema,
    )]
    struct Flask {
        name: String,
    }
    #[derive(
        Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema,
    )]
    struct Error {
        reason: String,
    }

    impl dropshot::error::IntoErrorResponse for Error {
        fn into_error_response(
            &self,
            _request_id: &str,
        ) -> http::Response<dropshot::Body> {
            http::Response::builder()
                .status(http::StatusCode::NOT_FOUND)
                .body(
                    serde_json::to_string(self)
                        .expect("serialization of Error should never fail")
                        .into(),
                )
                .expect("building response should never fail")
        }
    }

    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.reason.as_str())
        }
    }

    /// Gets ye flask.
    #[endpoint {
        method = GET,
        path = "/ye-flask",
    }]
    pub(crate) async fn get_ye_flask(
        _rqctx: RequestContext<()>,
    ) -> Result<dropshot::HttpResponseOk<Flask>, Error> {
        Err(Error { reason: "can't get ye flask!".to_string() })
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    // See dropshot/examples/basic.rs for more details on most of these pieces.
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-custom-error-type")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    let mut api = ApiDescription::new();
    api.register(flask::get_ye_flask).unwrap();
    api.register(latex::get_pdflatex).unwrap();

    // Print the OpenAPI spec to stdout as an example.
    println!("OpenAPI spec:");
    api.openapi("Custom Error Example", "1.0")
        .write(&mut std::io::stdout())
        .unwrap();

    let server = ServerBuilder::new(api, (), log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    server.await
}
