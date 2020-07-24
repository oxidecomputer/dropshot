// Copyright 2020 Oxide Computer Company
/*!
 * Example of an API endpoint that supports pagination using several different
 * fields as the sorting key.
 *
 * When you run this program, it will start an HTTP server on an available local
 * port.  See the log for example URLs to use.  Try passing different values of
 * the `limit` query parameter.  Try passing the `next_page` token from the
 * response as a query parameter called `page_token`, too.
 *
 * For background, see src/pagination.rs.  This example uses a resource called a
 * "Project", which only has a "name" and an "mtime" (modification time).  The
 * server creates 1,000 projects on startup and provides one API endpoint to
 * page through them.
 *
 * Initially, a client just invokes the API to list the first page of results
 * using the default sort order (we'll use limit=3 to keep the result set
 * short):
 *
 * ```ignore
 * $ curl -s http://127.0.0.1:50800/projects?limit=3 | json
 * {
 *   "next_page": "eyJ2IjoidjEiLCJwYWdlX3N0YXJ0Ijp7Im5hbWUiOlsiYXNjZW5kaW5nIiwicHJvamVjdDAwMyJdfX0=",
 *   "items": [
 *     {
 *       "name": "project001",
 *       "mtime": "2020-07-13T17:35:00Z"
 *     },
 *     {
 *       "name": "project002",
 *       "mtime": "2020-07-13T17:34:59.999Z"
 *     },
 *     {
 *       "name": "project003",
 *       "mtime": "2020-07-13T17:34:59.998Z"
 *     }
 *   ]
 * }
 * ```
 *
 * This should be pretty self-explanatory: we have three projects here and
 * they're sorted in ascending order by name.  The "next_page" token is used to
 * fetch the next page of results as follows:
 *
 * ```ignore
 * $ curl -s http://127.0.0.1:50800/projects?limit=3'&'page_token=eyJ2IjoidjEiLCJwYWdlX3N0YXJ0Ijp7Im5hbWUiOlsiYXNjZW5kaW5nIiwicHJvamVjdDAwMyJdfX0= | json
 * {
 *   "next_page": "eyJ2IjoidjEiLCJwYWdlX3N0YXJ0Ijp7Im5hbWUiOlsiYXNjZW5kaW5nIiwicHJvamVjdDAwNiJdfX0=",
 *   "items": [
 *     {
 *       "name": "project004",
 *       "mtime": "2020-07-13T17:34:59.997Z"
 *     },
 *     {
 *       "name": "project005",
 *       "mtime": "2020-07-13T17:34:59.996Z"
 *     },
 *     {
 *       "name": "project006",
 *       "mtime": "2020-07-13T17:34:59.995Z"
 *     }
 *   ]
 * }
 * ```
 *
 * Now we have the next three projects and a new token.  We can continue this
 * way until we've listed all the projects.
 *
 * What does that page token look like?  It's implementation-defined, so you
 * shouldn't rely on the structure.  In this case, it's a base64-encoded,
 * versioned JSON structure describing the scan and the client's position in the
 * scan:
 *
 * ```ignore
 * $ echo -n 'eyJ2IjoidjEiLCJwYWdlX3N0YXJ0Ijp7Im5hbWUiOlsiYXNjZW5kaW5nIiwicHJvamVjdDAwNiJdfX0=' | base64 -d | json
 * {
 *   "v": "v1",
 *   "page_start": {
 *     "name": [
 *       "ascending",
 *       "project006"
 *     ]
 *   }
 * }
 * ```
 *
 * This token says that we're scanning in ascending order of "name" and the last
 * one we saw was "project006".  Again, this is subject to change and should not
 * be relied upon.  We mention it here just to help explain how the pagination
 * mechanism works.
 */

use chrono::offset::TimeZone;
use chrono::DateTime;
use chrono::Utc;
use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::ExtractedParameter;
use dropshot::HttpError;
use dropshot::HttpResponseOkPage;
use dropshot::HttpServer;
use dropshot::PaginationOrder;
use dropshot::PaginationOrder::Ascending;
use dropshot::PaginationOrder::Descending;
use dropshot::PaginationParams;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::WhichPage;
use hyper::Uri;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::ops::Bound;
use std::sync::Arc;

#[macro_use]
extern crate slog;

/**
 * Item returned by our paginated endpoint
 *
 * Like anything returned by Dropshot, we must implement `JsonSchema` and
 * `Serialize`.  We also implement `Clone` to simplify the example.
 */
#[derive(Clone, JsonSchema, Serialize)]
struct Project {
    name: String,
    mtime: DateTime<Utc>,
    // lots more fields
}

/**
 * Specifies how the client wants to page through results (typically: what
 * field(s) to sort by and whether the sort should be ascending or descending)
 *
 * It's up to the consumer (e.g., this example) to decide exactly which modes
 * are supported here and what each one means.  This type represents an
 * interface that's part of the OpenAPI specification for the service.
 *
 * NOTE: To be useful, this field must be deserializable using the
 * `serde_querystring` module.  You can test this by writing test code to
 * serialize it using `serde_querystring`.  That code could fail at runtime for
 * certain types of values (e.g., enum variants that contain data).
 */
