//! WebSocket (`wss://`) connector: tokio TCP + rustls (shared
//! [`crate::tls`] configuration) + `tokio-tungstenite` handshake.

use std::sync::Arc;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};

use super::{MqttConnector, MqttError, PacketIo, WsTarget};

/// Default connector: `wss://` with the `mqtt` sub-protocol.
#[derive(Debug, Clone, Copy, Default)]
pub struct WsConnector;

struct WsIo {
    stream: WebSocketStream<TlsStream<TcpStream>>,
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

        let server_name = ServerName::try_from(target.host.clone()).map_err(transport)?;
        let config = crate::tls::client_config(crate::tls::ALPN_HTTP1, false).map_err(transport)?;
        let tcp = TcpStream::connect((target.host.as_str(), target.port)).await.map_err(transport)?;
        tcp.set_nodelay(true).map_err(transport)?;
        let tls = TlsConnector::from(Arc::new(config)).connect(server_name, tcp).await.map_err(transport)?;
        let (stream, _) = tokio_tungstenite::client_async(request, tls).await.map_err(transport)?;
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
                Some(Ok(Message::Binary(data))) => return Ok(Some(data.into())),
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
