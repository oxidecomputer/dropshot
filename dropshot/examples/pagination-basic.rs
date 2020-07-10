/*!
 * Basic example that shows a paginated API
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
use dropshot::ExtractedParameter;
use dropshot::HttpError;
use dropshot::HttpResponseOkPage;
use dropshot::HttpServer;
use dropshot::PaginationParams;
use dropshot::Query;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::ops::RangeFrom;
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
 * Holds the fields that a client can sort by and the value(s) describing the
 * client's current position in the scan
 *
 * This example shows how to support listing projects by name.
 *
 * This struct must implement `Deserialize` and `ExtractedParameter` because we
 * receive it as a query parameter (see [`dropshot::Query`]).  The struct must
 * also impl `Serialize` because we serialize it as part of the token we give to
 * clients for the next page.
 */
#[derive(Deserialize, ExtractedParameter, JsonSchema, Serialize)]
struct ProjectsByName {
    /** the name of the last project the client has seen so far */
    name: String,
}

/**
 * Defines a conversion from Projects to ProjectsByName so that Dropshot can
 * generate the pagination token for the client directly from our list of
 * results.
 */
impl From<&Project> for ProjectsByName {
    fn from(last_item: &Project) -> ProjectsByName {
        ProjectsByName {
            name: last_item.name.clone(),
        }
    }
}

/** Default number of returned results */
const DEFAULT_LIMIT: usize = 10;
/** Maximum number of returned results */
const MAX_LIMIT: usize = 100;

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
    rqctx: Arc<RequestContext>,
    query: Query<PaginationParams<ProjectsByName>>,
) -> Result<HttpResponseOkPage<ProjectsByName, Project>, HttpError> {
    let pag_params = query.into_inner();
    // XXX even a convenience method here would help
    let mut limit =
        pag_params.limit.map(|l| l.get() as usize).unwrap_or(DEFAULT_LIMIT);
    if limit > MAX_LIMIT {
        limit = MAX_LIMIT;
    }

    let tree = rqctx_to_tree(rqctx);
    let projects = match &pag_params.marker {
        None => {
            /* Return a list of the first "limit" projects. */
            tree.iter()
                .take(limit)
                .map(|(_, project)| project.clone())
                .collect()
        }
        Some(marker) => {
            /* Return a list of the first "limit" projects after this name. */
            let last_project_name_seen = &marker.page_start.name;
            tree.range(RangeFrom {
                start: last_project_name_seen.clone(),
            })
            .take(limit)
            .map(|(_, project)| project.clone())
            .collect()
        }
    };

    Ok(HttpResponseOkPage(pag_params, projects))
}

fn rqctx_to_tree(rqctx: Arc<RequestContext>) -> Arc<BTreeMap<String, Project>> {
    let c = Arc::clone(&rqctx.server.private);
    c.downcast::<BTreeMap<String, Project>>().unwrap()
}

#[tokio::main]
async fn main() -> Result<(), String> {
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
    let ctx = Arc::new(tree);
    let config_dropshot = ConfigDropshot {
        bind_address: "127.0.0.1:0".parse().unwrap(),
    };
    let config_logging = ConfigLogging::StderrTerminal {
        level: ConfigLoggingLevel::Debug,
    };
    let log = config_logging
        .to_logger("example-pagination-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;
    let mut api = ApiDescription::new();
    api.register(example_list_projects).unwrap();
    let mut server = HttpServer::new(&config_dropshot, api, ctx, &log)
        .map_err(|error| format!("failed to create server: {}", error))?;
    let server_task = server.run();
    server.wait_for_shutdown(server_task).await
}
