// Copyright 2020 Oxide Computer Company
/*!
 * Example that shows a paginated API that uses the same pagination fields on
 * multiple resources.  See the other pagination examples for more information
 * about how to run this.
 */

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::ConfigDropshot;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpResponseOkObject;
use dropshot::HttpServer;
use dropshot::PaginationOrder;
use dropshot::PaginationOrder::Ascending;
use dropshot::PaginationOrder::Descending;
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
use uuid::Uuid;

/*
 * Example API data model: we have three resources, each having an "id" and
 * "name".  We'll have one endpoint for each resource to list it.
 */

#[derive(Clone, JsonSchema, Serialize)]
struct Project {
    id: Uuid,
    name: String,
    // lots more project-like fields
}

#[derive(Clone, JsonSchema, Serialize)]
struct Disk {
    id: Uuid,
    name: String,
    // lots more disk-like fields
}

#[derive(Clone, JsonSchema, Serialize)]
struct Instance {
    id: Uuid,
    name: String,
    // lots more instance-like fields
}

/*
 * In an API with many resources sharing the same identifying fields, we might
 * define a trait to get those fields.  Then we could define pagination in terms
 * of that trait.  To avoid hand-writing the impls, we use a macro.  (This might
 * be better as a "derive" procedural macro.)
 */
trait HasIdentity {
    fn id(&self) -> &Uuid;
    fn name(&self) -> &String;
}

macro_rules! impl_HasIdentity {
    ($T:ident) => {
        impl HasIdentity for $T {
            fn id(&self) -> &Uuid {
                &self.id
            }
            fn name(&self) -> &String {
                &self.name
            }
        }
    };
}

impl_HasIdentity!(Project);
impl_HasIdentity!(Disk);
impl_HasIdentity!(Instance);

/*
 * Pagination-related types
 */
#[derive(Deserialize, Clone, JsonSchema, Serialize)]
struct ExScanParams {
    #[serde(default = "default_sort_mode")]
    sort: ExSortMode,
}

fn default_sort_mode() -> ExSortMode {
    ExSortMode::ByNameAscending
}

#[derive(Deserialize, Clone, JsonSchema, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ExSortMode {
    ByIdAscending,
    ByIdDescending,
    ByNameAscending,
    ByNameDescending,
}

#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ExPageSelector {
    Id(PaginationOrder, Uuid),
    Name(PaginationOrder, String),
}

fn page_selector<T: HasIdentity>(
    item: &T,
    scan_params: &ExScanParams,
) -> ExPageSelector {
    match scan_params {
        ExScanParams {
            sort: ExSortMode::ByIdAscending,
        } => ExPageSelector::Id(Ascending, *item.id()),
        ExScanParams {
            sort: ExSortMode::ByIdDescending,
        } => ExPageSelector::Id(Descending, *item.id()),
        ExScanParams {
            sort: ExSortMode::ByNameAscending,
        } => ExPageSelector::Name(Ascending, item.name().clone()),
        ExScanParams {
            sort: ExSortMode::ByNameDescending,
        } => ExPageSelector::Name(Descending, item.name().clone()),
    }
}

fn scan_params(p: &WhichPage<ExScanParams, ExPageSelector>) -> ExScanParams {
    ExScanParams {
        sort: match p {
            WhichPage::First(ExScanParams {
                sort,
            }) => sort.clone(),

            WhichPage::Next(ExPageSelector::Id(Ascending, ..)) => {
                ExSortMode::ByIdAscending
            }
            WhichPage::Next(ExPageSelector::Id(Descending, ..)) => {
                ExSortMode::ByIdDescending
            }
            WhichPage::Next(ExPageSelector::Name(Ascending, ..)) => {
                ExSortMode::ByNameAscending
            }
            WhichPage::Next(ExPageSelector::Name(Descending, ..)) => {
                ExSortMode::ByNameDescending
            }
        },
    }
}

/*
 * Paginated endpoints to list each type of resource.
 *
 * These could be commonized further (to the point where each of these endpoint
 * functions is just a one-line call to a generic function), but we implement
 * them separately here for clarity.
 */

