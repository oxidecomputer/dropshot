// Copyright 2023 Oxide Computer Company

//! Tests for configuration file.

use dropshot::test_util::read_config;
use dropshot::{
    ConfigDropshot, ConfigTls, HandlerDisposition, HttpError, HttpResponseOk,
    RequestContext,
};
use dropshot::{HttpServer, HttpServerStarter};
use futures::StreamExt;
use slog::o;
use slog::Logger;
use std::mem;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::mpsc;

pub mod common;
use common::create_log_context;

// Bad values for "bind_address"

#[test]
fn test_config_bad_bind_address_port_too_small() {
    let error = read_config::<ConfigDropshot>(
        "bad_bind_address_port_too_small",
        "bind_address = \"127.0.0.1:-3\"",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.contains("invalid socket address syntax"));
}

#[test]
fn test_config_bad_bind_address_port_too_large() {
    let error = read_config::<ConfigDropshot>(
        "bad_bind_address_port_too_large",
        "bind_address = \"127.0.0.1:65536\"",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.contains("invalid socket address syntax"));
}

#[test]
fn test_config_bad_bind_address_garbage() {
    let error = read_config::<ConfigDropshot>(
        "bad_bind_address_garbage",
        "bind_address = \"garbage\"",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.contains("invalid socket address syntax"));
}

// Bad values for "request_body_max_bytes"

#[test]
fn test_config_bad_request_body_max_bytes_negative() {
    let error = read_config::<ConfigDropshot>(
        "bad_request_body_max_bytes_negative",
        "request_body_max_bytes = -1024",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.contains("invalid value: integer"));
}

#[test]
fn test_config_bad_request_body_max_bytes_too_large() {
    let error = read_config::<ConfigDropshot>(
        "bad_request_body_max_bytes_too_large",
        "request_body_max_bytes = 999999999999999999999999999999",
    )
    .unwrap_err()
    .to_string();
    println!("found error: {}", error);
    assert!(error.starts_with(""));
}

fn make_server<T: Send + Sync + 'static>(
    context: T,
    config: &ConfigDropshot,
    log: &Logger,
    tls: Option<ConfigTls>,
    api_description: Option<dropshot::ApiDescription<T>>,
) -> HttpServerStarter<T> {
    HttpServerStarter::new_with_tls(
        config,
        api_description.unwrap_or_else(dropshot::ApiDescription::new),
        context,
        log,
        tls,
    )
    .unwrap()
}

fn make_config(
    bind_ip_str: &str,
    bind_port: u16,
    default_handler_disposition: HandlerDisposition,
) -> ConfigDropshot {
    ConfigDropshot {
        bind_address: std::net::SocketAddr::new(
            std::net::IpAddr::from_str(bind_ip_str).unwrap(),
            bind_port,
        ),
        request_body_max_bytes: 1024,
        default_handler_disposition,
    }
}

// Trait for abstracting out test case specific properties from the common bind
// test logic
trait TestConfigBindServer<C>
where
    C: hyper::client::connect::Connect + Clone + Send + Sync + 'static,
{
    type Context: Send + Sync + 'static;

    fn make_client(&self) -> hyper::Client<C>;
    fn make_server(&self, bind_port: u16) -> HttpServer<Self::Context>;
    fn make_uri(&self, bind_port: u16) -> hyper::Uri;

    fn log(&self) -> &slog::Logger;
}

// Validate that we can create a server with the given configuration and that
// it binds to ports as expected.
async fn test_config_bind_server<C, T>(test_config: T, bind_port: u16)
where
    C: hyper::client::connect::Connect + Clone + Send + Sync + 'static,
    T: TestConfigBindServer<C>,
{
    let client = test_config.make_client();

    // Make sure there is not currently a server running on our expected
    // port so that when we subsequently create a server and run it we know
    // we're getting the one we configured.
    let error = client.get(test_config.make_uri(bind_port)).await.unwrap_err();
    assert!(error.is_connect());

    // Now start a server with our configuration and make the request again.
    // This should succeed in terms of making the request.  (The request
    // itself might fail with a 400-level or 500-level response code -- we
    // don't want to depend on too much from the ApiServer here -- but we
    // should have successfully made the request.)
    let server = test_config.make_server(bind_port);
    client.get(test_config.make_uri(bind_port)).await.unwrap();
    server.close().await.unwrap();

    // Make another request to make sure it fails now that we've shut down
    // the server.  We need a new client to make sure our client-side connection
    // starts from a clean slate.  (Otherwise, a race during shutdown could
    // cause us to successfully send a request packet, only to have the TCP
    // stack return with ECONNRESET, which gets in the way of what we're trying
    // to test here.)
    let client = test_config.make_client();
    let error = client.get(test_config.make_uri(bind_port)).await.unwrap_err();
    assert!(error.is_connect());

    // Start a server on another TCP port and make sure we can reach that
    // one (and NOT the one we just shut down).
    let server = test_config.make_server(bind_port + 1);
    client.get(test_config.make_uri(bind_port + 1)).await.unwrap();
    let error = client.get(test_config.make_uri(bind_port)).await.unwrap_err();
    assert!(error.is_connect());
    server.close().await.unwrap();

    let error = client.get(test_config.make_uri(bind_port)).await.unwrap_err();
    assert!(error.is_connect());
    let error =
        client.get(test_config.make_uri(bind_port + 1)).await.unwrap_err();
    assert!(error.is_connect());
}

