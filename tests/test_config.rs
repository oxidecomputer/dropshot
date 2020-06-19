/*!
 * Tests for configuration file.
 */

use dropshot::test_util::read_config;
use dropshot::ConfigDropshot;
use dropshot::HttpServer;
use slog::Logger;
use std::fs;
use std::sync::Arc;

/*
 * Bad values for "bind_address"
 */

#[test]
fn test_config_bad_bind_address_port_too_small() {
    let error = read_config::<ConfigDropshot>(
        "bad_bind_address_port_too_small",
        "bind_address = \"127.0.0.1:-3\"",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.starts_with("invalid IP address syntax for key `bind_address`")
    );
}

#[test]
fn test_config_bad_bind_address_port_too_large() {
    let error = read_config::<ConfigDropshot>(
        "bad_bind_address_port_too_large",
        "bind_address = \"127.0.0.1:65536\"",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.starts_with("invalid IP address syntax for key `bind_address`")
    );
}

#[test]
fn test_config_bad_bind_address_garbage() {
    let error = read_config::<ConfigDropshot>(
        "bad_bind_address_garbage",
        "bind_address = \"garbage\"",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.starts_with("invalid IP address syntax for key `bind_address`")
    );
}

fn make_server(config: &ConfigDropshot, log: &Logger) -> HttpServer {
    HttpServer::new(&config, dropshot::ApiDescription::new(), Arc::new(0), log)
        .unwrap()
}

#[tokio::test]
async fn test_config_bind_address() {
    let log_path =
        dropshot::test_util::log_file_for_test("config_bind_address")
            .as_path()
            .display()
            .to_string();
    eprintln!("log file: {}", log_path);

    let log_config = dropshot::ConfigLogging::File {
        level: dropshot::ConfigLoggingLevel::Debug,
        path: log_path.clone(),
        if_exists: dropshot::ConfigLoggingIfExists::Append,
    };
    let log = log_config.to_logger("test_config_bind_address").unwrap();

    let client = hyper::Client::new();
    let bind_ip_str = "127.0.0.1";
    let bind_port: u16 = 12215;

    /*
     * This helper constructs a GET HTTP request to
     * http://$bind_ip_str:$port/, where $port is the argument to the
     * closure.
     */
    let cons_request = |port: u16| {
        let uri = hyper::Uri::builder()
            .scheme("http")
            .authority(format!("{}:{}", bind_ip_str, port).as_str())
            .path_and_query("/")
            .build()
            .unwrap();
        hyper::Request::builder()
            .method(http::method::Method::GET)
            .uri(&uri)
            .body(hyper::Body::empty())
            .unwrap()
    };

    /*
     * Make sure there is not currently a server running on our expected
     * port so that when we subsequently create a server and run it we know
     * we're getting the one we configured.
     */
    let error = client.request(cons_request(bind_port)).await.unwrap_err();
    assert!(error.is_connect());

    /*
     * Now start a server with our configuration and make the request again.
     * This should succeed in terms of making the request.  (The request
     * itself might fail with a 400-level or 500-level response code -- we
     * don't want to depend on too much from the ApiServer here -- but we
     * should have successfully made the request.)
     */
    let config_text =
        format!("bind_address = \"{}:{}\"\n", bind_ip_str, bind_port);
    let config =
        read_config::<ConfigDropshot>("bind_address", &config_text).unwrap();
    let mut server = make_server(&config, &log);
    let task = server.run();
    client.request(cons_request(bind_port)).await.unwrap();
    server.close();
    task.await.unwrap().unwrap();

    /*
     * Make another request to make sure it fails now that we've shut down
     * the server.
     */
    let error = client.request(cons_request(bind_port)).await.unwrap_err();
    assert!(error.is_connect());

    /*
     * Start a server on another TCP port and make sure we can reach that
     * one (and NOT the one we just shut down).
     */
    let config_text =
        format!("bind_address = \"{}:{}\"\n", bind_ip_str, bind_port + 1,);
    let config =
        read_config::<ConfigDropshot>("bind_address", &config_text).unwrap();
    let mut server = make_server(&config, &log);
    let task = server.run();
    client.request(cons_request(bind_port + 1)).await.unwrap();
    let error = client.request(cons_request(bind_port)).await.unwrap_err();
    assert!(error.is_connect());
    server.close();
    task.await.unwrap().unwrap();

    let error = client.request(cons_request(bind_port)).await.unwrap_err();
    assert!(error.is_connect());
    let error = client.request(cons_request(bind_port + 1)).await.unwrap_err();
    assert!(error.is_connect());

    fs::remove_file(log_path).unwrap();
}
