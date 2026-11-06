// Copyright 2026 Oxide Computer Company

//! Test cases for how the ways a request can end are reported, with
//! particular attention to panicking handlers.
//!
//! A request that reaches a handler can end in one of these ways:
//!
//! 1. the handler produces a response (success or error): reported as
//!    "request completed";
//! 2. the client disconnects first, in `HandlerTaskMode::CancelOnDisconnect`:
//!    the handler future is cancelled, reported as "request handling
//!    cancelled (client disconnected)";
//! 3. the client disconnects first, in `HandlerTaskMode::Detached`: reported
//!    as in (2), but the handler runs to completion, additionally reported
//!    as "request completed after handler was already cancelled";
//! 4. the handler -- including its extractors, which run on the handler's
//!    side of the boundary -- panics (in either task mode): the panic
//!    propagates and the connection is aborted with no response; reported
//!    as "request handler panicked" and NOT as a client disconnection;
//! 5. something in request handling *other than* the handler -- a
//!    user-provided version policy, dropshot's own routing -- panics: the
//!    connection is aborted as in (4), but reported as "request handling
//!    panicked (outside the handler)", distinguishing a broken handler
//!    from a bug elsewhere;
//! 6. the server is torn down with the request in flight -- including
//!    teardown caused by a panic elsewhere in the process (e.g. a failing
//!    test in a consumer's test suite that drops its runtime while
//!    unwinding).  This is a cancellation, and it must be reported as in
//!    (2), NOT as a panic, no matter why teardown happened.
//!
//! These tests pin all six, so that changes to how panics are detected can
//! demonstrate they distinguish the cases correctly.
//!
//! The contract, in short: "request handling cancelled (client
//! disconnected)" means the client went away; the two panic reports mean
//! the server software is broken, and name which side of the
//! handler boundary is at fault.  None of the three are ever conflated.
//! `panic_reported_as_panic` (a handler panic),
//! `extractor_panic_reported_as_handler_panic` (an extractor panic), and
//! `test_panic_outside_handler_reported_as_panic` (a version policy panic)
//! demonstrate the boundary side by side; in every case the
//! "panic_message" property identifies the broken code.

use camino::{Utf8Path, Utf8PathBuf};
use dropshot::test_util::{
    BunyanLogRecord, ClientTestContext, log_file_for_test, read_bunyan_log,
};
use dropshot::{
    ApiDescription, Body, ConfigDropshot, ConfigLogging, ConfigLoggingIfExists,
    ConfigLoggingLevel, DynamicVersionPolicy, HandlerTaskMode, HttpError,
    HttpResponseOk, Query, RequestContext, ServerBuilder, VersionPolicy,
    endpoint,
};
use http::{Method, StatusCode};
use hyper::Request;
use semver::Version;
use slog::Logger;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// How long the `/slow` handler takes.  Long enough that a client can
/// reliably disconnect (or tear the server down) mid-handler; short enough
/// to keep the tests quick.
const SLOW_HANDLER_DURATION: Duration = Duration::from_millis(1000);

/// How long to wait for an expected event before declaring failure.
const POLL_TIMEOUT: Duration = Duration::from_secs(15);

/// State shared with the `/slow` handler so tests can observe its progress.
#[derive(Default)]
struct TestState {
    slow_started: AtomicBool,
    slow_completed: AtomicBool,
}

fn api() -> ApiDescription<Arc<TestState>> {
    let mut api = ApiDescription::new();
    api.register(handler_panic).unwrap();
    api.register(handler_panic_extractor).unwrap();
    api.register(handler_ok).unwrap();
    api.register(handler_slow).unwrap();
    api
}

#[endpoint {
    method = GET,
    path = "/panic",
}]
async fn handler_panic(
    _rqctx: RequestContext<Arc<TestState>>,
) -> Result<HttpResponseOk<u64>, HttpError> {
    panic!("oh no, a panic!");
}

/// A query type whose deserialization panics: a stand-in for a bug in an
/// extractor.  Extractors run on the handler's side of the reporting
/// boundary (inside `handle_request`), so this panic must be reported as a
/// handler panic.
#[derive(schemars::JsonSchema)]
struct PanickyQuery {
    #[allow(dead_code)]
    x: String,
}

impl<'de> serde::Deserialize<'de> for PanickyQuery {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        panic!("deliberate panic in extractor");
    }
}