#[endpoint {
    method = GET,
    path = "/projects"
}]
async fn example_list_projects(
    rqctx: Arc<RequestContext>,
    query: Query<PaginationParams<ExScanParams, ExPageSelector>>,
) -> Result<HttpResponseOkObject<ResultsPage<Project>>, HttpError> {
    let pag_params = query.into_inner();
    let limit = rqctx.page_limit(&pag_params)?.get();
    let data = rqctx_to_data(rqctx);
    let scan_params = scan_params(&pag_params.page);

    let iter = do_list(
        &data,
        &scan_params,
        &pag_params.page,
        &data.projects_by_name,
        &data.projects_by_id,
    );

    let items = iter.take(limit).map(|p| (*p).clone()).collect();

    Ok(HttpResponseOkObject(ResultsPage::new(
        items,
        &scan_params,
        page_selector,
    )?))
}

#[endpoint {
    method = GET,
    path = "/disks"
}]
async fn example_list_disks(
    rqctx: Arc<RequestContext>,
    query: Query<PaginationParams<ExScanParams, ExPageSelector>>,
) -> Result<HttpResponseOkObject<ResultsPage<Disk>>, HttpError> {
    let pag_params = query.into_inner();
    let limit = rqctx.page_limit(&pag_params)?.get();
    let data = rqctx_to_data(rqctx);
    let scan_params = scan_params(&pag_params.page);

    let iter = do_list(
        &data,
        &scan_params,
        &pag_params.page,
        &data.disks_by_name,
        &data.disks_by_id,
    );

    let items = iter.take(limit).map(|p| (*p).clone()).collect();

    Ok(HttpResponseOkObject(ResultsPage::new(
        items,
        &scan_params,
        page_selector,
    )?))
}

#[endpoint {
    method = GET,
    path = "/instances"
}]
async fn example_list_instances(
    rqctx: Arc<RequestContext>,
    query: Query<PaginationParams<ExScanParams, ExPageSelector>>,
) -> Result<HttpResponseOkObject<ResultsPage<Instance>>, HttpError> {
    let pag_params = query.into_inner();
    let limit = rqctx.page_limit(&pag_params)?.get();
    let data = rqctx_to_data(rqctx);
    let scan_params = scan_params(&pag_params.page);

    let iter = do_list(
        &data,
        &scan_params,
        &pag_params.page,
        &data.instances_by_name,
        &data.instances_by_id,
    );

    let items = iter.take(limit).map(|p| (*p).clone()).collect();

    Ok(HttpResponseOkObject(ResultsPage::new(
        items,
        &scan_params,
        page_selector,
    )?))
}

fn do_list<'a, T>(
    data: &'a Arc<DataCollection>,
    scan_params: &ExScanParams,
    p: &'a WhichPage<ExScanParams, ExPageSelector>,
    by_name: &'a BTreeMap<String, Arc<T>>,
    by_id: &'a BTreeMap<Uuid, Arc<T>>,
) -> ItemIter<'a, T>
where
    T: Clone + JsonSchema + Serialize + Send + Sync + 'static,
{
    match p {
        WhichPage::First(_) => match scan_params.sort {
            ExSortMode::ByIdAscending => data.iter_asc(by_id),
            ExSortMode::ByIdDescending => data.iter_desc(by_id),
            ExSortMode::ByNameAscending => data.iter_asc(by_name),
            ExSortMode::ByNameDescending => data.iter_desc(by_name),
        },

        WhichPage::Next(ExPageSelector::Id(Ascending, id)) => {
            data.iter_asc_from(by_id, id)
        }
        WhichPage::Next(ExPageSelector::Id(Descending, id)) => {
            data.iter_desc_from(by_id, id)
        }
        WhichPage::Next(ExPageSelector::Name(Ascending, name)) => {
            data.iter_asc_from(by_name, name)
        }
        WhichPage::Next(ExPageSelector::Name(Descending, name)) => {
            data.iter_desc_from(by_name, name)
        }
    }
}

/*
 * General Dropshot-server boilerplate
 */

#[tokio::main]
async fn main() -> Result<(), String> {
    let port = std::env::args()
        .nth(1)
        .map(|p| p.parse::<u16>())
        .transpose()
        .map_err(|e| format!("failed to parse \"port\" argument: {}", e))?
        .unwrap_or(0);

    /*
     * Run the Dropshot server.
     */
    let ctx = Arc::new(DataCollection::new());
    let config_dropshot = ConfigDropshot {
        bind_address: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
    };
    let config_logging = ConfigLogging::StderrTerminal {
        level: ConfigLoggingLevel::Debug,
    };
    let log = config_logging
        .to_logger("example-pagination-basic")
        .map_err(|error| format!("failed to create logger: {}", error))?;
    let mut api = ApiDescription::new();
    api.register(example_list_projects).unwrap();
    api.register(example_list_disks).unwrap();
    api.register(example_list_instances).unwrap();
    let mut server = HttpServer::new(&config_dropshot, api, ctx, &log)
        .map_err(|error| format!("failed to create server: {}", error))?;
    let server_task = server.run();
    server.wait_for_shutdown(server_task).await
}

