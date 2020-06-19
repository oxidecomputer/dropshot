/*!
 * Automated testing facilities.  These are intended for use both by this crate
 * and dependents of this crate.
 */

use chrono::DateTime;
use chrono::Utc;
use http::method::Method;
use hyper::body::to_bytes;
use hyper::client::HttpConnector;
use hyper::Body;
use hyper::Client;
use hyper::Request;
use hyper::Response;
use hyper::StatusCode;
use hyper::Uri;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use slog::Logger;
use std::any::Any;
use std::fmt::Debug;
use std::fs;
use std::iter::Iterator;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::task::JoinHandle;

use crate::api_description::ApiDescription;
use crate::config::ConfigDropshot;
use crate::error::HttpErrorResponseBody;
use crate::logging::ConfigLogging;
use crate::server::HttpServer;

/**
 * List of allowed HTTP headers in responses.  This is used to make sure we
 * don't leak headers unexpectedly.
 */
const ALLOWED_HEADER_NAMES: [&str; 4] =
    ["content-length", "content-type", "date", "x-request-id"];

/**
 * ClientTestContext encapsulates several facilities associated with using an
 * HTTP client for testing.
 */
pub struct ClientTestContext {
    /** actual bind address of the HTTP server under test */
    bind_address: SocketAddr,
    /** HTTP client, used for making requests against the test server */
    client: Client<HttpConnector>,
    /** logger for the test suite HTTP client */
    client_log: Logger,
}

impl ClientTestContext {
    /**
     * Set up a `ClientTestContext` for running tests against an API server.
     */
    pub fn new(server_addr: SocketAddr, log: Logger) -> ClientTestContext {
        ClientTestContext {
            bind_address: server_addr,
            client: Client::new(),
            client_log: log,
        }
    }

    /**
     * Given the path for an API endpoint (e.g., "/projects"), return a Uri that
     * we can use to invoke this endpoint from the client.  This essentially
     * appends the path to a base URL constructed from the server's IP address
     * and port.
     */
    fn url(&self, path: &str) -> Uri {
        Uri::builder()
            .scheme("http")
            .authority(format!("{}", self.bind_address).as_str())
            .path_and_query(path)
            .build()
            .expect("attempted to construct invalid URI")
    }

    /**
     * Execute an HTTP request against the test server and perform basic
     * validation of the result, including:
     *
     * - the expected status code
     * - the expected Date header (within reason)
     * - for error responses: the expected body content
     * - header names are in allowed list
     * - any other semantics that can be verified in general
     */
    pub async fn make_request<RequestBodyType: Serialize + Debug>(
        &self,
        method: Method,
        path: &str,
        request_body: Option<RequestBodyType>,
        expected_status: StatusCode,
    ) -> Result<Response<Body>, HttpErrorResponseBody> {
        let body: Body = match request_body {
            None => Body::empty(),
            Some(input) => serde_json::to_string(&input).unwrap().into(),
        };

        self.make_request_with_body(method, path, body, expected_status).await
    }

    pub async fn make_request_no_body(
        &self,
        method: Method,
        path: &str,
        expected_status: StatusCode,
    ) -> Result<Response<Body>, HttpErrorResponseBody> {
        self.make_request_with_body(
            method,
            path,
            Body::empty(),
            expected_status,
        )
        .await
    }

    /**
     * Fetches a resource for which we expect to get an error response.
     */
    pub async fn make_request_error(
        &self,
        method: Method,
        path: &str,
        expected_status: StatusCode,
    ) -> HttpErrorResponseBody {
        self.make_request_with_body(method, path, "".into(), expected_status)
            .await
            .unwrap_err()
    }

    /**
     * Fetches a resource for which we expect to get an error response.
     * TODO-cleanup the make_request_error* interfaces are slightly different
     * than the non-error ones (and probably a bit more ergonomic).
     */
    pub async fn make_request_error_body<T: Serialize + Debug>(
        &self,
        method: Method,
        path: &str,
        body: T,
        expected_status: StatusCode,
    ) -> HttpErrorResponseBody {
        self.make_request(method, path, Some(body), expected_status)
            .await
            .unwrap_err()
    }