#[tokio::test]
async fn test_config_bind_address_http() {
    let logctx = create_log_context("config_bind_address_http");
    let log = logctx.log.new(o!());

    struct ConfigBindServerHttp {
        log: slog::Logger,
    }
    impl TestConfigBindServer<hyper::client::connect::HttpConnector>
        for ConfigBindServerHttp
    {
        type Context = i32;

        fn make_client(
            &self,
        ) -> hyper::Client<hyper::client::connect::HttpConnector> {
            hyper::Client::new()
        }

        fn make_uri(&self, bind_port: u16) -> hyper::Uri {
            format!("http://localhost:{}/", bind_port).parse().unwrap()
        }
        fn make_server(&self, bind_port: u16) -> HttpServer<i32> {
            let config = make_config(
                "127.0.0.1",
                bind_port,
                HandlerDisposition::CancelOnDisconnect,
            );
            make_server(0, &config, &self.log, None, None).start()
        }

        fn log(&self) -> &slog::Logger {
            &self.log
        }
    }

    let test_config = ConfigBindServerHttp { log };
    let bind_port = 12215;
    test_config_bind_server::<_, ConfigBindServerHttp>(test_config, bind_port)
        .await;

    logctx.cleanup_successful();
}

#[tokio::test]
async fn test_config_bind_address_https() {
    struct ConfigBindServerHttps {
        log: slog::Logger,
        certs: Vec<rustls::Certificate>,
        cert_file: NamedTempFile,
        key_file: NamedTempFile,
    }

    impl
        TestConfigBindServer<
            hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>,
        > for ConfigBindServerHttps
    {
        type Context = i32;

        fn make_client(
            &self,
        ) -> hyper::Client<
            hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>,
        > {
            // Configure TLS to trust the self-signed cert
            let mut root_store = rustls::RootCertStore { roots: vec![] };
            root_store
                .add(&self.certs[self.certs.len() - 1])
                .expect("adding root cert");

            let tls_config = rustls::ClientConfig::builder()
                .with_safe_defaults()
                .with_root_certificates(root_store)
                .with_no_client_auth();
            let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
                .with_tls_config(tls_config)
                .https_only()
                .enable_http1()
                .build();
            hyper::Client::builder().build(https_connector)
        }

        fn make_uri(&self, bind_port: u16) -> hyper::Uri {
            format!("https://localhost:{}/", bind_port).parse().unwrap()
        }

        fn make_server(&self, bind_port: u16) -> HttpServer<i32> {
            let tls = Some(ConfigTls::AsFile {
                cert_file: self.cert_file.path().to_path_buf(),
                key_file: self.key_file.path().to_path_buf(),
            });
            let config = make_config(
                "127.0.0.1",
                bind_port,
                HandlerDisposition::CancelOnDisconnect,
            );
            make_server(0, &config, &self.log, tls, None).start()
        }

        fn log(&self) -> &Logger {
            &self.log
        }
    }

    let logctx = create_log_context("config_bind_address_https");
    let log = logctx.log.new(o!());

    // Generate key for the server
    let (certs, key) = common::generate_tls_key();
    let (cert_file, key_file) = common::tls_key_to_file(&certs, &key);
    let test_config = ConfigBindServerHttps { log, certs, cert_file, key_file };

    // This must be different than the bind_port used in the http test.
    let bind_port = 12217;
    test_config_bind_server::<_, ConfigBindServerHttps>(test_config, bind_port)
        .await;

    logctx.cleanup_successful();
}

