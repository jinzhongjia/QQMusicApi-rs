//! WebSocket (`wss://`) connector backed by `tokio-tungstenite`.

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::{MqttConnector, MqttError, PacketIo, WsTarget};

/// Default connector: plain `wss://` with the `mqtt` sub-protocol.
#[derive(Debug, Clone, Copy, Default)]
pub struct WsConnector;

struct WsIo {
    stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

fn transport(err: impl std::fmt::Display) -> MqttError {
    MqttError::Transport(err.to_string())
}

#[async_trait]
impl MqttConnector for WsConnector {
    async fn connect(&self, target: &WsTarget) -> Result<Box<dyn PacketIo>, MqttError> {
        let mut request = target.url().into_client_request().map_err(transport)?;
        let headers = request.headers_mut();
        headers.insert("Sec-WebSocket-Protocol", HeaderValue::from_static("mqtt"));
        for (name, value) in &target.headers {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(transport)?;
            let value = HeaderValue::from_str(value).map_err(transport)?;
            headers.insert(name, value);
        }
        let (stream, _) = tokio_tungstenite::connect_async(request).await.map_err(transport)?;
        Ok(Box::new(WsIo { stream }))
    }
}

#[async_trait]
impl PacketIo for WsIo {
    async fn send(&mut self, data: Vec<u8>) -> Result<(), MqttError> {
        self.stream.send(Message::Binary(data.into())).await.map_err(transport)
    }

    async fn recv(&mut self) -> Result<Option<Vec<u8>>, MqttError> {
        loop {
            match self.stream.next().await {
                None | Some(Ok(Message::Close(_))) => return Ok(None),
                Some(Ok(Message::Binary(data))) => return Ok(Some(data.to_vec())),
                Some(Ok(Message::Text(text))) => return Ok(Some(text.as_bytes().to_vec())),
                Some(Ok(_)) => {}
                Some(Err(err)) => return Err(transport(err)),
            }
        }
    }

    async fn close(&mut self) {
        let _ = self.stream.close(None).await;
    }
}