    pub async fn make_request_with_body(
        &self,
        method: Method,
        path: &str,
        body: Body,
        expected_status: StatusCode,
    ) -> Result<Response<Body>, HttpErrorResponseBody> {
        let uri = self.url(path);

        let time_before = chrono::offset::Utc::now().timestamp();
        info!(self.client_log, "client request";
            "method" => %method,
            "uri" => %uri,
            "body" => ?&body,
        );

        let mut response = self
            .client
            .request(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(body)
                    .expect("attempted to construct invalid request"),
            )
            .await
            .expect("failed to make request to server");

        /* Check that we got the expected response code. */
        let status = response.status();
        info!(self.client_log, "client received response"; "status" => ?status);
        assert_eq!(expected_status, status);

        /*
         * Check that we didn't have any unexpected headers.  This could be more
         * efficient by putting the allowed headers into a BTree or Hash, but
         * right now the structure is tiny and it's convenient to have it
         * statically-defined above.
         */
        let headers = response.headers();
        for header_name in headers.keys() {
            let mut okay = false;
            for allowed_name in ALLOWED_HEADER_NAMES.iter() {
                if header_name == allowed_name {
                    okay = true;
                    break;
                }
            }

            if !okay {
                panic!("header name not in allowed list: \"{}\"", header_name);
            }
        }

        /*
         * Sanity check the Date header in the response.  Note that this
         * assertion will fail spuriously in the unlikely event that the system
         * clock is adjusted backwards in between when we sent the request and
         * when we received the response, but we consider that case unlikely
         * enough to be worth doing this check anyway.  (We'll try to check for
         * the clock reset condition, too, but we cannot catch all cases that
         * would cause the Date header check to be incorrect.)
         *
         * Note that the Date header typically only has precision down to one
         * second, so we don't want to try to do a more precise comparison.
         */
        let time_after = chrono::offset::Utc::now().timestamp();
        let date_header = headers
            .get(http::header::DATE)
            .expect("missing Date header")
            .to_str()
            .expect("non-ASCII characters in Date header");
        let time_request = chrono::DateTime::parse_from_rfc2822(date_header)
            .expect("unable to parse server's Date header");
        assert!(
            time_before <= time_after,
            "time obviously went backwards during the test"
        );
        assert!(time_request.timestamp() >= time_before - 1);
        assert!(time_request.timestamp() <= time_after + 1);

        /*
         * Validate that we have a request id header.
         * TODO-coverage check that it's unique among requests we've issued
         */
        let request_id_header = headers
            .get(crate::HEADER_REQUEST_ID)
            .expect("missing request id header")
            .to_str()
            .expect("non-ASCII characters in request id")
            .to_string();

        /*
         * For "204 No Content" responses, validate that we got no content in the
         * body.
         */
        if status == StatusCode::NO_CONTENT {
            let body_bytes = to_bytes(response.body_mut())
                .await
                .expect("error reading body");
            assert_eq!(0, body_bytes.len());
        }

        /*
         * If this was a successful response, there's nothing else to check
         * here.  Return the response so the caller can validate the content if
         * they want.
         */
        if !status.is_client_error() && !status.is_server_error() {
            return Ok(response);
        }

        /*
         * We got an error.  Parse the response body to make sure it's valid and
         * then return that.
         */
        let error_body: HttpErrorResponseBody = read_json(&mut response).await;
        info!(self.client_log, "client error"; "error_body" => ?error_body);
        assert_eq!(error_body.request_id, request_id_header);
        Err(error_body)
    }
}

