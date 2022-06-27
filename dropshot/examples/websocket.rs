use dropshot::{
    endpoint, ApiDescription, HttpError, HttpResponseUpgradedWebSocket,
    RequestContext, WebSocketExt,
};
use futures::FutureExt;
use futures::StreamExt;
use std::sync::Arc;

fn main() -> Result<(), String> {
    /*
     * Build a description of the API.
     */
    let mut api = ApiDescription::new();
    api.register(websocket).unwrap();

    api.openapi("WebSocket Echo", "")
        .write(&mut std::io::stdout())
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[allow(unused_variables)]
#[endpoint {
    method = GET,
    path = "/echo",
}]
/// Echo a message back to the client.
async fn websocket(
    rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseUpgradedWebSocket, HttpError> {
    let (response, _) = rqctx
        .upgrade(None, |ws| {
            // Just echo all messages back...
            let (tx, rx) = ws.split();
            rx.forward(tx).map(|result| {
                if let Err(e) = result {
                    eprintln!("websocket error: {:?}", e);
                }
            })
        })
        .await?;

    Ok(response)
}