#[endpoint {
    method = GET,
    path = "/panic-extractor",
}]
async fn handler_panic_extractor(
    _rqctx: RequestContext<Arc<TestState>>,
    _query: Query<PanickyQuery>,
) -> Result<HttpResponseOk<u64>, HttpError> {
    unreachable!("the extractor panics before the handler body runs");
}

#[endpoint {
    method = GET,
    path = "/ok",
}]
async fn handler_ok(
    _rqctx: RequestContext<Arc<TestState>>,
) -> Result<HttpResponseOk<u64>, HttpError> {
    Ok(HttpResponseOk(1))
}

#[endpoint {
    method = GET,
    path = "/slow",
}]
async fn handler_slow(
    rqctx: RequestContext<Arc<TestState>>,
) -> Result<HttpResponseOk<u64>, HttpError> {
    let state = rqctx.context();
    state.slow_started.store(true, Ordering::SeqCst);
    tokio::time::sleep(SLOW_HANDLER_DURATION).await;
    state.slow_completed.store(true, Ordering::SeqCst);
    Ok(HttpResponseOk(2))
}

/// Creates a file-based logger so that tests can verify what was reported.
/// Debug level, so that debug-level breadcrumbs (like Detached mode's
/// "handler panicked; relaying panic") are observable too.
fn file_logger(test_name: &str) -> (Utf8PathBuf, slog::Logger) {
    let log_path = log_file_for_test(test_name);
    let config_logging = ConfigLogging::File {
        level: ConfigLoggingLevel::Debug,
        path: log_path.clone(),
        if_exists: ConfigLoggingIfExists::Fail,
    };
    let log = config_logging.to_logger(test_name).unwrap();
    (log_path, log)
}

fn log_has(records: &[BunyanLogRecord], msg: &str) -> bool {
    records.iter().any(|r| r.msg == msg)
}

/// The report for a panic in the handler (or its extractors).
const HANDLER_PANIC: &str = "request handler panicked";
/// The report for a panic anywhere else in request handling.
const OTHER_PANIC: &str = "request handling panicked (outside the handler)";
/// The report for a client disconnect (or server teardown) mid-request.
const DISCONNECT: &str = "request handling cancelled (client disconnected)";

/// Returns the "panic_message" property of the log record whose message is
/// `msg`, if any.  (`BunyanLogRecord` does not carry custom properties, so
/// this parses the raw log.)
fn logged_panic_message(log_path: &Utf8Path, msg: &str) -> Option<String> {
    std::fs::read_to_string(log_path)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|record| record["msg"] == msg)
        .and_then(|record| record["panic_message"].as_str().map(str::to_string))
}