#[tokio::test]
async fn test_config_bind_address_https_buffer() {
    struct ConfigBindServerHttps {
        log: slog::Logger,
        certs: Vec<rustls::Certificate>,
        serialized_certs: Vec<u8>,
        serialized_key: Vec<u8>,
    }

    impl
        TestConfigBindServer<
            hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>,
        > for ConfigBindServerHttps
    {
        type Context = i32;

        fn make_client(
            &self,
        ) -> hyper::Client<
            hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>,
        > {
            // Configure TLS to trust the self-signed cert
            let mut root_store = rustls::RootCertStore { roots: vec![] };
            root_store
                .add(&self.certs[self.certs.len() - 1])
                .expect("adding root cert");

            let tls_config = rustls::ClientConfig::builder()
                .with_safe_defaults()
                .with_root_certificates(root_store)
                .with_no_client_auth();
            let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
                .with_tls_config(tls_config)
                .https_only()
                .enable_http1()
                .build();
            hyper::Client::builder().build(https_connector)
        }

        fn make_uri(&self, bind_port: u16) -> hyper::Uri {
            format!("https://localhost:{}/", bind_port).parse().unwrap()
        }

        fn make_server(&self, bind_port: u16) -> HttpServer<i32> {
            let tls = Some(ConfigTls::AsBytes {
                certs: self.serialized_certs.clone(),
                key: self.serialized_key.clone(),
            });
            let config = make_config(
                "127.0.0.1",
                bind_port,
                HandlerDisposition::CancelOnDisconnect,
            );
            make_server(0, &config, &self.log, tls, None).start()
        }

        fn log(&self) -> &Logger {
            &self.log
        }
    }

    let logctx = create_log_context("config_bind_address_https_buffer");
    let log = logctx.log.new(o!());

    // Generate key for the server
    let (certs, key) = common::generate_tls_key();
    let (serialized_certs, serialized_key) =
        common::tls_key_to_buffer(&certs, &key);
    let test_config =
        ConfigBindServerHttps { log, certs, serialized_certs, serialized_key };

    // This must be different than the bind_port used in the http test.
    let bind_port = 12219;
    test_config_bind_server::<_, ConfigBindServerHttps>(test_config, bind_port)
        .await;

    logctx.cleanup_successful();
}