#[derive(
    Deserialize, Clone, Debug, ExtractedParameter, JsonSchema, Serialize,
)]
struct ProjectScanParams {
    sort_mode: Option<ProjectScanMode>,
}

#[derive(
    Deserialize, Clone, Debug, ExtractedParameter, JsonSchema, Serialize,
)]
#[serde(rename_all = "kebab-case")]
enum ProjectScanMode {
    /** by name ascending */
    ByNameAscending,
    /** by name descending */
    ByNameDescending,
    /** by mtime ascending, then by name ascending */
    ByMtimeAscending,
    /** by mtime descending, then by name descending */
    ByMtimeDescending,
}

/**
 * Specifies the scan mode and the client's current position in the scan
 *
 * Dropshot uses this information to construct a page token that's sent to the
 * client with each page of results.  The client provides that page token in a
 * subsequent request for the next page of results.  Your endpoint is expected
 * to use this information to resume the scan where the previous request left
 * off.
 *
 * The most common robust and scalable implementation is to have this structure
 * include the scan mode (see above) and the last value seen the key field(s)
 * (i.e., the fields that the results are sorted by).  When you get this
 * selector back, you find the object having the next value after the one stored
 * in the token and start returning results from there.
 */
#[derive(Debug, Deserialize, ExtractedParameter, JsonSchema, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ProjectScanPageSelector {
    Name(PaginationOrder, String),
    MtimeName(PaginationOrder, DateTime<Utc>, String),
}

/**
 * Given a project (typically representing the last project in a page of
 * results) and scan mode, return a page selector that can be sent to the client
 * to request the next page of results.
 */
fn page_selector_for(
    last_item: &Project,
    scan_mode: &ProjectScanMode,
) -> ProjectScanPageSelector {
    match scan_mode {
        ProjectScanMode::ByNameAscending => {
            ProjectScanPageSelector::Name(Ascending, last_item.name.clone())
        }
        ProjectScanMode::ByNameDescending => {
            ProjectScanPageSelector::Name(Descending, last_item.name.clone())
        }
        ProjectScanMode::ByMtimeAscending => {
            ProjectScanPageSelector::MtimeName(
                Ascending,
                last_item.mtime,
                last_item.name.clone(),
            )
        }
        ProjectScanMode::ByMtimeDescending => {
            ProjectScanPageSelector::MtimeName(
                Descending,
                last_item.mtime,
                last_item.name.clone(),
            )
        }
    }
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
    rqctx: Arc<RequestContext>,
    query: Query<PaginationParams<ProjectScanParams, ProjectScanPageSelector>>,
) -> Result<HttpResponseOkPage<Project>, HttpError> {
    let pag_params = query.into_inner();
    let limit = rqctx.page_limit(&pag_params)?.get();
    let data = rqctx_to_data(rqctx);
    let scan_mode = match &pag_params.page_params {
        WhichPage::First(ProjectScanParams {
            sort_mode: None,
        }) => ProjectScanMode::ByNameAscending,

        WhichPage::First(ProjectScanParams {
            sort_mode: Some(p),
        }) => p.clone(),

        WhichPage::Next(ProjectScanPageSelector::Name(Ascending, ..)) => {
            ProjectScanMode::ByNameAscending
        }
        WhichPage::Next(ProjectScanPageSelector::Name(Descending, ..)) => {
            ProjectScanMode::ByNameDescending
        }
        WhichPage::Next(ProjectScanPageSelector::MtimeName(Ascending, ..)) => {
            ProjectScanMode::ByMtimeAscending
        }
        WhichPage::Next(ProjectScanPageSelector::MtimeName(Descending, ..)) => {
            ProjectScanMode::ByMtimeDescending
        }
    };

    let iter = match &pag_params.page_params {
        WhichPage::First(..) => match scan_mode {
            ProjectScanMode::ByNameAscending => data.iter_by_name_asc(),
            ProjectScanMode::ByNameDescending => data.iter_by_name_desc(),
            ProjectScanMode::ByMtimeAscending => data.iter_by_mtime_asc(),
            ProjectScanMode::ByMtimeDescending => data.iter_by_mtime_desc(),
        },

        WhichPage::Next(ProjectScanPageSelector::Name(Ascending, name)) => {
            data.iter_by_name_asc_from(name)
        }
        WhichPage::Next(ProjectScanPageSelector::Name(Descending, name)) => {
            data.iter_by_name_desc_from(name)
        }
        WhichPage::Next(ProjectScanPageSelector::MtimeName(
            Ascending,
            mtime,
            name,
        )) => data.iter_by_mtime_asc_from(mtime, name),
        WhichPage::Next(ProjectScanPageSelector::MtimeName(
            Descending,
            mtime,
            name,
        )) => data.iter_by_mtime_desc_from(mtime, name),
    };

    let projects = iter.take(limit).map(|p| (*p).clone()).collect();
    Ok(HttpResponseOkPage::new_with_paginator(
        projects,
        &scan_mode,
        page_selector_for,
    )?)
}

