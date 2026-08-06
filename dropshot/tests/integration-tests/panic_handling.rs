// Copyright 2026 Oxide Computer Company

//! Test cases for how panicking HTTP handlers are reported.

use dropshot::test_util::{ClientTestContext, read_bunyan_log};
use dropshot::{
    ApiDescription, ConfigLogging, ConfigLoggingIfExists, ConfigLoggingLevel,
    HttpError, HttpResponseOk, RequestContext, ServerBuilder, endpoint,
};
use http::{Method, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn api() -> ApiDescription<()> {
    let mut api = ApiDescription::new();
    api.register(handler_panic).unwrap();
    api.register(handler_ok).unwrap();
    api
}

#[endpoint {
    method = GET,
    path = "/panic",
}]
async fn handler_panic(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<u64>, HttpError> {
    panic!("oh no, a panic!");
}

#[endpoint {
    method = GET,
    path = "/ok",
}]
async fn handler_ok(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<u64>, HttpError> {
    Ok(HttpResponseOk(1))
}

/// With `catch_handler_panics(true)`, a panicking handler produces a 500
/// response whose body does not include the panic message, and the server
/// remains usable afterwards.
async fn test_panic_returns_500(
    test_name: &str,
    default_handler_task_mode: dropshot::HandlerTaskMode,
) {
    let logctx = crate::common::create_log_context(test_name);
    let log = logctx.log.new(slog::o!());
    let server = ServerBuilder::new(api(), (), log.clone())
        .config(dropshot::ConfigDropshot {
            default_handler_task_mode,
            ..Default::default()
        })
        .catch_handler_panics(true)
        .start()
        .unwrap();
    let client = ClientTestContext::new(server.local_addr(), log);

    let error = client
        .make_request_error(
            Method::GET,
            "/panic",
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;
    assert_eq!(error.message, "Internal Server Error");

    client
        .make_request_no_body(Method::GET, "/ok", StatusCode::OK)
        .await
        .expect("server should still be usable after a handler panic");

    server.close().await.unwrap();
    logctx.cleanup_successful();
}

#[tokio::test]
async fn test_panic_returns_500_detached() {
    test_panic_returns_500(
        "panic_returns_500_detached",
        dropshot::HandlerTaskMode::Detached,
    )
    .await;
}

#[tokio::test]
async fn test_panic_returns_500_cancel_on_disconnect() {
    test_panic_returns_500(
        "panic_returns_500_cancel_on_disconnect",
        dropshot::HandlerTaskMode::CancelOnDisconnect,
    )
    .await;
}

/// By default, a panicking handler tears down the connection without sending
/// a response; the panic is reported as a panic (not as a client disconnect),
/// and the server remains usable for subsequent connections.
#[tokio::test]
async fn test_panic_reported_as_panic() {
    // Log to a file so that we can verify how the panic was reported.
    let log_path = std::env::temp_dir().join(format!(
        "test_panic_reported_as_panic.{}.log",
        std::process::id()
    ));
    let config_logging = ConfigLogging::File {
        level: ConfigLoggingLevel::Info,
        path: log_path.clone().try_into().unwrap(),
        if_exists: ConfigLoggingIfExists::Truncate,
    };
    let log = config_logging.to_logger("panic_reported_as_panic").unwrap();

    let server = ServerBuilder::new(api(), (), log.clone()).start().unwrap();

    // Speak raw HTTP so that we can observe the aborted connection instead of
    // an HTTP response.
    let mut stream =
        tokio::net::TcpStream::connect(server.local_addr()).await.unwrap();
    stream
        .write_all(b"GET /panic HTTP/1.1\r\nhost: test\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    match stream.read_to_end(&mut buf).await {
        // Clean EOF: the connection must have been closed with no response.
        Ok(_) => assert!(
            buf.is_empty(),
            "expected no response bytes, got: {:?}",
            String::from_utf8_lossy(&buf)
        ),
        // A connection reset is an equally acceptable way to observe the
        // aborted connection.
        Err(e) => {
            assert_eq!(e.kind(), std::io::ErrorKind::ConnectionReset)
        }
    }

    // The server remains usable on a fresh connection.
    let client = ClientTestContext::new(server.local_addr(), log.clone());
    client
        .make_request_no_body(Method::GET, "/ok", StatusCode::OK)
        .await
        .expect("server should still be usable after a handler panic");

    // Drop our references to the logger so the async drain flushes, then
    // check how the panic was logged: as a panic, not as a client disconnect.
    server.close().await.unwrap();
    drop(client);
    drop(log);
    let log_records = {
        let mut records = Vec::new();
        for _ in 0..100 {
            records = read_bunyan_log(&log_path);
            if records.iter().any(|r| r.msg == "request handling panicked") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        records
    };
    assert!(
        log_records.iter().any(|r| r.msg == "request handling panicked"),
        "expected the panic to be logged as a panic"
    );
    assert!(
        !log_records.iter().any(|r| r.msg.contains("client disconnected")),
        "a handler panic must not be reported as a client disconnect"
    );
    std::fs::remove_file(&log_path).unwrap();
}