/// Reads the bunyan log until `pred` is satisfied or `POLL_TIMEOUT` elapses
/// (the slog-async drain writes asynchronously), returning the records last
/// read.  Only called after the last `Logger` clone has been dropped, which
/// joins the drain thread; the poll loop is belt-and-braces.
fn wait_for_log(
    log_path: &Utf8Path,
    pred: impl Fn(&[BunyanLogRecord]) -> bool,
) -> Vec<BunyanLogRecord> {
    let deadline = Instant::now() + POLL_TIMEOUT;
    loop {
        let records = read_bunyan_log(log_path.as_std_path());
        if pred(&records) || Instant::now() >= deadline {
            return records;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Polls until `flag` becomes true, panicking with `failure` after
/// `POLL_TIMEOUT`.
async fn wait_for_flag(flag: &AtomicBool, failure: &str) {
    let deadline = Instant::now() + POLL_TIMEOUT;
    while !flag.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "{}", failure);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Issues a GET for `path` over a raw TCP connection, without waiting for a
/// response, returning the connection.  Used where an HTTP client's
/// lifecycle would get in the way: to observe an aborted connection, or to
/// disconnect mid-request by dropping the stream.
async fn raw_get(
    addr: std::net::SocketAddr,
    path: &str,
) -> tokio::net::TcpStream {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(
            format!("GET {} HTTP/1.1\r\nhost: test\r\n\r\n", path).as_bytes(),
        )
        .await
        .unwrap();
    stream
}

/// Case 4: a panicking handler aborts the connection with no response; the
/// panic is reported as a panic, not as a client disconnect, and the server
/// remains usable.
async fn panic_reported_as_panic(test_name: &str, task_mode: HandlerTaskMode) {
    let (log_path, log) = file_logger(test_name);
    let server = ServerBuilder::new(api(), Default::default(), log.clone())
        .config(ConfigDropshot {
            default_handler_task_mode: task_mode,
            ..Default::default()
        })
        .start()
        .unwrap();

    // Depending on platform and timing, the read may end with a clean EOF
    // or a connection-reset error.  What matters is that no response bytes
    // were written.
    let mut stream = raw_get(server.local_addr(), "/panic").await;
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await;
    assert!(
        buf.is_empty(),
        "expected no response bytes, got: {:?}",
        String::from_utf8_lossy(&buf)
    );

    let client = ClientTestContext::new(server.local_addr(), log.clone());
    client
        .make_request_no_body(Method::GET, "/ok", StatusCode::OK)
        .await
        .expect("server should still be usable after a handler panic");

    server.close().await.unwrap();
    drop(client);
    drop(log);
    let records = wait_for_log(&log_path, |r| log_has(r, HANDLER_PANIC));
    assert!(
        log_has(&records, HANDLER_PANIC),
        "expected the panic to be reported as a handler panic"
    );
    assert!(
        !log_has(&records, OTHER_PANIC),
        "a handler panic must be attributed to the handler, not reported \
         as a panic outside it"
    );
    assert!(
        !records.iter().any(|r| r.msg.contains("client disconnected")),
        "a handler panic must not be reported as a client disconnect"
    );
    // Case 1: the follow-up request to `/ok` is reported as completed.
    assert!(
        log_has(&records, "request completed"),
        "expected the follow-up request to be reported as completed"
    );
    // Detached mode relays the panic out of the handler task; check for its
    // breadcrumb so a regression can't silently reroute one mode's panics
    // through the other's path.
    match task_mode {
        HandlerTaskMode::Detached => assert!(
            log_has(&records, "handler panicked; relaying panic"),
            "expected the panic to be relayed from the detached handler task"
        ),
        HandlerTaskMode::CancelOnDisconnect => assert!(
            !log_has(&records, "handler panicked; relaying panic"),
            "in CancelOnDisconnect mode the panic is caught around the \
             handler call itself, not relayed from a detached task"
        ),
    }
    // The report identifies the broken code: the handler's own panic
    // message is carried in the "panic_message" property.
    let panic_message = logged_panic_message(&log_path, HANDLER_PANIC)
        .expect("expected a panic_message property on the panic report");
    assert!(
        panic_message.contains("oh no, a panic!"),
        "expected the handler's panic message, got: {:?}",
        panic_message
    );
    std::fs::remove_file(&log_path).unwrap();
}

/// A version policy that panics while extracting the version: a stand-in
/// for a bug anywhere in request handling outside the handler itself
/// (routing, extractors, and the like).
#[derive(Debug)]
struct PanickyVersionPolicy;

impl DynamicVersionPolicy for PanickyVersionPolicy {
    fn request_extract_version(
        &self,
        _request: &Request<Body>,
        _log: &Logger,
    ) -> Result<Version, HttpError> {
        panic!("deliberate panic in version policy");
    }
}

/// Case 5: a panic in request handling outside the handler aborts the
/// connection just like a handler panic, but is reported distinctly, as
/// "request handling panicked (outside the handler)" -- so a bug in (say)
/// a version policy is not blamed on the endpoint handler.  The version
/// policy runs before the handler is even looked up, so this is
/// independent of the handler task mode.
#[tokio::test]
async fn test_panic_outside_handler_reported_as_panic() {
    let (log_path, log) =
        file_logger("panic_outside_handler_reported_as_panic");
    let server = ServerBuilder::new(api(), Default::default(), log)
        .version_policy(VersionPolicy::Dynamic(Box::new(PanickyVersionPolicy)))
        .start()
        .unwrap();

    let mut stream = raw_get(server.local_addr(), "/ok").await;
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await;
    assert!(
        buf.is_empty(),
        "expected no response bytes, got: {:?}",
        String::from_utf8_lossy(&buf)
    );

    server.close().await.unwrap();
    let records = wait_for_log(&log_path, |r| log_has(r, OTHER_PANIC));
    assert!(
        log_has(&records, OTHER_PANIC),
        "expected the panic to be reported as a panic outside the handler"
    );
    assert!(
        !log_has(&records, HANDLER_PANIC),
        "a panic outside the handler must not be blamed on the handler"
    );
    assert!(
        !records.iter().any(|r| r.msg.contains("client disconnected")),
        "a panic must not be reported as a client disconnect"
    );
    let panic_message = logged_panic_message(&log_path, OTHER_PANIC)
        .expect("expected a panic_message property on the panic report");
    assert!(
        panic_message.contains("version policy"),
        "expected the version policy's panic message, got: {:?}",
        panic_message
    );
    std::fs::remove_file(&log_path).unwrap();
}

#[tokio::test]
async fn test_panic_reported_as_panic_detached() {
    panic_reported_as_panic(
        "panic_reported_as_panic_detached",
        HandlerTaskMode::Detached,
    )
    .await;
}

#[tokio::test]
async fn test_panic_reported_as_panic_cancel_on_disconnect() {
    panic_reported_as_panic(
        "panic_reported_as_panic_cancel_on_disconnect",
        HandlerTaskMode::CancelOnDisconnect,
    )
    .await;
}

/// Case 4, via an extractor: extractors run on the handler's side of the
/// reporting boundary (inside the handler dispatch, in both task modes),
/// so a panicking extractor is reported as a handler panic.
async fn extractor_panic_reported_as_handler_panic(
    test_name: &str,
    task_mode: HandlerTaskMode,
) {
    let (log_path, log) = file_logger(test_name);
    let server = ServerBuilder::new(api(), Default::default(), log)
        .config(ConfigDropshot {
            default_handler_task_mode: task_mode,
            ..Default::default()
        })
        .start()
        .unwrap();

    let mut stream = raw_get(server.local_addr(), "/panic-extractor").await;
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await;
    assert!(
        buf.is_empty(),
        "expected no response bytes, got: {:?}",
        String::from_utf8_lossy(&buf)
    );

    server.close().await.unwrap();
    let records = wait_for_log(&log_path, |r| log_has(r, HANDLER_PANIC));
    assert!(
        log_has(&records, HANDLER_PANIC),
        "expected an extractor panic to be reported as a handler panic"
    );
    assert!(
        !log_has(&records, OTHER_PANIC),
        "an extractor panic belongs to the handler, not to the rest of \
         request handling"
    );
    assert!(
        !records.iter().any(|r| r.msg.contains("client disconnected")),
        "an extractor panic must not be reported as a client disconnect"
    );
    let panic_message = logged_panic_message(&log_path, HANDLER_PANIC)
        .expect("expected a panic_message property on the panic report");
    assert!(
        panic_message.contains("deliberate panic in extractor"),
        "expected the extractor's panic message, got: {:?}",
        panic_message
    );
    std::fs::remove_file(&log_path).unwrap();
}

#[tokio::test]
async fn test_extractor_panic_reported_as_handler_panic_detached() {
    extractor_panic_reported_as_handler_panic(
        "extractor_panic_reported_as_handler_panic_detached",
        HandlerTaskMode::Detached,
    )
    .await;
}

#[tokio::test]
async fn test_extractor_panic_reported_as_handler_panic_cancel_on_disconnect() {
    extractor_panic_reported_as_handler_panic(
        "extractor_panic_reported_as_handler_panic_cancel_on_disconnect",
        HandlerTaskMode::CancelOnDisconnect,
    )
    .await;
}

/// Cases 2 and 3: a mid-handler client disconnect is reported as a client
/// disconnect (never as a panic); in `Detached` mode the handler
/// additionally runs to completion, and in `CancelOnDisconnect` mode it is
/// cancelled.
async fn disconnect_reported_as_disconnect(
    test_name: &str,
    task_mode: HandlerTaskMode,
) {
    let (log_path, log) = file_logger(test_name);
    let state = Arc::new(TestState::default());
    let server = ServerBuilder::new(api(), state.clone(), log.clone())
        .config(ConfigDropshot {
            default_handler_task_mode: task_mode,
            ..Default::default()
        })
        .start()
        .unwrap();

    // Connect, get the handler running, then disconnect.
    let stream = raw_get(server.local_addr(), "/slow").await;
    wait_for_flag(&state.slow_started, "handler never started").await;
    drop(stream);

    // Observe the handler's fate through the shared state; the log is
    // examined only after teardown.
    match task_mode {
        HandlerTaskMode::Detached => {
            wait_for_flag(
                &state.slow_completed,
                "detached handler never completed",
            )
            .await;
        }
        HandlerTaskMode::CancelOnDisconnect => {
            // Give the handler its full duration (and margin) to show that
            // it never completes.
            tokio::time::sleep(SLOW_HANDLER_DURATION * 2).await;
            assert!(
                !state.slow_completed.load(Ordering::SeqCst),
                "handler should have been cancelled by the disconnect"
            );
        }
    }

    server.close().await.unwrap();
    drop(log);
    let records = wait_for_log(&log_path, |r| log_has(r, DISCONNECT));
    let messages = records.iter().map(|r| &r.msg).collect::<Vec<_>>();
    assert!(
        log_has(&records, DISCONNECT),
        "expected a client disconnect to be reported; log: {:?}",
        messages
    );
    if task_mode == HandlerTaskMode::Detached {
        assert!(
            log_has(
                &records,
                "request completed after handler was already cancelled",
            ),
            "expected the detached handler's completion to be reported; \
             log: {:?}",
            messages
        );
    }
    assert!(
        !log_has(&records, HANDLER_PANIC) && !log_has(&records, OTHER_PANIC),
        "a client disconnect must not be reported as a panic; log: {:?}",
        messages
    );
    std::fs::remove_file(&log_path).unwrap();
}

#[tokio::test]
async fn test_disconnect_reported_as_disconnect_detached() {
    disconnect_reported_as_disconnect(
        "disconnect_reported_as_disconnect_detached",
        HandlerTaskMode::Detached,
    )
    .await;
}

#[tokio::test]
async fn test_disconnect_reported_as_disconnect_cancel_on_disconnect() {
    disconnect_reported_as_disconnect(
        "disconnect_reported_as_disconnect_cancel_on_disconnect",
        HandlerTaskMode::CancelOnDisconnect,
    )
    .await;
}

/// Case 6: tearing the server down while a request is in flight is a
/// cancellation, and must be reported as one even when the teardown is
/// caused by a panic elsewhere in the process.  Concretely: a consumer's
/// test creates a Dropshot server, makes a request, and then fails
/// (panics), dropping its runtime -- and with it the server and the
/// in-flight request -- while the thread is unwinding.  The handler did
/// not panic, and must not be reported as having panicked.
fn teardown_during_external_panic(test_name: &str, task_mode: HandlerTaskMode) {
    let (log_path, log) = file_logger(test_name);

    let state = Arc::new(TestState::default());
    let thread_state = state.clone();
    let thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let server = ServerBuilder::new(api(), thread_state.clone(), log)
                .config(ConfigDropshot {
                    default_handler_task_mode: task_mode,
                    ..Default::default()
                })
                .start()
                .unwrap();

            // Get a request in flight and its handler running.
            let _stream = raw_get(server.local_addr(), "/slow").await;
            wait_for_flag(&thread_state.slow_started, "handler never started")
                .await;

            // The consumer's own code fails.  Unwinding drops the runtime,
            // tearing down the server and the in-flight request.
            panic!("deliberate panic external to any handler");
        });
    });
    assert!(thread.join().is_err(), "the external panic should propagate");

    let records = wait_for_log(&log_path, |r| {
        log_has(r, DISCONNECT)
            || log_has(r, HANDLER_PANIC)
            || log_has(r, OTHER_PANIC)
    });
    assert!(
        !log_has(&records, HANDLER_PANIC) && !log_has(&records, OTHER_PANIC),
        "a request cancelled by server teardown must not be reported as a \
         panic (nothing in request handling panicked); log: {:?}",
        records.iter().map(|r| &r.msg).collect::<Vec<_>>()
    );
    assert!(
        log_has(&records, DISCONNECT),
        "expected the torn-down request to be reported as cancelled; \
         log: {:?}",
        records.iter().map(|r| &r.msg).collect::<Vec<_>>()
    );
    std::fs::remove_file(&log_path).unwrap();
}

#[test]
fn test_teardown_during_external_panic_detached() {
    teardown_during_external_panic(
        "teardown_during_external_panic_detached",
        HandlerTaskMode::Detached,
    );
}

#[test]
fn test_teardown_during_external_panic_cancel_on_disconnect() {
    teardown_during_external_panic(
        "teardown_during_external_panic_cancel_on_disconnect",
        HandlerTaskMode::CancelOnDisconnect,
    );
}