fn rqctx_to_data(rqctx: Arc<RequestContext>) -> Arc<DataCollection> {
    let c = Arc::clone(&rqctx.server.private);
    c.downcast::<DataCollection>().unwrap()
}

/**
 * Tracks a (static) collection of Projects indexed in two different ways to
 * demonstrate an endpoint that provides multiple ways to scan a large
 * collection.
 */
struct DataCollection {
    projects_by_name: BTreeMap<String, Arc<Project>>,
    projects_by_id: BTreeMap<Uuid, Arc<Project>>,
    disks_by_name: BTreeMap<String, Arc<Disk>>,
    disks_by_id: BTreeMap<Uuid, Arc<Disk>>,
    instances_by_name: BTreeMap<String, Arc<Instance>>,
    instances_by_id: BTreeMap<Uuid, Arc<Instance>>,
}

type ItemIter<'a, T> = Box<dyn Iterator<Item = Arc<T>> + 'a>;

impl DataCollection {
    /**
     * Constructs an example collection of projects, disks, and instances to
     * back the API endpoints
     */
    pub fn new() -> DataCollection {
        let mut data = DataCollection {
            projects_by_id: BTreeMap::new(),
            projects_by_name: BTreeMap::new(),
            disks_by_id: BTreeMap::new(),
            disks_by_name: BTreeMap::new(),
            instances_by_id: BTreeMap::new(),
            instances_by_name: BTreeMap::new(),
        };
        for n in 1..1000 {
            let pname = format!("project{:03}", n);
            let project = Arc::new(Project {
                id: Uuid::new_v4(),
                name: pname.clone(),
            });
            data.projects_by_name.insert(pname.clone(), Arc::clone(&project));
            data.projects_by_id.insert(project.id, project);

            let dname = format!("disk{:03}", n);
            let disk = Arc::new(Disk {
                id: Uuid::new_v4(),
                name: dname.clone(),
            });
            data.disks_by_name.insert(dname.clone(), Arc::clone(&disk));
            data.disks_by_id.insert(disk.id, disk);

            let iname = format!("disk{:03}", n);
            let instance = Arc::new(Instance {
                id: Uuid::new_v4(),
                name: iname.clone(),
            });
            data.instances_by_name.insert(iname.clone(), Arc::clone(&instance));
            data.instances_by_id.insert(instance.id, instance);
        }

        data
    }

    pub fn iter_asc<'a, T: Clone + 'static, K>(
        &'a self,
        tree: &'a BTreeMap<K, Arc<T>>,
    ) -> ItemIter<'a, T> {
        self.make_iter(tree.iter())
    }

    pub fn iter_desc<'a, T: Clone + 'static, K>(
        &'a self,
        tree: &'a BTreeMap<K, Arc<T>>,
    ) -> ItemIter<'a, T> {
        self.make_iter(tree.iter().rev())
    }

    pub fn iter_asc_from<'a, T: Clone + 'static, K: Clone + Ord>(
        &'a self,
        tree: &'a BTreeMap<K, Arc<T>>,
        last_seen: &K,
    ) -> ItemIter<'a, T> {
        let iter =
            tree.range((Bound::Excluded(last_seen.clone()), Bound::Unbounded));
        self.make_iter(iter)
    }

    pub fn iter_desc_from<'a, T: Clone + 'static, K: Clone + Ord>(
        &'a self,
        tree: &'a BTreeMap<K, Arc<T>>,
        last_seen: &K,
    ) -> ItemIter<'a, T> {
        let iter = tree
            .range((Bound::Unbounded, Bound::Excluded(last_seen.clone())))
            .rev();
        self.make_iter(iter)
    }

    /**
     * Helper function to turn the initial iterators produced above into what we
     * actually need to provide consumers.
     */
    fn make_iter<'a, K, I, T>(&'a self, iter: I) -> ItemIter<'a, T>
    where
        I: Iterator<Item = (K, &'a Arc<T>)> + 'a,
        T: Clone + 'static,
    {
        Box::new(iter.map(|(_, item)| Arc::clone(item)))
    }
}