#[tokio::test]
async fn test_config_handler_disposition_cancel() {
    let logctx = create_log_context("config_handler_disposition_cancel");
    let log = logctx.log.new(o!());

    struct DropCounter {
        count: Arc<AtomicU64>,
        tx: mpsc::UnboundedSender<()>,
    }
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.count.fetch_add(1, Ordering::SeqCst);
            self.tx.send(()).unwrap();
        }
    }

    struct ServerContext {
        // Count of how many times `increment_on_drop` has started and then been
        // dropped.
        drop_count: Arc<AtomicU64>,

        // Channel on which `DropCounter`'s `drop()` can report it ran.
        drop_ran_tx: mpsc::UnboundedSender<()>,

        // Channel on which we send a message once `increment_on_drop` is
        // running.
        increment_on_drop_tx: mpsc::UnboundedSender<()>,

        // `increment_on_drop` blocks until it receives a message on this
        // channel; this allows us to control its cancellation without timers.
        block_until_rx: async_channel::Receiver<()>,
    }

    #[dropshot::endpoint {
        method = GET,
        path = "/",
    }]
    async fn increment_on_drop(
        rqctx: RequestContext<ServerContext>,
    ) -> Result<HttpResponseOk<u64>, HttpError> {
        let ctx = rqctx.context();

        // Create a drop handler that will increment our count if we're dropped.
        let drop_counter = DropCounter {
            count: Arc::clone(&ctx.drop_count),
            tx: ctx.drop_ran_tx.clone(),
        };

        // Notify test that we're running.
        ctx.increment_on_drop_tx.send(()).unwrap();

        // Block until the test tells us to return.
        () = ctx.block_until_rx.recv().await.unwrap();

        // We weren't cancelled: mem::forget() drop_counter so its drop impl
        // doesn't run. This leaks a reference to our drop_count, but we're in a
        // unit test so won't worry about it.
        mem::forget(drop_counter);

        Ok(HttpResponseOk(ctx.drop_count.load(Ordering::SeqCst)))
    }

    struct ConfigBindServerHttp {
        increment_on_drop_tx: mpsc::UnboundedSender<()>,
        block_until_rx: async_channel::Receiver<()>,
        drop_count: Arc<AtomicU64>,
        drop_ran_tx: mpsc::UnboundedSender<()>,
        log: slog::Logger,
    }
    impl TestConfigBindServer<hyper::client::connect::HttpConnector>
        for ConfigBindServerHttp
    {
        type Context = ServerContext;

        fn make_client(
            &self,
        ) -> hyper::Client<hyper::client::connect::HttpConnector> {
            hyper::Client::new()
        }

        fn make_server(&self, bind_port: u16) -> HttpServer<ServerContext> {
            let context = ServerContext {
                drop_count: Arc::clone(&self.drop_count),
                drop_ran_tx: self.drop_ran_tx.clone(),
                increment_on_drop_tx: self.increment_on_drop_tx.clone(),
                block_until_rx: self.block_until_rx.clone(),
            };
            let config = make_config(
                "127.0.0.1",
                bind_port,
                HandlerDisposition::CancelOnDisconnect,
            );
            let mut api = dropshot::ApiDescription::new();
            api.register(increment_on_drop).unwrap();
            make_server(context, &config, &self.log, None, Some(api)).start()
        }
        fn make_uri(&self, bind_port: u16) -> hyper::Uri {
            format!("http://localhost:{}/", bind_port).parse().unwrap()
        }

        fn log(&self) -> &slog::Logger {
            &self.log
        }
    }

    let (increment_on_drop_tx, mut increment_on_drop_rx) =
        mpsc::unbounded_channel();
    let (drop_ran_tx, mut drop_ran_rx) = mpsc::unbounded_channel();
    let (block_until_tx, block_until_rx) = async_channel::unbounded();
    let drop_count = Arc::new(AtomicU64::new(0));

    let test_config = ConfigBindServerHttp {
        increment_on_drop_tx,
        block_until_rx,
        drop_count: Arc::clone(&drop_count),
        drop_ran_tx,
        log,
    };
    let bind_port = 12221;

    // Make sure there is not currently a server running on our expected
    // port so that when we subsequently create a server and run it we know
    // we're getting the one we configured.
    let client = test_config.make_client();
    let error = client.get(test_config.make_uri(bind_port)).await.unwrap_err();
    assert!(error.is_connect());

    // Now start a server with our configuration.
    let server = test_config.make_server(bind_port);

    // Spawn a task to hit our `increment_on_drop` endpoint.
    let client_task = {
        let client = test_config.make_client();
        let uri = test_config.make_uri(bind_port);
        tokio::spawn(async move { client.get(uri).await.unwrap() })
    };

    // Wait until the handler receives the request.
    () = increment_on_drop_rx.recv().await.unwrap();

    // Cancel the client.
    client_task.abort();
    let result = client_task.await.unwrap_err();
    assert!(result.is_cancelled());

    // Wait for `DropCounter::drop()` to run.
    () = drop_ran_rx.recv().await.unwrap();

    // `drop_count` should have gone up.
    assert_eq!(drop_count.load(Ordering::SeqCst), 1);

    // We should also be able to fetch the drop count with a client that is not
    // cancelled.
    let client_task = {
        let client = test_config.make_client();
        let uri = test_config.make_uri(bind_port);
        tokio::spawn(async move { client.get(uri).await.unwrap() })
    };

    // Wait until the handler receives the request.
    () = increment_on_drop_rx.recv().await.unwrap();

    // Release `increment_on_drop` to continue.
    block_until_tx.send(()).await.unwrap();

    // Wait for the response, which should still be 1.
    let body = client_task.await.unwrap().into_body().collect::<Vec<_>>().await;
    let mut data = Vec::new();
    for result in body {
        data.extend_from_slice(&result.unwrap());
    }
    assert_eq!(data, b"1");

    server.close().await.unwrap();

    logctx.cleanup_successful();
}