/**
 * Constructs a Logger for use by a test suite.  If a file-based logger is
 * requested, the file will be put in a temporary directory and the name will be
 * unique for a given test name and is likely to be unique across multiple runs
 * of this test.  The file will also be deleted if the test succeeds, indicated
 * by invoking [`LogContext::cleanup_successful`].  This way, you can debug a
 * test failure from the failed instance rather than hoping the failure is
 * reproducible.
 *
 * ## Example
 *
 * ```
 * # use dropshot::ConfigLoggingLevel;
 * #
 * # fn my_logging_config() -> ConfigLogging {
 * #     ConfigLogging::StderrTerminal {
 * #         level: ConfigLoggingLevel::Info,
 * #     }
 * # }
 * #
 * # fn some_invariant() -> bool {
 * #     true
 * # }
 * #
 * use dropshot::ConfigLogging;
 * use dropshot::test_util::LogContext;
 *
 * #[macro_use]
 * extern crate slog; /* for the `info!` macro below */
 *
 * # fn main() {
 * let log_config: ConfigLogging = my_logging_config();
 * let logctx = LogContext::new("my_test", &log_config);
 * let log = &logctx.log;
 *
 * /* Run your test.  Use the log like you normally would. */
 * info!(log, "the test is going great");
 * assert!(some_invariant());
 *
 * /* Upon successful completion, invoke `cleanup_successful()`. */
 * logctx.cleanup_successful();
 * # }
 * ```
 *
 * If the test fails (e.g., the `some_invariant()` assertion fails), the log
 * file will be retained.  If the test gets as far as calling
 * `cleanup_successful()`, the log file will be removed.
 *
 * Note that `cleanup_successful()` is not invoked automatically on `drop`
 * because that would remove the file even if the test failed, which isn't what
 * we want.  You have to explicitly call `cleanup_successful`.  Normally, you
 * just do this as one of the last steps in your test.  This pattern ensures
 * that the log file sticks around if the test fails, but is removed if the test
 * succeeds.
 */
pub struct LogContext {
    /** general-purpose logger */
    pub log: Logger,
    log_path: Option<PathBuf>,
}

impl LogContext {
    /**
     * Sets up a LogContext.  If `initial_config_logging` specifies a file-based
     * log (i.e., [`ConfigLogging::File`]), then the requested path _must_ be
     * the string `"UNUSED"` and it will be replaced with a file name (in a
     * temporary directory) containing `test_name` and other information to make
     * the filename likely to be unique across multiple runs (e.g., process id).
     */
    pub fn new(
        test_name: &str,
        initial_config_logging: &ConfigLogging,
    ) -> LogContext {
        /*
         * See above.  If the caller requested a file path, assert that the path
         * matches our sentinel (just to improve debuggability -- otherwise
         * people might be pretty confused about where the logs went).  Then
         * override the path with one uniquely generated for this test.
         * TODO-developer allow keeping the logs in successful cases with an
         * environment variable or other flag.
         */
        let (log_path, log_config) = match initial_config_logging {
            ConfigLogging::File {
                level,
                path: dummy_path,
                if_exists,
            } => {
                assert_eq!(
                    dummy_path, "UNUSED",
                    "for test suite logging configuration, when mode = \
                     \"file\" is used, the path MUST be the sentinel string \
                     \"UNUSED\".  It will be replaced with a unique path for \
                     each test."
                );
                let new_path = log_file_for_test(test_name);
                let new_path_str = new_path.as_path().display().to_string();
                eprintln!("log file: {:?}", new_path_str);
                (Some(new_path), ConfigLogging::File {
                    level: level.clone(),
                    path: new_path_str.clone(),
                    if_exists: if_exists.clone(),
                })
            }
            other_config @ _ => (None, other_config.clone()),
        };

        let log = log_config.to_logger(test_name).unwrap();
        LogContext {
            log: log,
            log_path: log_path,
        }
    }

    /**
     * Removes the log file, if this was a file-based logger.
     */
    pub fn cleanup_successful(self) {
        if let Some(ref log_path) = self.log_path {
            fs::remove_file(log_path).unwrap();
        }
    }
}

/**
 * TestContext is used to manage a matched server and client for the common
 * test-case pattern of setting up a logger, server, and client and tearing them
 * all down at the end.
 */
pub struct TestContext {
    pub client_testctx: ClientTestContext,
    pub server: HttpServer,
    pub log: Logger,
    server_task: JoinHandle<Result<(), hyper::error::Error>>,
    log_context: Option<LogContext>,
}

impl TestContext {
    /**
     * Instantiate a TestContext by creating a new Dropshot server with `api`,
     * `private`, `config_dropshot`, and `log`, and then creating a
     * `ClientTestContext` with whatever address the server wound up bound to.
     *
     * This interfaces requires that `config_dropshot.bind_address.port()` be
     * `0` to allow the server to bind to any available port.  This is necessary
     * in order for it to be used concurrently by many tests.
     */
    pub fn new(
        api: ApiDescription,
        private: Arc<dyn Any + Send + Sync + 'static>,
        config_dropshot: &ConfigDropshot,
        log_context: Option<LogContext>,
        log: Logger,
    ) -> TestContext {
        assert_eq!(
            0,
            config_dropshot.bind_address.port(),
            "test suite only supports binding on port 0 (any available port)"
        );

        /*
         * Set up the server itself.
         */
        let mut server =
            HttpServer::new(&config_dropshot, api, private, &log).unwrap();
        let server_task = server.run();

        let server_addr = server.local_addr();
        let client_log = log.new(o!("http_client" => "dropshot test suite"));
        let client_testctx = ClientTestContext::new(server_addr, client_log);

        TestContext {
            client_testctx,
            server,
            log,
            server_task,
            log_context,
        }
    }

    /**
     * Requests a graceful shutdown of the server, waits for that to complete,
     * and cleans up the associated log context (if any).
     */
    /* TODO-cleanup: is there an async analog to Drop? */
    pub async fn teardown(self) {
        self.server.close();
        let join_result = self.server_task.await.unwrap();
        join_result.expect("server stopped with an error");
        if let Some(log_context) = self.log_context {
            log_context.cleanup_successful();
        }
    }
}

/**
 * Given a Hyper Response whose body is expected to represent newline-separated
 * JSON, each line of which is expected to be parseable via Serde as type T,
 * asynchronously read the body of the response and parse it accordingly,
 * returning a vector of T.
 */
pub async fn read_ndjson<T: DeserializeOwned>(
    response: &mut Response<Body>,
) -> Vec<T> {
    let headers = response.headers();
    assert_eq!(
        crate::CONTENT_TYPE_NDJSON,
        headers.get(http::header::CONTENT_TYPE).expect("missing content-type")
    );
    let body_bytes =
        to_bytes(response.body_mut()).await.expect("error reading body");
    let body_string = String::from_utf8(body_bytes.as_ref().into())
        .expect("response contained non-UTF-8 bytes");

    /*
     * TODO-cleanup: Consider using serde_json::StreamDeserializer or maybe
     * implementing an NDJSON-based Serde type?
     * TODO-correctness: If we don't do that, this should split on (\r?\n)+ to
     * be NDJSON-compatible.
     */
    body_string
        .split("\n")
        .filter(|line| line.len() > 0)
        .map(|line| {
            serde_json::from_str(line)
                .expect("failed to parse server body as expected type")
        })
        .collect::<Vec<T>>()
}

/**
 * Given a Hyper response whose body is expected to be a JSON object that should
 * be parseable via Serde as type T, asynchronously read the body of the
 * response and parse it, returning an instance of T.
 */
pub async fn read_json<T: DeserializeOwned>(
    response: &mut Response<Body>,
) -> T {
    let headers = response.headers();
    assert_eq!(
        crate::CONTENT_TYPE_JSON,
        headers.get(http::header::CONTENT_TYPE).expect("missing content-type")
    );
    let body_bytes =
        to_bytes(response.body_mut()).await.expect("error reading body");
    serde_json::from_slice(body_bytes.as_ref())
        .expect("failed to parse server body as expected type")
}

/**
 * Given a Hyper Response whose body is expected to be a UTF-8-encoded string,
 * asynchronously read the body.
 */
pub async fn read_string(response: &mut Response<Body>) -> String {
    let body_bytes =
        to_bytes(response.body_mut()).await.expect("error reading body");
    String::from_utf8(body_bytes.as_ref().into())
        .expect("response contained non-UTF-8 bytes")
}

/**
 * Fetches a single resource from the API.
 */
pub async fn object_get<T: DeserializeOwned>(
    client: &ClientTestContext,
    object_url: &str,
) -> T {
    let mut response = client
        .make_request_with_body(
            Method::GET,
            &object_url,
            "".into(),
            StatusCode::OK,
        )
        .await
        .unwrap();
    read_json::<T>(&mut response).await
}

/**
 * Fetches a list of resources from the API.
 */
pub async fn objects_list<T: DeserializeOwned>(
    client: &ClientTestContext,
    list_url: &str,
) -> Vec<T> {
    let mut response = client
        .make_request_with_body(
            Method::GET,
            &list_url,
            "".into(),
            StatusCode::OK,
        )
        .await
        .unwrap();
    read_ndjson::<T>(&mut response).await
}

/**
 * Issues an HTTP POST to the specified collection URL to create an object.
 */
pub async fn objects_post<S: Serialize + Debug, T: DeserializeOwned>(
    client: &ClientTestContext,
    collection_url: &str,
    input: S,
) -> T {
    let mut response = client
        .make_request(
            Method::POST,
            &collection_url,
            Some(input),
            StatusCode::CREATED,
        )
        .await
        .unwrap();
    read_json::<T>(&mut response).await
}

static TEST_SUITE_LOGGER_ID: AtomicU32 = AtomicU32::new(0);

/**
 * Returns a unique path name in a temporary directory that includes the given
 * `test_name`.
 */
pub fn log_file_for_test(test_name: &str) -> PathBuf {
    let arg0 = {
        let arg0path = std::env::args().next().unwrap();
        Path::new(&arg0path).file_name().unwrap().to_str().unwrap().to_string()
    };

    let log_path = {
        let mut pathbuf = std::env::temp_dir();
        let id = TEST_SUITE_LOGGER_ID.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        pathbuf.push(format!("{}-{}.{}.{}.log", arg0, test_name, pid, id));
        pathbuf
    };

    log_path
}

/**
 * Load an object of type `T` (usually a hunk of configuration) from the string
 * `contents`.  `label` is used as an identifying string in a log message.  It
 * should be unique for each test.
 */
pub fn read_config<T: DeserializeOwned + Debug>(
    label: &str,
    contents: &str,
) -> Result<T, toml::de::Error> {
    let result = toml::from_str(contents);
    eprintln!("config \"{}\": {:?}", label, result);
    result
}

/*
 * Bunyan testing facilities
 */

/**
 * Represents a Bunyan log record.  This form does not support any non-standard
 * fields.  "level" is not yet supported because we don't (yet) need it.
 */
#[derive(Deserialize)]
pub struct BunyanLogRecord {
    pub time: DateTime<Utc>,
    pub name: String,
    pub hostname: String,
    pub pid: u32,
    pub msg: String,
    pub v: usize,
}

/**
 * Read a file containing a Bunyan-format log, returning an array of records.
 */
pub fn read_bunyan_log(logpath: &Path) -> Vec<BunyanLogRecord> {
    let log_contents = fs::read_to_string(logpath).unwrap();
    let log_records = log_contents
        .split("\n")
        .filter(|line| line.len() > 0)
        .map(|line| serde_json::from_str::<BunyanLogRecord>(line).unwrap())
        .collect::<Vec<BunyanLogRecord>>();
    log_records
}

/**
 * Analogous to a BunyanLogRecord, but where all fields are optional.
 */
pub struct BunyanLogRecordSpec {
    pub name: Option<String>,
    pub hostname: Option<String>,
    pub pid: Option<u32>,
    pub v: Option<usize>,
}

/**
 * Verify that the key fields of the log records emitted by `iter` match the
 * corresponding values in `expected`.  Fields that are `None` in `expected`
 * will not be checked.
 */
pub fn verify_bunyan_records<'a, 'b, I>(
    iter: I,
    expected: &'a BunyanLogRecordSpec,
) where
    I: Iterator<Item = &'b BunyanLogRecord>,
{
    for record in iter {
        if let Some(ref expected_name) = expected.name {
            assert_eq!(expected_name, &record.name);
        }
        if let Some(ref expected_hostname) = expected.hostname {
            assert_eq!(expected_hostname, &record.hostname);
        }
        if let Some(expected_pid) = expected.pid {
            assert_eq!(expected_pid, record.pid);
        }
        if let Some(expected_v) = expected.v {
            assert_eq!(expected_v, record.v);
        }
    }
}

/**
 * Verify that the Bunyan records emitted by `iter` are chronologically
 * sequential and after `maybe_time_before` and before `maybe_time_after`, if
 * those latter two parameters are specified.
 */
pub fn verify_bunyan_records_sequential<'a, 'b, I>(
    iter: I,
    maybe_time_before: Option<&'a DateTime<Utc>>,
    maybe_time_after: Option<&'a DateTime<Utc>>,
) where
    I: Iterator<Item = &'a BunyanLogRecord>,
{
    let mut maybe_should_be_before = maybe_time_before;

    for record in iter {
        if let Some(should_be_before) = maybe_should_be_before {
            assert!(should_be_before.timestamp() <= record.time.timestamp());
        }
        maybe_should_be_before = Some(&record.time);
    }

    if let Some(should_be_before) = maybe_should_be_before {
        if let Some(time_after) = maybe_time_after {
            assert!(should_be_before.timestamp() <= time_after.timestamp());
        }
    }
}

#[cfg(test)]
mod test {
    const T1_STR: &str = "2020-03-24T00:00:00Z";
    const T2_STR: &str = "2020-03-25T00:00:00Z";

    use super::verify_bunyan_records;
    use super::verify_bunyan_records_sequential;
    use super::BunyanLogRecord;
    use super::BunyanLogRecordSpec;
    use chrono::DateTime;
    use chrono::Utc;

    fn make_dummy_record() -> BunyanLogRecord {
        let t1: DateTime<Utc> =
            DateTime::parse_from_rfc3339(T1_STR).unwrap().into();
        BunyanLogRecord {
            time: t1.into(),
            name: "n1".to_string(),
            hostname: "h1".to_string(),
            pid: 1,
            msg: "msg1".to_string(),
            v: 0,
        }
    }

    /*
     * Tests various cases where verify_bunyan_records() should not panic.
     */
    #[test]
    fn test_bunyan_easy_cases() {
        let t1: DateTime<Utc> =
            DateTime::parse_from_rfc3339(T1_STR).unwrap().into();
        let r1 = make_dummy_record();
        let r2 = BunyanLogRecord {
            time: t1.into(),
            name: "n1".to_string(),
            hostname: "h2".to_string(),
            pid: 1,
            msg: "msg2".to_string(),
            v: 1,
        };

        /* Test case: nothing to check. */
        let records: Vec<&BunyanLogRecord> = vec![&r1];
        let iter = records.iter().map(|x| *x);
        verify_bunyan_records(iter, &BunyanLogRecordSpec {
            name: None,
            hostname: None,
            pid: None,
            v: None,
        });

        /* Test case: check name, no problem. */
        let records: Vec<&BunyanLogRecord> = vec![&r1];
        let iter = records.iter().map(|x| *x);
        verify_bunyan_records(iter, &BunyanLogRecordSpec {
            name: Some("n1".to_string()),
            hostname: None,
            pid: None,
            v: None,
        });

        /* Test case: check hostname, no problem. */
        let records: Vec<&BunyanLogRecord> = vec![&r1];
        let iter = records.iter().map(|x| *x);
        verify_bunyan_records(iter, &BunyanLogRecordSpec {
            name: None,
            hostname: Some("h1".to_string()),
            pid: None,
            v: None,
        });

        /* Test case: check pid, no problem. */
        let records: Vec<&BunyanLogRecord> = vec![&r1];
        let iter = records.iter().map(|x| *x);
        verify_bunyan_records(iter, &BunyanLogRecordSpec {
            name: None,
            hostname: None,
            pid: Some(1),
            v: None,
        });

        /* Test case: check hostname, no problem. */
        let records: Vec<&BunyanLogRecord> = vec![&r1];
        let iter = records.iter().map(|x| *x);
        verify_bunyan_records(iter, &BunyanLogRecordSpec {
            name: None,
            hostname: None,
            pid: None,
            v: Some(0),
        });

        /* Test case: check all, no problem. */
        let records: Vec<&BunyanLogRecord> = vec![&r1];
        let iter = records.iter().map(|x| *x);
        verify_bunyan_records(iter, &BunyanLogRecordSpec {
            name: Some("n1".to_string()),
            hostname: Some("h1".to_string()),
            pid: Some(1),
            v: Some(0),
        });

        /* Test case: check multiple records, no problem. */
        let records: Vec<&BunyanLogRecord> = vec![&r1, &r2];
        let iter = records.iter().map(|x| *x);
        verify_bunyan_records(iter, &BunyanLogRecordSpec {
            name: Some("n1".to_string()),
            hostname: None,
            pid: Some(1),
            v: None,
        });
    }

    /*
     * Test cases exercising violations of each of the fields.
     */

    #[test]
    #[should_panic(expected = "assertion failed")]
    fn test_bunyan_bad_name() {
        let r1 = make_dummy_record();
        let records: Vec<&BunyanLogRecord> = vec![&r1];
        let iter = records.iter().map(|x| *x);
        verify_bunyan_records(iter, &BunyanLogRecordSpec {
            name: Some("n2".to_string()),
            hostname: None,
            pid: None,
            v: None,
        });
    }

    #[test]
    #[should_panic(expected = "assertion failed")]
    fn test_bunyan_bad_hostname() {
        let r1 = make_dummy_record();
        let records: Vec<&BunyanLogRecord> = vec![&r1];
        let iter = records.iter().map(|x| *x);
        verify_bunyan_records(iter, &BunyanLogRecordSpec {
            name: None,
            hostname: Some("h2".to_string()),
            pid: None,
            v: None,
        });
    }

    #[test]
    #[should_panic(expected = "assertion failed")]
    fn test_bunyan_bad_pid() {
        let r1 = make_dummy_record();
        let records: Vec<&BunyanLogRecord> = vec![&r1];
        let iter = records.iter().map(|x| *x);
        verify_bunyan_records(iter, &BunyanLogRecordSpec {
            name: None,
            hostname: None,
            pid: Some(2),
            v: None,
        });
    }

    #[test]
    #[should_panic(expected = "assertion failed")]
    fn test_bunyan_bad_v() {
        let r1 = make_dummy_record();
        let records: Vec<&BunyanLogRecord> = vec![&r1];
        let iter = records.iter().map(|x| *x);
        verify_bunyan_records(iter, &BunyanLogRecordSpec {
            name: None,
            hostname: None,
            pid: None,
            v: Some(1),
        });
    }

    /*
     * These cases exercise 0, 1, and 2 records with every valid combination
     * of lower and upper bounds.
     */
    #[test]
    fn test_bunyan_seq_easy_cases() {
        let t1: DateTime<Utc> =
            DateTime::parse_from_rfc3339(T1_STR).unwrap().into();
        let t2: DateTime<Utc> =
            DateTime::parse_from_rfc3339(T2_STR).unwrap().into();
        let v0: Vec<BunyanLogRecord> = vec![];
        let v1: Vec<BunyanLogRecord> = vec![BunyanLogRecord {
            time: t1.into(),
            name: "dummy_name".to_string(),
            hostname: "dummy_hostname".to_string(),
            pid: 123,
            msg: "dummy_msg".to_string(),
            v: 0,
        }];
        let v2: Vec<BunyanLogRecord> = vec![
            BunyanLogRecord {
                time: t1.into(),
                name: "dummy_name".to_string(),
                hostname: "dummy_hostname".to_string(),
                pid: 123,
                msg: "dummy_msg".to_string(),
                v: 0,
            },
            BunyanLogRecord {
                time: t2.into(),
                name: "dummy_name".to_string(),
                hostname: "dummy_hostname".to_string(),
                pid: 123,
                msg: "dummy_msg".to_string(),
                v: 0,
            },
        ];

        verify_bunyan_records_sequential(v0.iter(), None, None);
        verify_bunyan_records_sequential(v0.iter(), Some(&t1), None);
        verify_bunyan_records_sequential(v0.iter(), None, Some(&t1));
        verify_bunyan_records_sequential(v0.iter(), Some(&t1), Some(&t2));
        verify_bunyan_records_sequential(v1.iter(), None, None);
        verify_bunyan_records_sequential(v1.iter(), Some(&t1), None);
        verify_bunyan_records_sequential(v1.iter(), None, Some(&t2));
        verify_bunyan_records_sequential(v1.iter(), Some(&t1), Some(&t2));
        verify_bunyan_records_sequential(v2.iter(), None, None);
        verify_bunyan_records_sequential(v2.iter(), Some(&t1), None);
        verify_bunyan_records_sequential(v2.iter(), None, Some(&t2));
        verify_bunyan_records_sequential(v2.iter(), Some(&t1), Some(&t2));
    }

    /*
     * Test case: no records, but the bounds themselves violate the constraint.
     */
    #[test]
    #[should_panic(expected = "assertion failed: should_be_before")]
    fn test_bunyan_seq_bounds_bad() {
        let t1: DateTime<Utc> =
            DateTime::parse_from_rfc3339(T1_STR).unwrap().into();
        let t2: DateTime<Utc> =
            DateTime::parse_from_rfc3339(T2_STR).unwrap().into();
        let v0: Vec<BunyanLogRecord> = vec![];
        verify_bunyan_records_sequential(v0.iter(), Some(&t2), Some(&t1));
    }

    /*
     * Test case: sole record appears before early bound.
     */
    #[test]
    #[should_panic(expected = "assertion failed: should_be_before")]
    fn test_bunyan_seq_lower_violated() {
        let t1: DateTime<Utc> =
            DateTime::parse_from_rfc3339(T1_STR).unwrap().into();
        let t2: DateTime<Utc> =
            DateTime::parse_from_rfc3339(T2_STR).unwrap().into();
        let v1: Vec<BunyanLogRecord> = vec![BunyanLogRecord {
            time: t1.into(),
            name: "dummy_name".to_string(),
            hostname: "dummy_hostname".to_string(),
            pid: 123,
            msg: "dummy_msg".to_string(),
            v: 0,
        }];
        verify_bunyan_records_sequential(v1.iter(), Some(&t2), None);
    }

    /*
     * Test case: sole record appears after late bound.
     */
    #[test]
    #[should_panic(expected = "assertion failed: should_be_before")]
    fn test_bunyan_seq_upper_violated() {
        let t1: DateTime<Utc> =
            DateTime::parse_from_rfc3339(T1_STR).unwrap().into();
        let t2: DateTime<Utc> =
            DateTime::parse_from_rfc3339(T2_STR).unwrap().into();
        let v1: Vec<BunyanLogRecord> = vec![BunyanLogRecord {
            time: t2.into(),
            name: "dummy_name".to_string(),
            hostname: "dummy_hostname".to_string(),
            pid: 123,
            msg: "dummy_msg".to_string(),
            v: 0,
        }];
        verify_bunyan_records_sequential(v1.iter(), None, Some(&t1));
    }

    /*
     * Test case: two records out of order.
     */
    #[test]
    #[should_panic(expected = "assertion failed: should_be_before")]
    fn test_bunyan_seq_bad_order() {
        let t1: DateTime<Utc> =
            DateTime::parse_from_rfc3339(T1_STR).unwrap().into();
        let t2: DateTime<Utc> =
            DateTime::parse_from_rfc3339(T2_STR).unwrap().into();
        let v2: Vec<BunyanLogRecord> = vec![
            BunyanLogRecord {
                time: t2.into(),
                name: "dummy_name".to_string(),
                hostname: "dummy_hostname".to_string(),
                pid: 123,
                msg: "dummy_msg".to_string(),
                v: 0,
            },
            BunyanLogRecord {
                time: t1.into(),
                name: "dummy_name".to_string(),
                hostname: "dummy_hostname".to_string(),
                pid: 123,
                msg: "dummy_msg".to_string(),
                v: 0,
            },
        ];
        verify_bunyan_records_sequential(v2.iter(), None, None);
    }
}
