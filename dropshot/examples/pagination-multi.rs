/*!
 * Example of an API endpoint that supports pagination using several different
 * fields as the sorting key.
 *
 * When you run this program, it will start an HTTP server on an available local
 * port.  See the log for example URLs to use.  Try passing different values of
 * the `limit` query parameter.  Try passing the `next_page` token from the
 * response as a query parameter called `page_token`, too.
 *
 * Key terms:
 *
 * * This server exposes a single **API endpoint** that returns the **items**
 *   contained within a **collection**.
 * * The client is not allowed to list the entire collection in one request.
 *   Instead, they list the collection using a sequence of requests to the one
 *   endpoint.  This sequence of requests is a **scan** of the collection, and
 *   we sometimes say that the client **pages through** the collection.
 * * The initial request in the scan may specify the **scan mode**, which
 *   typically specifies how the results are to be sorted (i.e., by which field
 *   and whether the sort is ascending or descending).
 * * Each request returns a **page** of results at a time, along with a **page
 *   token** that's provided with the next request as a query parameter.
 * * The scan mode cannot change between requests that are part of the same
 *   scan.
 *
 * XXX document the query parameters here with examples
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
use dropshot::PaginatedResource;
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
use std::convert::TryFrom;
use std::net::SocketAddr;
use std::ops::Bound;
use std::sync::Arc;

#[macro_use]
extern crate slog;

/**
 * Structure on which we hang our implementation of [`PaginatedResource`].
 */
// XXX shouldn't need to be Deserialize
#[derive(Deserialize)]
struct ProjectScan;
impl PaginatedResource for ProjectScan {
    type ScanMode = ProjectScanMode;
    type PageSelector = ProjectScanPageSelector;
    type Item = Project;

    fn page_selector_for(
        last_item: &Project,
        scan_mode: &ProjectScanMode,
    ) -> ProjectScanPageSelector {
        match scan_mode {
            ProjectScanMode::ByNameAscending => {
                ProjectScanPageSelector::Name(Ascending, last_item.name.clone())
            }
            ProjectScanMode::ByNameDescending => ProjectScanPageSelector::Name(
                Descending,
                last_item.name.clone(),
            ),
            ProjectScanMode::ByMtimeDescending => {
                ProjectScanPageSelector::MtimeName(
                    Descending,
                    last_item.mtime,
                    last_item.name.clone(),
                )
            }
        }
    }

    fn scan_mode_for(
        which: &WhichPage<ProjectScan>,
    ) -> Result<ProjectScanMode, HttpError> {
        match which {
            WhichPage::FirstPage {
                list_mode: None,
            } => Ok(ProjectScanMode::ByNameAscending),

            WhichPage::FirstPage {
                list_mode: Some(p),
            } => Ok(p.clone()),

            WhichPage::NextPage {
                page_token,
            } => match &page_token.page_start {
                ProjectScanPageSelector::Name(Ascending, ..) => {
                    Ok(ProjectScanMode::ByNameAscending)
                }
                ProjectScanPageSelector::Name(Descending, ..) => {
                    Ok(ProjectScanMode::ByNameDescending)
                }
                ProjectScanPageSelector::MtimeName(Descending, ..) => {
                    Ok(ProjectScanMode::ByMtimeDescending)
                }
                _ => Err(HttpError::for_bad_request(
                    None,
                    String::from("unsupported scan mode"),
                )),
            },
        }
    }
}

/**
 * Specifies how the client wants to page through results (typically: what
 * field(s) to sort by and whether the sort should be ascending or descending)
 *
 * It's up to the consumer (e.g., this example) to decide exactly which modes
 * are supported here and what each one means.  This enum represents an
 * interface that's part of the OpenAPI specification for the service.
 * TODO-correctness: this structure should appear in the OpenAPI spec, but it
 * doesn't.
 *
 * NOTE: To be useful, this field must be deserializable using the
 * `serde_querystring` module.  You can test this by writing test code to
 * serialize it using `serde_querystring`.  That code could fail at runtime for
 * certain types of values (e.g., enum variants that contain data).
 */
