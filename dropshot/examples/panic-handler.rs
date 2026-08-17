// Copyright 2026 Oxide Computer Company

//! Reproduces, interactively, how Dropshot reports a request handler that
//! panics: what appears in the log, what the request-done DTrace probe
//! reports, and what the client observes on the wire.
//!
//! Run it with the probes compiled in:
//!
//! ```text
//! cargo run --example panic-handler --features usdt-probes
//! ```
//!
//! then follow the printed instructions: optionally attach dtrace with the
//! printed one-liner, press Enter, and watch the sequence for a request to
//! `/panic`.  The server stays up afterward for further poking with curl.
//!
//! The log goes to stderr at debug level so that every message involved in
//! the sequence is visible, including debug-level breadcrumbs.

use dropshot::ApiDescription;
use dropshot::ConfigLogging;
use dropshot::ConfigLoggingLevel;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::ProbeRegistration;
use dropshot::RequestContext;
use dropshot::ServerBuilder;
use dropshot::endpoint;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

#[endpoint {
    method = GET,
    path = "/panic",
}]
async fn example_panic(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<u64>, HttpError> {
    panic!("oh no, a panic!");
}

#[endpoint {
    method = GET,
    path = "/ok",
}]
async fn example_ok(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<u64>, HttpError> {
    Ok(HttpResponseOk(1))
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let config_logging =
        ConfigLogging::StderrTerminal { level: ConfigLoggingLevel::Debug };
    let log = config_logging
        .to_logger("panic-handler-example")
        .map_err(|error| format!("failed to create logger: {}", error))?;

    let mut api = ApiDescription::new();
    api.register(example_panic).unwrap();
    api.register(example_ok).unwrap();

    let server = ServerBuilder::new(api, (), log)
        .start()
        .map_err(|error| format!("failed to start server: {}", error))?;
    let addr = server.local_addr();
    let pid = std::process::id();

    println!();
    println!("server:  http://{}", addr);
    println!("pid:     {}", pid);
    match server.probe_registration() {
        ProbeRegistration::Succeeded => println!("probes:  registered"),
        other => println!(
            "probes:  NOT registered ({:?}); \
             rebuild with --features usdt-probes",
            other
        ),
    }
    println!();
    println!("to watch the request-done probe, run (in another terminal):");
    println!();
    println!(
        "  dtrace -q -p {} -n 'dropshot$target:::request-done \
         {{ printf(\"%s\\n\", copyinstr(arg0)); }}'",
        pid
    );
    println!();
    println!(
        "press Enter to make a request to /panic \
         (attach dtrace first if you want the probe) ..."
    );
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    })
    .await
    .unwrap();

    // Make the request over a raw TCP connection so that exactly what the
    // client observes on the wire can be reported.
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .map_err(|error| format!("failed to connect: {}", error))?;
    stream
        .write_all(b"GET /panic HTTP/1.1\r\nhost: example\r\n\r\n")
        .await
        .map_err(|error| format!("failed to send request: {}", error))?;
    let mut buf = Vec::new();
    let result = stream.read_to_end(&mut buf).await;
    println!();
    println!("the client's view of GET /panic:");
    println!("  response bytes received: {}", buf.len());
    match result {
        Ok(_) => println!("  connection closed (clean EOF), no response"),
        Err(error) => println!("  connection aborted: {}", error),
    }

    println!();
    println!("server still running; things to try:");
    println!("  curl -v http://{}/panic", addr);
    println!("  curl -v http://{}/ok", addr);
    println!("^C to exit");
    server.await
}
