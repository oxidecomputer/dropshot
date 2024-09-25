// Copyright 2023 Oxide Computer Company

//! Tests for configuration file.

use dropshot::test_util::read_config;
use dropshot::{
    ConfigDropshot, ConfigTls, HandlerTaskMode, HttpError, HttpResponseOk,
    RequestContext,
};
use dropshot::{HttpServer, HttpServerStarter};
use slog::o;
use slog::Logger;
use std::str::FromStr;
use std::sync::atomic::{AtomicU16, Ordering};
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
    default_handler_task_mode: HandlerTaskMode,
) -> ConfigDropshot {
    ConfigDropshot {
        bind_address: std::net::SocketAddr::new(
            std::net::IpAddr::from_str(bind_ip_str).unwrap(),
            bind_port,
        ),
        request_body_max_bytes: 1024,
        default_handler_task_mode,
        log_headers: Default::default(),
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
                HandlerTaskMode::CancelOnDisconnect,
            );
            make_server(0, &config, &self.log, None, None).start()
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
    struct ConfigBindServerHttps<'a> {
        log: slog::Logger,
        certs: Vec<rustls::pki_types::CertificateDer<'a>>,
        cert_file: NamedTempFile,
        key_file: NamedTempFile,
    }

    impl<'a>
        TestConfigBindServer<
            hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>,
        > for ConfigBindServerHttps<'a>
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
                .add(self.certs[self.certs.len() - 1].clone())
                .expect("adding root cert");

            let tls_config = rustls::ClientConfig::builder()
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
                HandlerTaskMode::CancelOnDisconnect,
            );
            make_server(0, &config, &self.log, tls, None).start()
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
    struct ConfigBindServerHttps<'a> {
        log: slog::Logger,
        certs: Vec<rustls::pki_types::CertificateDer<'a>>,
        serialized_certs: Vec<u8>,
        serialized_key: Vec<u8>,
    }

    impl<'a>
        TestConfigBindServer<
            hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>,
        > for ConfigBindServerHttps<'a>
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
                .add(self.certs[self.certs.len() - 1].clone())
                .expect("adding root cert");

            let tls_config = rustls::ClientConfig::builder()
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
                HandlerTaskMode::CancelOnDisconnect,
            );
            make_server(0, &config, &self.log, tls, None).start()
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

struct HandlerTaskModeContext {
    // Our endpoint handler reports that it has started on this channel.
    endpoint_started_tx: mpsc::UnboundedSender<()>,

    // Our endpoint handler waits to proceed until it receives a message on this
    // channel.
    release_endpoint_rx: async_channel::Receiver<()>,

    // Our endpoint handler reports how it completed (either normally or
    // cancelled) on this channel.
    endpoint_finished_tx: mpsc::UnboundedSender<HandlerCompletionMode>,
}

#[derive(Debug, Clone, Copy)]
enum HandlerCompletionMode {
    CompletedNormally,
    Cancelled,
}

struct ConfigHandlerTaskModeHttp {
    // Channels used to construct `HandlerTaskModeContext`.
    endpoint_started_tx: mpsc::UnboundedSender<()>,
    release_endpoint_rx: async_channel::Receiver<()>,
    endpoint_finished_tx: mpsc::UnboundedSender<HandlerCompletionMode>,

    // We bind to port 0 but need to know the port in `make_uri` below, so we
    // stash the actual port in this atomic.
    bound_port: AtomicU16,
    task_mode: HandlerTaskMode,

    log: slog::Logger,
}

impl TestConfigBindServer<hyper::client::connect::HttpConnector>
    for ConfigHandlerTaskModeHttp
{
    type Context = HandlerTaskModeContext;

    fn make_client(
        &self,
    ) -> hyper::Client<hyper::client::connect::HttpConnector> {
        hyper::Client::new()
    }

    fn make_server(&self, bind_port: u16) -> HttpServer<Self::Context> {
        struct DropReporter {
            endpoint_finished_tx:
                Option<mpsc::UnboundedSender<HandlerCompletionMode>>,
        }

        impl Drop for DropReporter {
            fn drop(&mut self) {
                // If we still have this channel, report that we've been
                // cancelled. The endpoint should steal it from us before we get
                // dropped if it completed normally.
                if let Some(tx) = self.endpoint_finished_tx.take() {
                    tx.send(HandlerCompletionMode::Cancelled).unwrap();
                }
            }
        }

        #[dropshot::endpoint {
            method = GET,
            path = "/",
        }]
        async fn track_cancel_endpoint(
            rqctx: RequestContext<HandlerTaskModeContext>,
        ) -> Result<HttpResponseOk<()>, HttpError> {
            let ctx = rqctx.context();

            // Construct a `DropReporter` to report our cancellation, unless we
            // steal the channel back from it (below).
            let mut drop_reporter = DropReporter {
                endpoint_finished_tx: Some(ctx.endpoint_finished_tx.clone()),
            };

            // Notify driving test that we've started.
            ctx.endpoint_started_tx.send(()).unwrap();

            // Wait until driving test tells us to continue. We may never
            // continue from this point if it cancels us instead.
            () = ctx.release_endpoint_rx.recv().await.unwrap();

            // We were not cancelled: steal the channel back from our drop
            // reporter to report normal completion.
            let tx = drop_reporter.endpoint_finished_tx.take().unwrap();
            tx.send(HandlerCompletionMode::CompletedNormally).unwrap();

            Ok(HttpResponseOk(()))
        }

        let context = HandlerTaskModeContext {
            endpoint_started_tx: self.endpoint_started_tx.clone(),
            release_endpoint_rx: self.release_endpoint_rx.clone(),
            endpoint_finished_tx: self.endpoint_finished_tx.clone(),
        };

        let config = make_config("127.0.0.1", bind_port, self.task_mode);
        let mut api = dropshot::ApiDescription::new();
        api.register(track_cancel_endpoint).unwrap();

        let server =
            make_server(context, &config, &self.log, None, Some(api)).start();

        self.bound_port.store(server.local_addr().port(), Ordering::SeqCst);

        server
    }

    fn make_uri(&self, _bind_port: u16) -> hyper::Uri {
        let bind_port = self.bound_port.load(Ordering::SeqCst);
        format!("http://localhost:{}/", bind_port).parse().unwrap()
    }
}