#[derive(
    Deserialize, Clone, Debug, ExtractedParameter, JsonSchema, Serialize,
)]
#[serde(rename_all = "kebab-case")]
enum ProjectScanMode {
    /** by name ascending */
    ByNameAscending,
    /** by name descending */
    ByNameDescending,
    /** by mtime descending, then by name ascending */
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
 *
 * In our case, we support a limited set of scan modes.  To keep future options
 * open to support things like `ByMtimeAscending`, the structure for the page
 * selector is more general.
 */
#[derive(Debug, Deserialize, ExtractedParameter, JsonSchema, Serialize)]
#[serde(rename_all = "lowercase")]
enum ProjectScanPageSelector {
    Name(PaginationOrder, String),
    MtimeName(PaginationOrder, DateTime<Utc>, String),
}

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
 * For our own convenience, we define a conversion that extracts the scan mode
 * out of the page selector.  This can fail because as noted above, our page
 * selector supports configurations for which we don't define a real scan mode.
 * (Clients have no way to use these directly, but the serialization format
 * allows us to add this compatibly in the future.)
 */
impl TryFrom<&ProjectScanPageSelector> for ProjectScanMode {
    type Error = HttpError;

    fn try_from(
        p: &ProjectScanPageSelector,
    ) -> Result<ProjectScanMode, HttpError> {
        match p {
            ProjectScanPageSelector::Name(Ascending, ..) => {
                Ok(ProjectScanMode::ByNameAscending)
            }
            ProjectScanPageSelector::Name(Descending, ..) => {
                Ok(ProjectScanMode::ByNameDescending)
            }
            ProjectScanPageSelector::MtimeName(Descending, ..) => {
                Ok(ProjectScanMode::ByMtimeDescending)
            }
            _ => Err(HttpError::for_bad_request(
                None,
                String::from("unsupported scan mode"),
            )),
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
    query: Query<PaginationParams<ProjectScan>>,
) -> Result<HttpResponseOkPage<ProjectScan>, HttpError> {
    let pag_params = query.into_inner();
    let limit = rqctx.page_limit(&pag_params)?.get();

    let data = rqctx_to_data(rqctx);
    let scan_mode = ProjectScan::scan_mode_for(&pag_params.page_params)?;
    let iter = match &pag_params.page_params {
        WhichPage::FirstPage { .. } => {
            match scan_mode {
                ProjectScanMode::ByNameAscending => data.iter_by_name_asc(),
                ProjectScanMode::ByNameDescending => data.iter_by_name_desc(),
                ProjectScanMode::ByMtimeDescending => data.iter_by_mtime_desc(),
            }
        }

        WhichPage::NextPage {
            page_token: page_params,
        } => {
            match &page_params.page_start {
                ProjectScanPageSelector::Name(Ascending, name) => {
                    data.iter_by_name_asc_from(name)
                }
                ProjectScanPageSelector::Name(Descending, name) => {
                    data.iter_by_name_desc_from(name)
                }
                ProjectScanPageSelector::MtimeName(Ascending, mtime, name) => {
                    data.iter_by_mtime_asc_from(mtime, name)
                }
                ProjectScanPageSelector::MtimeName(Descending, mtime, name) => {
                    data.iter_by_mtime_desc_from(mtime, name)
                }
            }
        }
    };

    let projects = iter.take(limit).map(|p| (*p).clone()).collect();
    Ok(HttpResponseOkPage(scan_mode, projects))
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
    /*
     * We want to print out the valid querystring values for our API endpoint.
     * Querystrings have to be a sequence key-value pairs.  Variants of
     * ProjectScanMode wind up being values, so they need to be wrapped in a
     * structure with a key.  The correct key is "list_mode" -- see
     * [`dropshot::PaginationParams`]`.page_params`, which is a
     * [`dropshot::WhichPage::FirstPage`].  It would be simpler to use
     * `WhichPage::FirstPage` directly here, but that doesn't implement
     * `Serialize`.  (Dropshot doesn't even require that `ProjectScanMode`
     * implement `Serialize`).
     */
    #[derive(Serialize)]
    struct ToPrint {
        list_mode: ProjectScanMode,
    };
    let all_modes = vec![
        ProjectScanMode::ByNameAscending,
        ProjectScanMode::ByNameDescending,
        ProjectScanMode::ByMtimeDescending,
    ];
    for mode in all_modes {
        let to_print = ToPrint {
            list_mode: mode,
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