fn rqctx_to_data(rqctx: Arc<RequestContext>) -> Arc<ProjectCollection> {
    let c = Arc::clone(&rqctx.server.private);
    c.downcast::<ProjectCollection>().unwrap()
}

#[tokio::main]
async fn main() -> Result<(), String> {
    /*
     * Run the Dropshot server.
     */
    let ctx = Arc::new(ProjectCollection::new());
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

    /*
     * Print out some example requests to start with.
     */
    print_example_requests(log, &server.local_addr());

    server.wait_for_shutdown(server_task).await
}

fn print_example_requests(log: slog::Logger, addr: &SocketAddr) {
    let all_modes = vec![
        ProjectScanMode::ByNameAscending,
        ProjectScanMode::ByNameDescending,
        ProjectScanMode::ByMtimeAscending,
        ProjectScanMode::ByMtimeDescending,
    ];
    for mode in all_modes {
        let to_print = ProjectScanParams {
            sort_mode: Some(mode),
        };
        let query_string = serde_urlencoded::to_string(to_print).unwrap();
        let uri = Uri::builder()
            .scheme("http")
            .authority(addr.to_string().as_str())
            .path_and_query(format!("/projects?{}", query_string).as_str())
            .build()
            .unwrap();
        info!(log, "example: {}", uri);
    }
}

/**
 * Tracks a (static) collection of Projects indexed in two different ways to
 * demonstrate an endpoint that provides multiple ways to scan a large
 * collection.
 */
struct ProjectCollection {
    by_name: BTreeMap<String, Arc<Project>>,
    by_mtime: BTreeMap<(DateTime<Utc>, String), Arc<Project>>,
}

type ProjectIter<'a> = Box<dyn Iterator<Item = Arc<Project>> + 'a>;

impl ProjectCollection {
    /** Constructs an example collection of projects to back the API endpoint */
    pub fn new() -> ProjectCollection {
        let mut data = ProjectCollection {
            by_name: BTreeMap::new(),
            by_mtime: BTreeMap::new(),
        };
        let mut timestamp =
            DateTime::parse_from_rfc3339("2020-07-13T17:35:00Z")
                .unwrap()
                .timestamp_millis();
        for n in 1..1000 {
            let name = format!("project{:03}", n);
            let project = Arc::new(Project {
                name: name.clone(),
                mtime: Utc.timestamp_millis(timestamp),
            });
            /*
             * To make this dataset at least somewhat interesting in terms of
             * exercising different pagination parameters, we'll make the mtimes
             * decrease with the names, and we'll have some objects with the same
             * mtime.
             */
            if n % 10 != 0 {
                timestamp = timestamp - 1;
            }
            data.by_name.insert(name.clone(), Arc::clone(&project));
            data.by_mtime.insert((project.mtime, name), project);
        }

        data
    }

    /*
     * Iterate by name (ascending, descending)
     */

    pub fn iter_by_name_asc(&self) -> ProjectIter {
        self.make_iter(self.by_name.iter())
    }
    pub fn iter_by_name_desc(&self) -> ProjectIter {
        self.make_iter(self.by_name.iter().rev())
    }
    pub fn iter_by_name_asc_from(&self, last_seen: &String) -> ProjectIter {
        let iter = self
            .by_name
            .range((Bound::Excluded(last_seen.clone()), Bound::Unbounded));
        self.make_iter(iter)
    }
    pub fn iter_by_name_desc_from(&self, last_seen: &String) -> ProjectIter {
        let iter = self
            .by_name
            .range((Bound::Unbounded, Bound::Excluded(last_seen.clone())))
            .rev();
        self.make_iter(iter)
    }

    /*
     * Iterate by mtime (ascending, descending)
     */

    pub fn iter_by_mtime_asc(&self) -> ProjectIter {
        self.make_iter(self.by_mtime.iter())
    }
    pub fn iter_by_mtime_desc(&self) -> ProjectIter {
        self.make_iter(self.by_mtime.iter().rev())
    }
    pub fn iter_by_mtime_asc_from(
        &self,
        last_mtime: &DateTime<Utc>,
        last_name: &String,
    ) -> ProjectIter {
        let last_seen = &(*last_mtime, last_name.clone());
        let iter =
            self.by_mtime.range((Bound::Excluded(last_seen), Bound::Unbounded));
        self.make_iter(iter)
    }
    pub fn iter_by_mtime_desc_from(
        &self,
        last_mtime: &DateTime<Utc>,
        last_name: &String,
    ) -> ProjectIter {
        let last_seen = &(*last_mtime, last_name.clone());
        let iter = self
            .by_mtime
            .range((Bound::Unbounded, Bound::Excluded(last_seen)))
            .rev();
        self.make_iter(iter)
    }

    /**
     * Helper function to turn the initial iterators produced above into what we
     * actually need to provide consumers.
     */
    fn make_iter<'a, K, I>(&'a self, iter: I) -> ProjectIter<'a>
    where
        I: Iterator<Item = (K, &'a Arc<Project>)> + 'a,
    {
        Box::new(iter.map(|(_, project)| Arc::clone(project)))
    }
}
