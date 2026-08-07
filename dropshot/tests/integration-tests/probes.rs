// Copyright 2026 Oxide Computer Company

//! Test that the USDT probes fire with the expected contents, verified by
//! running the real `dtrace(8)` against our own process.
//!
//! This can only work where dtrace exists and we have the privileges to use
//! it (e.g. as root, or in an illumos zone with the `dtrace_user` privilege
//! and friends).  Anywhere else -- including typical CI -- the test prints
//! why it is being skipped and passes vacuously.  Set the environment
//! variable `DROPSHOT_DTRACE_TEST=require` to turn those skips into failures
//! on hosts where the test is expected to run for real.
//!
//! A note on plumbing: dtrace's stdout is fully buffered when it's a pipe,
//! so record output cannot be streamed reliably.  Instead, once the requests
//! have been made (and dtrace's principal buffer given a switchrate's grace
//! to be consumed), dtrace is stopped with SIGTERM -- which it handles
//! gracefully, flushing everything on exit -- and its complete output is
//! collected then.  The `BEGIN { READY }` marker is the exception: it is
//! emitted during dtrace startup, where it does reach the pipe promptly, so
//! it serves to detect successful attachment (and, by its absence, missing
//! privileges).
//!
//! Other tests in this process may fire the same probes concurrently (under
//! plain `cargo test`; nextest runs each test in its own process), so
//! everything observed here is filtered down to this test's server by its
//! local address and request ids.

use dropshot::test_util::ClientTestContext;
use dropshot::{
    ApiDescription, HttpError, HttpResponseOk, ProbeRegistration,
    RequestContext, ServerBuilder, endpoint,
};
use http::{Method, StatusCode};
use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

fn api() -> ApiDescription<()> {
    let mut api = ApiDescription::new();
    api.register(probe_ok).unwrap();
    api.register(probe_fail).unwrap();
    api.register(probe_panic).unwrap();
    api
}

