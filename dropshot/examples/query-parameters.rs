// Copyright 2026 Oxide Computer Company
//! Query parameter behavior for scalar and list values.
//!
//! Dropshot uses `serde_urlencoded` to deserialize query strings.  A derived
//! query struct therefore rejects repeated scalar fields, and changing the
//! field to `Vec<T>` does not make repeated fields work automatically.  This
//! example also shows two ways to accept lists explicitly: repeated keys with
//! a custom `Deserialize` implementation, and comma-separated values with a
//! field deserializer.  Both workarounds expose the query parameter as a string
//! in the generated OpenAPI document, so its list syntax must be documented by
//! the API.

use dropshot::ApiDescription;
#[cfg(test)]
use dropshot::ApiDescriptionRegisterError;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use dropshot::endpoint;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::de;
use serde::de::MapAccess;
use serde::de::Visitor;
use std::fmt;
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<(), String> {
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Info };
    let log = config_logging
        .to_logger("example-query-parameters")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    let server = ServerBuilder::new(api(), 0_usize, log)
        .start()
        .map_err(|error| format!("failed to create server: {}", error))?;

    server.await
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum State {
    Active,
    Closed,
}

impl FromStr for State {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "active" => Ok(Self::Active),
            "closed" => Ok(Self::Closed),
            _ => Err(format!("unknown state {value:?}")),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct ScalarStateQuery {
    state: State,
}

/// A conventionally derived `Vec` query field.
///
/// Dropshot rejects an endpoint using this query type during API registration
/// because query parameters must have scalar schemas.  Independently,
/// `serde_urlencoded` cannot deserialize either `?state=active&state=closed` or
/// `?state=active,closed` into this type.
#[cfg(test)]
#[derive(Deserialize, JsonSchema)]
struct DerivedVectorQuery {
    state: Vec<State>,
}

/// Repeated query parameters collected by a custom deserializer.
#[derive(JsonSchema)]
struct RepeatedStateQuery {
    // Dropshot requires query parameter schemas to be scalar.  The custom
    // deserializer below still collects each occurrence into this vector.
    #[schemars(with = "String")]
    state: Vec<State>,
}

impl<'de> Deserialize<'de> for RepeatedStateQuery {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RepeatedStateVisitor;

        impl<'de> Visitor<'de> for RepeatedStateVisitor {
            type Value = RepeatedStateQuery;

            fn expecting(
                &self,
                formatter: &mut fmt::Formatter<'_>,
            ) -> fmt::Result {
                formatter.write_str("one or more state query parameters")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut state = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "state" => state.push(map.next_value()?),
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &["state"],
                            ));
                        }
                    }
                }

                if state.is_empty() {
                    return Err(de::Error::missing_field("state"));
                }

                Ok(RepeatedStateQuery { state })
            }
        }

        deserializer.deserialize_map(RepeatedStateVisitor)
    }
}

#[derive(Deserialize, JsonSchema)]
struct CommaSeparatedStateQuery {
    #[serde(deserialize_with = "deserialize_comma_separated_states")]
    // The wire value is one string, which the deserializer splits into a Vec.
    #[schemars(with = "String")]
    state: Vec<State>,
}

fn deserialize_comma_separated_states<'de, D>(
    deserializer: D,
) -> Result<Vec<State>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    value
        .split(',')
        .map(|state| state.parse().map_err(de::Error::custom))
        .collect()
}

#[endpoint {
    method = GET,
    path = "/state/scalar",
}]
async fn get_scalar_state(
    _rqctx: RequestContext<usize>,
    query: Query<ScalarStateQuery>,
) -> Result<HttpResponseOk<State>, HttpError> {
    Ok(HttpResponseOk(query.into_inner().state))
}

#[cfg(test)]
#[endpoint {
    method = GET,
    path = "/states/derived-vector",
}]
async fn get_derived_vector(
    _rqctx: RequestContext<usize>,
    query: Query<DerivedVectorQuery>,
) -> Result<HttpResponseOk<Vec<State>>, HttpError> {
    Ok(HttpResponseOk(query.into_inner().state))
}

#[endpoint {
    method = GET,
    path = "/states/repeated",
}]
async fn get_repeated_states(
    _rqctx: RequestContext<usize>,
    query: Query<RepeatedStateQuery>,
) -> Result<HttpResponseOk<Vec<State>>, HttpError> {
    Ok(HttpResponseOk(query.into_inner().state))
}

#[endpoint {
    method = GET,
    path = "/states/comma-separated",
}]
async fn get_comma_separated_states(
    _rqctx: RequestContext<usize>,
    query: Query<CommaSeparatedStateQuery>,
) -> Result<HttpResponseOk<Vec<State>>, HttpError> {
    Ok(HttpResponseOk(query.into_inner().state))
}

pub(crate) fn api() -> ApiDescription<usize> {
    let mut api = ApiDescription::new();
    api.register(get_scalar_state).unwrap();
    api.register(get_repeated_states).unwrap();
    api.register(get_comma_separated_states).unwrap();
    api
}

#[cfg(test)]
pub(crate) fn register_vec() -> Result<(), ApiDescriptionRegisterError> {
    let mut api = ApiDescription::<usize>::new();
    api.register(get_derived_vector)
}

#[cfg(test)]
pub(crate) fn deserialize_derived_vector(
    query: &str,
) -> Result<Vec<State>, serde_urlencoded::de::Error> {
    serde_urlencoded::from_str::<DerivedVectorQuery>(query)
        .map(|query| query.state)
}
