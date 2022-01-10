// Copyright 2020 Oxide Computer Company
/*!
 * Example showing a relatively simple use of the pagination API
 *
 * When you run this program, it will start an HTTP server on an available local
 * port.  See the log entry to see what port it ran on.  Then use curl to use
 * it, like this:
 *
 * ```ignore
 * $ curl localhost:50568/projects
 * ```
 *
 * (Replace 50568 with whatever port your server is listening on.)
 *
 * Try passing different values of the `limit` query parameter.  Try passing the
 * next page token from the response as a query parameter, too.
 */

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::EmptyScanParams;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::HttpServerStarter;
use dropshot::PaginationParams;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::ResultsPage;
use dropshot::WhichPage;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::ops::Bound;
use std::sync::Arc;

/**
 * Object returned by our paginated endpoint
 *
 * Like anything returned by Dropshot, we must implement `JsonSchema` and
 * `Serialize`.  We also implement `Clone` to simplify the example.
 */
#[derive(Clone, JsonSchema, Serialize)]
struct Project {
    name: String,
    // lots more fields
}

/**
 * Parameters describing the client's position in a scan through all projects
 *
 * This implementation only needs the name of the last project seen, as we only
 * support listing projects in ascending order by name.
 *
 * This must be `Serialize` so that Dropshot can turn it into a page token to
 * include with each page of results, and it must be `Deserialize` to get it
 * back in a querystring.
 */
#[derive(Deserialize, JsonSchema, Serialize)]
struct ProjectPage {
    name: String,
}

/**
 * API endpoint for listing projects
 *
 * This implementation stores all the projects in a BTreeMap, which makes it
 * very easy to fetch a particular range of items based on the key.
 */
#[endpoint {
    method = GET,
    path = "/projects"
}]
async fn example_list_projects(
    rqctx: Arc<RequestContext<BTreeMap<String, Project>>>,
    query: Query<PaginationParams<EmptyScanParams, ProjectPage>>,
) -> Result<HttpResponseOk<ResultsPage<Project>>, HttpError> {
    let pag_params = query.into_inner();
    let limit = rqctx.page_limit(&pag_params)?.get() as usize;
    let tree = rqctx.context();
    let projects = match &pag_params.page {
        WhichPage::First(..) => {
            /* Return a list of the first "limit" projects. */
            tree.iter()
                .take(limit)
                .map(|(_, project)| project.clone())
                .collect()
        }
        WhichPage::Next(ProjectPage {
            name: last_seen,
        }) => {
            /* Return a list of the first "limit" projects after this name. */
            tree.range((Bound::Excluded(last_seen.clone()), Bound::Unbounded))
                .take(limit)
                .map(|(_, project)| project.clone())
                .collect()
        }
    };

    Ok(HttpResponseOk(ResultsPage::new(
        projects,
        &EmptyScanParams {},
        |p: &Project, _| ProjectPage {
            name: p.name.clone(),
        },
    )?))
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let port = std::env::args()
        .nth(1)
        .map(|p| p.parse::<u16>())
        .transpose()
        .map_err(|e| format!("failed to parse \"port\" argument: {}", e))?
        .unwrap_or(0);

    /*
     * Create 1000 projects up front.
     */
    let mut tree = BTreeMap::new();
    for n in 1..1000 {
        let name = format!("project{:03}", n);
        let project = Project {
            name: name.clone(),
        };
        tree.insert(name, project);
    }

    /*
     * Run the Dropshot server.
     */
    let ctx = tree;
    let config_dropshot = ConfigDropshot {
        bind_address: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        ..Default::default()
    };
    let config_logging = ConfigLogging::StderrTerminal {
        level: ConfigLoggingLevel::Debug,
    };
    let log = config_logging
        .to_logger("example-pagination-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;
    let mut api = ApiDescription::new();
    api.register(example_list_projects).unwrap();
    let server = HttpServerStarter::new(&config_dropshot, api, ctx, &log)
        .map_err(|error| format!("failed to create server: {}", error))?
        .start();
    server.await
}