#[endpoint {
    method = GET,
    path = "/ok",
}]
async fn probe_ok(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<u64>, HttpError> {
    Ok(HttpResponseOk(1))
}

#[endpoint {
    method = GET,
    path = "/fail",
}]
async fn probe_fail(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<u64>, HttpError> {
    Err(HttpError::for_bad_request(None, "bad request".to_string()))
}

#[endpoint {
    method = GET,
    path = "/panic",
}]
async fn probe_panic(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<u64>, HttpError> {
    panic!("deliberate panic");
}

/// Skips the test (or fails it, if `DROPSHOT_DTRACE_TEST=require`).
fn skip(reason: &str) {
    if std::env::var("DROPSHOT_DTRACE_TEST").as_deref() == Ok("require") {
        panic!(
            "DROPSHOT_DTRACE_TEST=require but the dtrace test cannot \
             run: {}",
            reason
        );
    }
    eprintln!("skipping dtrace probe test: {}", reason);
}

const DTRACE_PROGRAM: &str = r#"
BEGIN { printf("READY\n"); }
dropshot$target:::request-start { printf("START %s\n", copyinstr(arg0)); }
dropshot$target:::request-done { printf("DONE %s\n", copyinstr(arg0)); }
"#;

/// Sifts dtrace output lines into this test's request-start records (keyed
/// by path) and request-done records (keyed by request id), keeping only
/// records for the server at `local_addr` (other tests in this process fire
/// the same probes).
///
/// usdt serializes each probe argument wrapped in the result of its
/// serialization, so the payload proper is nested under an "ok" key (and
/// "err" would carry a serialization error message).  Unwrap that envelope,
/// tolerating its absence in case usdt ever drops it.
fn sift_records(
    lines: &[String],
    local_addr: &str,
) -> (HashMap<String, serde_json::Value>, HashMap<String, serde_json::Value>) {
    let unwrap_envelope = |mut record: serde_json::Value| {
        assert!(
            record.get("err").is_none(),
            "probe argument failed to serialize: {}",
            record["err"]
        );
        match record.get_mut("ok") {
            Some(inner) => inner.take(),
            None => record,
        }
    };
    let mut starts = HashMap::new();
    let mut dones = HashMap::new();
    for line in lines {
        if let Some(json) = line.strip_prefix("START ") {
            let record = unwrap_envelope(serde_json::from_str(json).unwrap());
            if record["local_addr"] == local_addr {
                let path = record["path"].as_str().unwrap().to_string();
                starts.insert(path, record);
            }
        } else if let Some(json) = line.strip_prefix("DONE ") {
            let record = unwrap_envelope(serde_json::from_str(json).unwrap());
            if record["local_addr"] == local_addr {
                let id = record["id"].as_str().unwrap().to_string();
                dones.insert(id, record);
            }
        }
    }
    (starts, dones)
}

/// Checks the record sifting (in particular the "ok" envelope handling)
/// against verbatim output captured from a real dtrace run, so that this
/// much is verified even where dtrace itself is unavailable.
#[test]
fn test_sift_records() {
    let lines: Vec<String> = [
        "READY",
        r#"START {"ok":{"id":"7198f46a-2a62-492d-86e4-ce257615dfc7","local_addr":"127.0.0.1:34991","remote_addr":"127.0.0.1:50556","method":"GET","path":"/panic","query":null}}"#,
        r#"START {"ok":{"id":"6c98b16b-3e81-4a5b-8645-e2faeefcb738","local_addr":"127.0.0.1:34991","remote_addr":"127.0.0.1:64479","method":"GET","path":"/ok","query":null}}"#,
        r#"DONE {"ok":{"id":"6c98b16b-3e81-4a5b-8645-e2faeefcb738","local_addr":"127.0.0.1:34991","remote_addr":"127.0.0.1:64479","status_code":200,"message":""}}"#,
        r#"START {"ok":{"id":"00000000-0000-0000-0000-000000000000","local_addr":"127.0.0.1:9999","remote_addr":"127.0.0.1:64479","method":"GET","path":"/other-test","query":null}}"#,
        "",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let (starts, dones) = sift_records(&lines, "127.0.0.1:34991");
    assert_eq!(starts.len(), 2, "starts: {:?}", starts);
    assert_eq!(starts["/panic"]["method"], "GET");
    assert_eq!(starts["/ok"]["id"], "6c98b16b-3e81-4a5b-8645-e2faeefcb738");
    assert_eq!(dones.len(), 1);
    let done = &dones["6c98b16b-3e81-4a5b-8645-e2faeefcb738"];
    assert_eq!(done["status_code"], 200);
    assert_eq!(done["message"], "");
}

#[tokio::test]
async fn test_usdt_probes_fire_with_expected_contents() {
    let logctx = crate::common::create_log_context(
        "usdt_probes_fire_with_expected_contents",
    );
    let log = logctx.log.new(slog::o!());
    let server = ServerBuilder::new(api(), (), log.clone()).start().unwrap();
    match server.probe_registration() {
        ProbeRegistration::Succeeded => (),
        other => {
            skip(&format!("probe registration did not succeed: {:?}", other));
            server.close().await.unwrap();
            logctx.cleanup_successful();
            return;
        }
    }

    // Run dtrace against our own process.  If the binary is missing or we
    // lack the privileges to use it, skip.
    let child = tokio::process::Command::new("dtrace")
        .arg("-q")
        .arg("-p")
        .arg(std::process::id().to_string())
        .arg("-n")
        .arg(DTRACE_PROGRAM)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(e) => {
            skip(&format!("could not run dtrace: {}", e));
            server.close().await.unwrap();
            logctx.cleanup_successful();
            return;
        }
    };
    let dtrace_pid = child.id().expect("child has not been waited on");

    // Read dtrace's output continuously (so its stdout pipe can never fill
    // up), signalling when the BEGIN probe's READY marker is seen.
    let stdout = child.stdout.take().unwrap();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let reader = tokio::spawn(async move {
        let mut ready_tx = Some(ready_tx);
        let mut collected = Vec::new();
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.contains("READY") {
                if let Some(tx) = ready_tx.take() {
                    let _ = tx.send(());
                }
            }
            collected.push(line);
        }
        collected
    });

    // Wait for the READY marker.  If dtrace exits or times out instead,
    // it's (almost certainly) a privilege problem; report its stderr and
    // skip.
    let ready = tokio::time::timeout(Duration::from_secs(30), ready_rx).await;
    if !matches!(ready, Ok(Ok(()))) {
        let mut stderr = String::new();
        if let Some(mut err) = child.stderr.take() {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                err.read_to_string(&mut stderr),
            )
            .await;
        }
        let _ = child.kill().await;
        skip(&format!(
            "dtrace did not become ready (insufficient privileges?): {}",
            stderr.trim()
        ));
        server.close().await.unwrap();
        logctx.cleanup_successful();
        return;
    }

    // From here on, the environment has proven itself: failures are real
    // failures.
    //
    // Make three requests: one that panics, one success, and one error
    // response.
    let mut stream =
        tokio::net::TcpStream::connect(server.local_addr()).await.unwrap();
    stream
        .write_all(b"GET /panic HTTP/1.1\r\nhost: test\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    // The connection is aborted without a response; ignore how.
    let _ = stream.read_to_end(&mut buf).await;

    let client = ClientTestContext::new(server.local_addr(), log);
    client
        .make_request_no_body(Method::GET, "/ok", StatusCode::OK)
        .await
        .unwrap();
    client
        .make_request_error(Method::GET, "/fail", StatusCode::BAD_REQUEST)
        .await;

    // Give dtrace's principal buffer time to be consumed (the default
    // switchrate is one second), then stop dtrace gracefully with SIGTERM
    // so that it flushes its buffered output on exit.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let terminated = tokio::process::Command::new("kill")
        .arg(dtrace_pid.to_string())
        .status()
        .await;
    assert!(
        matches!(terminated, Ok(status) if status.success()),
        "failed to send SIGTERM to dtrace: {:?}",
        terminated
    );
    let waited =
        tokio::time::timeout(Duration::from_secs(15), child.wait()).await;
    if waited.is_err() {
        // dtrace did not exit on SIGTERM; kill it so the reader
        // terminates, and fail below if output is missing.
        let _ = child.kill().await;
    }
    let lines = reader.await.unwrap();

    // Sift the output for this test's records.
    let local_addr = server.local_addr().to_string();
    let (starts, dones) = sift_records(&lines, &local_addr);

    // All three requests must have fired request-start with the right
    // contents.
    for path in ["/panic", "/ok", "/fail"] {
        let start = starts.get(path).unwrap_or_else(|| {
            panic!(
                "no request-start probe for {}; dtrace output: {:?}",
                path, lines
            )
        });
        assert_eq!(start["method"], "GET");
        assert_eq!(start["local_addr"], local_addr.as_str());
        assert!(start["remote_addr"].as_str().is_some());
        assert!(start["id"].as_str().is_some());
    }

    // The success and the error must have fired request-done, correlated by
    // request id, with the status codes and messages the client saw.
    let ok_id = starts["/ok"]["id"].as_str().unwrap();
    let ok_done = &dones[ok_id];
    assert_eq!(ok_done["status_code"], 200);
    assert_eq!(ok_done["message"], "");

    let fail_id = starts["/fail"]["id"].as_str().unwrap();
    let fail_done = &dones[fail_id];
    assert_eq!(fail_done["status_code"], 400);
    assert_eq!(fail_done["message"], "bad request");

    // The panicked request fires request-done too, reporting status code 0
    // ("no response was received") and the panic message.
    let panic_id = starts["/panic"]["id"].as_str().unwrap();
    let panic_done = &dones[panic_id];
    assert_eq!(panic_done["status_code"], 0);
    assert_eq!(
        panic_done["message"],
        "request handling panicked: deliberate panic"
    );
    assert_eq!(dones.len(), 3, "unexpected extra request-done probes");

    server.close().await.unwrap();
    logctx.cleanup_successful();
}