#[tokio::test]
async fn test_config_handler_task_mode_cancel() {
    let logctx = create_log_context("config_handler_task_mode_cancel");
    let log = logctx.log.new(o!());

    let (endpoint_started_tx, mut endpoint_started_rx) =
        mpsc::unbounded_channel();
    let (_release_endpoint_tx, release_endpoint_rx) =
        async_channel::unbounded();
    let (endpoint_finished_tx, mut endpoint_finished_rx) =
        mpsc::unbounded_channel();

    let test_config = ConfigHandlerTaskModeHttp {
        endpoint_started_tx,
        release_endpoint_rx,
        endpoint_finished_tx,
        bound_port: AtomicU16::new(0),
        task_mode: HandlerTaskMode::CancelOnDisconnect,
        log,
    };
    let bind_port = 0;

    let server = test_config.make_server(bind_port);

    // Spawn a task to hit the test endpoint.
    let client_task = {
        let client = test_config.make_client();
        let uri = test_config.make_uri(bind_port);
        tokio::spawn(async move { client.get(uri).await.unwrap() })
    };

    // Wait until the handler starts running.
    () = endpoint_started_rx.recv().await.unwrap();

    // Cancel the client task.
    client_task.abort();

    // Check that the handler was indeed cancelled.
    match endpoint_finished_rx.recv().await.unwrap() {
        HandlerCompletionMode::Cancelled => (),
        HandlerCompletionMode::CompletedNormally => {
            panic!("handler unexpectedly completed")
        }
    }

    server.close().await.unwrap();

    logctx.cleanup_successful();
}

#[tokio::test]
async fn test_config_handler_task_mode_detached() {
    let logctx = create_log_context("config_handler_task_mode_detached");
    let log = logctx.log.new(o!());

    let (endpoint_started_tx, mut endpoint_started_rx) =
        mpsc::unbounded_channel();
    let (release_endpoint_tx, release_endpoint_rx) = async_channel::unbounded();
    let (endpoint_finished_tx, mut endpoint_finished_rx) =
        mpsc::unbounded_channel();

    let test_config = ConfigHandlerTaskModeHttp {
        endpoint_started_tx,
        release_endpoint_rx,
        endpoint_finished_tx,
        bound_port: AtomicU16::new(0),
        task_mode: HandlerTaskMode::Detached,
        log,
    };
    let bind_port = 0;

    let server = test_config.make_server(bind_port);

    // Spawn a task to hit the test endpoint.
    let client_task = {
        let client = test_config.make_client();
        let uri = test_config.make_uri(bind_port);
        tokio::spawn(async move { client.get(uri).await.unwrap() })
    };

    // Wait until the handler starts running.
    () = endpoint_started_rx.recv().await.unwrap();

    // Cancel the client task.
    client_task.abort();

    // Despite cancelling the client task, it should still be running, and it's
    // waiting for us to release it to continue; do so.
    release_endpoint_tx.send(()).await.unwrap();

    // Check that the handler indeed completed normally despite the client
    // disconnect.
    match endpoint_finished_rx.recv().await.unwrap() {
        HandlerCompletionMode::CompletedNormally => (),
        HandlerCompletionMode::Cancelled => {
            panic!("handler unexpectedly cancelled")
        }
    }

    server.close().await.unwrap();

    logctx.cleanup_successful();
}

#[tokio::test]
async fn test_unversioned_servers_with_versioned_routes() {
    #[dropshot::endpoint {
        method = GET,
        path = "/handler",
        versions = "1.0.1".."1.0.1",
    }]
    async fn versioned_handler(
        _rqctx: RequestContext<i32>,
    ) -> Result<HttpResponseOk<u64>, HttpError> {
        Ok(HttpResponseOk(3))
    }

    let logctx =
        create_log_context("test_unversioned_servers_with_versioned_routes");
    let config_dropshot = ConfigDropshot::default();

    // Test both the HTTP and HTTPS code paths because the check is present in
    // both.
    let (certs, key) = common::generate_tls_key();
    let (serialized_certs, serialized_key) =
        common::tls_key_to_buffer(&certs, &key);
    let tls = Some(ConfigTls::AsBytes {
        certs: serialized_certs.clone(),
        key: serialized_key.clone(),
    });
    for tls_arg in [None, tls] {
        let mut api = dropshot::ApiDescription::new();
        api.register(versioned_handler).unwrap();
        let Err(error) = HttpServerStarter::new_with_tls(
            &config_dropshot,
            api,
            0,
            &logctx.log,
            tls_arg,
        ) else {
            panic!("expected failure to create server");
        };
        println!("{}", error);
        assert_eq!(
            error.to_string(),
            "unversioned servers cannot have endpoints with specific versions"
        );
    }
}