// The setup for this test is identical to
// `test_config_handler_disposition_cancel`, but we configure the server with a
// different disposition and have to drive it differently.
#[tokio::test]
async fn test_config_handler_disposition_detach() {
    let logctx = create_log_context("config_handler_disposition_detach");
    let log = logctx.log.new(o!());

    struct DropCounter {
        count: Arc<AtomicU64>,
        tx: mpsc::UnboundedSender<()>,
    }
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.count.fetch_add(1, Ordering::SeqCst);
            self.tx.send(()).unwrap();
        }
    }

    struct ServerContext {
        // Count of how many times `increment_on_drop` has started and then been
        // dropped.
        drop_count: Arc<AtomicU64>,

        // Channel on which `DropCounter`'s `drop()` can report it ran.
        drop_ran_tx: mpsc::UnboundedSender<()>,

        // Channel on which we send a message once `increment_on_drop` is
        // running.
        increment_on_drop_tx: mpsc::UnboundedSender<()>,

        // `increment_on_drop` blocks until it receives a message on this
        // channel; this allows us to control its cancellation without timers.
        block_until_rx: async_channel::Receiver<()>,
    }

    #[dropshot::endpoint {
        method = GET,
        path = "/",
    }]
    async fn increment_on_drop(
        rqctx: RequestContext<ServerContext>,
    ) -> Result<HttpResponseOk<u64>, HttpError> {
        let ctx = rqctx.context();

        // Create a drop handler that will increment our count if we're dropped.
        let drop_counter = DropCounter {
            count: Arc::clone(&ctx.drop_count),
            tx: ctx.drop_ran_tx.clone(),
        };

        // Notify test that we're running.
        ctx.increment_on_drop_tx.send(()).unwrap();

        // Block until the test tells us to return.
        () = ctx.block_until_rx.recv().await.unwrap();

        // We weren't cancelled: mem::forget() drop_counter so its drop impl
        // doesn't run. This leaks a reference to our drop_count, but we're in a
        // unit test so won't worry about it.
        mem::forget(drop_counter);

        Ok(HttpResponseOk(ctx.drop_count.load(Ordering::SeqCst)))
    }

    struct ConfigBindServerHttp {
        increment_on_drop_tx: mpsc::UnboundedSender<()>,
        block_until_rx: async_channel::Receiver<()>,
        drop_count: Arc<AtomicU64>,
        drop_ran_tx: mpsc::UnboundedSender<()>,
        log: slog::Logger,
    }
    impl TestConfigBindServer<hyper::client::connect::HttpConnector>
        for ConfigBindServerHttp
    {
        type Context = ServerContext;

        fn make_client(
            &self,
        ) -> hyper::Client<hyper::client::connect::HttpConnector> {
            hyper::Client::new()
        }

        fn make_server(&self, bind_port: u16) -> HttpServer<ServerContext> {
            let context = ServerContext {
                drop_count: Arc::clone(&self.drop_count),
                drop_ran_tx: self.drop_ran_tx.clone(),
                increment_on_drop_tx: self.increment_on_drop_tx.clone(),
                block_until_rx: self.block_until_rx.clone(),
            };
            let config = make_config(
                "127.0.0.1",
                bind_port,
                HandlerDisposition::DetachFromClient,
            );
            let mut api = dropshot::ApiDescription::new();
            api.register(increment_on_drop).unwrap();
            make_server(context, &config, &self.log, None, Some(api)).start()
        }
        fn make_uri(&self, bind_port: u16) -> hyper::Uri {
            format!("http://localhost:{}/", bind_port).parse().unwrap()
        }

        fn log(&self) -> &slog::Logger {
            &self.log
        }
    }

    let (increment_on_drop_tx, mut increment_on_drop_rx) =
        mpsc::unbounded_channel();
    let (drop_ran_tx, _drop_ran_rx) = mpsc::unbounded_channel();
    let (block_until_tx, block_until_rx) = async_channel::unbounded();
    let drop_count = Arc::new(AtomicU64::new(0));

    let test_config = ConfigBindServerHttp {
        increment_on_drop_tx,
        block_until_rx,
        drop_count: Arc::clone(&drop_count),
        drop_ran_tx,
        log,
    };
    let bind_port = 12223;

    // Make sure there is not currently a server running on our expected
    // port so that when we subsequently create a server and run it we know
    // we're getting the one we configured.
    let client = test_config.make_client();
    let error = client.get(test_config.make_uri(bind_port)).await.unwrap_err();
    assert!(error.is_connect());

    // Now start a server with our configuration.
    let server = test_config.make_server(bind_port);

    // Spawn a task to hit our `increment_on_drop` endpoint.
    let client_task = {
        let client = test_config.make_client();
        let uri = test_config.make_uri(bind_port);
        tokio::spawn(async move { client.get(uri).await.unwrap() })
    };

    // Wait until the handler receives the request.
    () = increment_on_drop_rx.recv().await.unwrap();

    // Cancel the client.
    client_task.abort();
    let result = client_task.await.unwrap_err();
    assert!(result.is_cancelled());

    // The handler _should still be running_; send it a message to continue. If
    // the handler was cancelled, this send would hang.
    block_until_tx.send(()).await.unwrap();

    // Hit the endpoint again without cancelling; it should return a drop count
    // of 0.
    // cancelled.
    let client_task = {
        let client = test_config.make_client();
        let uri = test_config.make_uri(bind_port);
        tokio::spawn(async move { client.get(uri).await.unwrap() })
    };

    // Wait until the handler receives the request.
    () = increment_on_drop_rx.recv().await.unwrap();

    // Release `increment_on_drop` to continue.
    block_until_tx.send(()).await.unwrap();

    // Wait for the response.
    let body = client_task.await.unwrap().into_body().collect::<Vec<_>>().await;
    let mut data = Vec::new();
    for result in body {
        data.extend_from_slice(&result.unwrap());
    }
    assert_eq!(data, b"0");

    server.close().await.unwrap();

    logctx.cleanup_successful();
}
