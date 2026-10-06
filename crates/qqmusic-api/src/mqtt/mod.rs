//! Minimal MQTT v5 client used by the QQ Music App QR login.
//!
//! Only the subset needed by `mu.y.qq.com` is implemented: CONNECT (with
//! authentication method / user properties), CONNACK (including server
//! redirects), SUBSCRIBE / SUBACK, PUBLISH (QoS 0/1), PINGREQ and DISCONNECT.
//! The network layer is pluggable through [`MqttConnector`] so the protocol
//! logic can be tested without sockets and routed through custom proxies.

pub mod codec;
#[cfg(feature = "mobile-login")]
pub mod ws;

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::Value;
use tokio::time::{Instant, sleep_until, timeout};

use crate::error::{Error, TransportError};
pub use codec::{Packet, Properties};

/// Default connect timeout.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// MQTT failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MqttError {
    /// Network / WebSocket failure.
    #[error("MQTT transport error: {0}")]
    Transport(String),
    /// Malformed packet.
    #[error("MQTT protocol error: {0}")]
    Protocol(String),
    /// The broker refused the connection.
    #[error("MQTT connect failed, reason code 0x{0:02x}")]
    ConnectRefused(u8),
    /// Too many server redirects.
    #[error("MQTT server moved to {server} (reason 0x{reason:02x})")]
    Redirect {
        /// Server reference.
        server: String,
        /// Reason code.
        reason: u8,
    },
    /// SUBACK contained failure reason codes.
    #[error("MQTT SUBACK rejected: {0:?}")]
    SubscribeRejected(Vec<u8>),
    /// The broker disconnected unexpectedly.
    #[error("MQTT disconnected, reason code 0x{0:02x}")]
    Disconnected(u8),
    /// The connection was closed.
    #[error("MQTT connection closed")]
    Closed,
    /// An operation timed out.
    #[error("MQTT {0} timed out")]
    Timeout(&'static str),
}

impl From<MqttError> for Error {
    fn from(err: MqttError) -> Self {
        let message = err.to_string();
        Error::Network(match err {
            MqttError::Timeout(_) => TransportError::timeout(message),
            MqttError::Transport(_) | MqttError::ConnectRefused(_) | MqttError::Redirect { .. } => {
                TransportError::connect(message)
            }
            _ => TransportError::new(message),
        })
    }
}

/// Bidirectional packet channel (one WebSocket binary frame per write).
#[async_trait]
pub trait PacketIo: Send {
    /// Send bytes.
    async fn send(&mut self, data: Vec<u8>) -> Result<(), MqttError>;
    /// Receive the next chunk; `None` when the peer closed the connection.
    ///
    /// Must be cancel safe (it is polled inside `select!`).
    async fn recv(&mut self) -> Result<Option<Vec<u8>>, MqttError>;
    /// Close the channel.
    async fn close(&mut self) {}
}

/// WebSocket endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsTarget {
    /// Host.
    pub host: String,
    /// Port.
    pub port: u16,
    /// Path.
    pub path: String,
    /// Extra handshake headers.
    pub headers: Vec<(String, String)>,
}

impl WsTarget {
    /// `wss://` URL.
    pub fn url(&self) -> String {
        if self.port == 443 {
            format!("wss://{}{}", self.host, self.path)
        } else {
            format!("wss://{}:{}{}", self.host, self.port, self.path)
        }
    }
}

/// Opens [`PacketIo`] channels.
#[async_trait]
pub trait MqttConnector: Send + Sync + fmt::Debug {
    /// Connect to `target`.
    async fn connect(&self, target: &WsTarget) -> Result<Box<dyn PacketIo>, MqttError>;
}

/// Received PUBLISH message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttMessage {
    /// Topic.
    pub topic: String,
    /// Payload.
    pub payload: Vec<u8>,
    /// QoS.
    pub qos: u8,
    /// User properties.
    pub properties: IndexMap<String, String>,
}

impl MqttMessage {
    /// Payload parsed as JSON.
    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.payload).ok()
    }
}

/// Connection options.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// Client id.
    pub client_id: String,
    /// Keep alive in seconds.
    pub keep_alive: u16,
    /// CONNECT properties.
    pub properties: Properties,
    /// Maximum number of redirects to follow.
    pub max_redirects: usize,
    /// Connect timeout.
    pub connect_timeout: Duration,
}

impl ConnectOptions {
    /// Options with defaults.
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            keep_alive: 45,
            properties: Properties::default(),
            max_redirects: 3,
            connect_timeout: CONNECT_TIMEOUT,
        }
    }
}

/// Compute the path used after a server redirect.
pub fn redirect_path(path: &str, server_reference: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    let mut parts: Vec<&str> = trimmed.split('/').collect();
    if parts.last().is_some_and(|p| p.contains(':')) {
        parts.pop();
        parts.push(server_reference);
        return parts.join("/");
    }
    format!("{trimmed}/{server_reference}")
}

/// Connected MQTT session.
pub struct MqttSession {
    io: Box<dyn PacketIo>,
    decoder: codec::Decoder,
    keep_alive: Duration,
    next_ping: Instant,
    next_packet_id: u16,
    pending: VecDeque<MqttMessage>,
    path: String,
}

impl fmt::Debug for MqttSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttSession")
            .field("keep_alive", &self.keep_alive)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl MqttSession {
    /// Connect (following up to `max_redirects` server redirects).
    pub async fn connect(connector: &dyn MqttConnector, target: WsTarget, options: &ConnectOptions) -> Result<Self, MqttError> {
        let mut target = target;
        let mut redirects = 0;
        loop {
            let attempt = Self::connect_once(connector, &target, options);
            let (session, reason, props) = timeout(options.connect_timeout, attempt)
                .await
                .map_err(|_| MqttError::Timeout("connect"))??;
            if reason == 0 {
                let mut session = session;
                if let Some(keep_alive) = props.server_keep_alive {
                    session.keep_alive = Duration::from_secs(keep_alive.into());
                }
                session.path = target.path.clone();
                session.schedule_ping();
                return Ok(session);
            }
            let mut session = session;
            session.io.close().await;
            match props.server_reference {
                Some(server) if matches!(reason, 0x9C | 0x9D) && !server.is_empty() => {
                    if redirects >= options.max_redirects {
                        return Err(MqttError::Redirect { server, reason });
                    }
                    redirects += 1;
                    tracing::debug!(server, reason, "MQTT redirect");
                    target.path = redirect_path(&target.path, &server);
                }
                _ => return Err(MqttError::ConnectRefused(reason)),
            }
        }
    }

    async fn connect_once(
        connector: &dyn MqttConnector,
        target: &WsTarget,
        options: &ConnectOptions,
    ) -> Result<(Self, u8, Properties), MqttError> {
        let io = connector.connect(target).await?;
        let mut session = Self {
            io,
            decoder: codec::Decoder::default(),
            keep_alive: Duration::from_secs(options.keep_alive.into()),
            next_ping: Instant::now(),
            next_packet_id: 1,
            pending: VecDeque::new(),
            path: target.path.clone(),
        };
        session
            .send(codec::encode_connect(&options.client_id, options.keep_alive, &options.properties))
            .await?;
        loop {
            match session.read_packet().await? {
                Packet::ConnAck { reason, properties, .. } => return Ok((session, reason, properties)),
                Packet::Disconnect { reason, .. } => return Err(MqttError::Disconnected(reason)),
                _ => {}
            }
        }
    }

    /// Effective keep alive.
    pub fn keep_alive(&self) -> Duration {
        self.keep_alive
    }

    /// Path of the connected endpoint (after redirects).
    pub fn path(&self) -> &str {
        &self.path
    }

    fn schedule_ping(&mut self) {
        self.next_ping = Instant::now() + self.keep_alive.max(Duration::from_secs(1));
    }

    async fn send(&mut self, data: Vec<u8>) -> Result<(), MqttError> {
        self.io.send(data).await?;
        self.schedule_ping();
        Ok(())
    }

    /// Read the next packet, sending PINGREQ when idle.
    async fn read_packet(&mut self) -> Result<Packet, MqttError> {
        loop {
            if let Some(packet) = self.decoder.next_packet()? {
                return Ok(packet);
            }
            let deadline = self.next_ping;
            tokio::select! {
                chunk = self.io.recv() => match chunk? {
                    Some(data) => self.decoder.push(&data),
                    None => return Err(MqttError::Closed),
                },
                () = sleep_until(deadline) => self.send(codec::encode_pingreq()).await?,
            }
        }
    }

    /// Subscribe with QoS 0 and wait for the SUBACK.
    pub async fn subscribe(&mut self, topic: &str, properties: &Properties) -> Result<(), MqttError> {
        let packet_id = self.next_packet_id;
        self.next_packet_id = self.next_packet_id.wrapping_add(1).max(1);
        self.send(codec::encode_subscribe(packet_id, topic, properties)).await?;
        let wait = self.keep_alive.max(Duration::from_secs(5));
        let result = timeout(wait, async {
            loop {
                match self.read_packet().await? {
                    Packet::SubAck { packet_id: id, reason_codes } if id == packet_id => return Ok(reason_codes),
                    packet => self.handle_async_packet(packet).await?,
                }
            }
        })
        .await
        .map_err(|_| MqttError::Timeout("subscribe"))??;
        if result.iter().any(|code| *code >= 0x80) {
            return Err(MqttError::SubscribeRejected(result));
        }
        Ok(())
    }

    async fn handle_async_packet(&mut self, packet: Packet) -> Result<(), MqttError> {
        match packet {
            Packet::Publish {
                topic,
                payload,
                qos,
                packet_id,
                properties,
            } => {
                if let (1, Some(id)) = (qos, packet_id) {
                    self.send(codec::encode_puback(id)).await?;
                }
                self.pending.push_back(MqttMessage {
                    topic,
                    payload,
                    qos,
                    properties: properties.user.into_iter().collect(),
                });
                Ok(())
            }
            Packet::Disconnect { reason, .. } => Err(MqttError::Disconnected(reason)),
            _ => Ok(()),
        }
    }

    /// Next PUBLISH message; `None` after a normal server disconnect.
    pub async fn next_message(&mut self) -> Result<Option<MqttMessage>, MqttError> {
        loop {
            if let Some(message) = self.pending.pop_front() {
                return Ok(Some(message));
            }
            match self.read_packet().await? {
                Packet::Disconnect { reason: 0, .. } => return Ok(None),
                packet => self.handle_async_packet(packet).await?,
            }
        }
    }

    /// Send DISCONNECT and close.
    pub async fn disconnect(mut self) {
        let _ = self.io.send(codec::encode_disconnect()).await;
        self.io.close().await;
    }
}

/// In-memory connector used by tests.
#[cfg(test)]
pub(crate) mod mock {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Scripted server: each connection receives the next script.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct MockConnector {
        pub(crate) scripts: Arc<Mutex<VecDeque<Vec<Vec<u8>>>>>,
        pub(crate) targets: Arc<Mutex<Vec<WsTarget>>>,
        pub(crate) sent: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl MockConnector {
        pub(crate) fn push_script(&self, chunks: Vec<Vec<u8>>) {
            self.scripts.lock().unwrap().push_back(chunks);
        }

        pub(crate) fn sent(&self) -> Vec<Vec<u8>> {
            self.sent.lock().unwrap().clone()
        }
    }

    struct MockIo {
        inbound: VecDeque<Vec<u8>>,
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
        close_when_empty: bool,
    }

    #[async_trait]
    impl PacketIo for MockIo {
        async fn send(&mut self, data: Vec<u8>) -> Result<(), MqttError> {
            self.sent.lock().unwrap().push(data);
            Ok(())
        }

        async fn recv(&mut self) -> Result<Option<Vec<u8>>, MqttError> {
            match self.inbound.pop_front() {
                Some(chunk) if chunk.is_empty() => {
                    // Empty chunk = "close now" marker.
                    Ok(None)
                }
                Some(chunk) => Ok(Some(chunk)),
                None if self.close_when_empty => Ok(None),
                None => std::future::pending().await,
            }
        }
    }

    #[async_trait]
    impl MqttConnector for MockConnector {
        async fn connect(&self, target: &WsTarget) -> Result<Box<dyn PacketIo>, MqttError> {
            self.targets.lock().unwrap().push(target.clone());
            let script = self
                .scripts
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| MqttError::Transport("no script".into()))?;
            Ok(Box::new(MockIo {
                inbound: script.into(),
                sent: self.sent.clone(),
                close_when_empty: false,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::codec::*;
    use super::mock::MockConnector;
    use super::*;

    fn target() -> WsTarget {
        WsTarget {
            host: "mu.y.qq.com".into(),
            port: 443,
            path: "/ws/handshake".into(),
            headers: vec![],
        }
    }

    #[test]
    fn redirect_paths() {
        assert_eq!(redirect_path("/ws/handshake", "a:1"), "/ws/handshake/a:1");
        assert_eq!(redirect_path("/ws/handshake/a:1/", "b:2"), "/ws/handshake/b:2");
        assert_eq!(target().url(), "wss://mu.y.qq.com/ws/handshake");
        assert_eq!(WsTarget { port: 8443, ..target() }.url(), "wss://mu.y.qq.com:8443/ws/handshake");
    }

    #[test]
    fn error_mapping() {
        let err: Error = MqttError::Timeout("connect").into();
        assert!(matches!(err, Error::Network(ref e) if e.timeout));
        let err: Error = MqttError::ConnectRefused(0x87).into();
        assert!(matches!(err, Error::Network(ref e) if e.connect));
        let err: Error = MqttError::Closed.into();
        assert!(matches!(err, Error::Network(ref e) if !e.connect && !e.timeout));
    }

    #[tokio::test]
    async fn connect_subscribe_and_receive() {
        let connector = MockConnector::default();
        let publish = encode_publish_for_test("t", br#"{"a":1}"#, 1, Some(9), &[("type", "scanned")]);
        // CONNACK + SUBACK in one chunk, PUBLISH split across two chunks.
        let mut first = encode_connack_for_test(0, &Properties { server_keep_alive: Some(30), ..Default::default() });
        first.extend(encode_suback_for_test(1, &[0]));
        let (a, b) = publish.split_at(3);
        connector.push_script(vec![first, a.to_vec(), b.to_vec(), encode_disconnect_for_test(0)]);
        let mut options = ConnectOptions::new("cid");
        options.properties.auth_method = Some("pass".into());
        options.properties.user.push(("k".into(), "v".into()));
        let mut session = MqttSession::connect(&connector, target(), &options).await.unwrap();
        assert_eq!(session.keep_alive(), Duration::from_secs(30));
        session.subscribe("t", &Properties::default()).await.unwrap();
        let message = session.next_message().await.unwrap().unwrap();
        assert_eq!(message.topic, "t");
        assert_eq!(message.properties["type"], "scanned");
        assert_eq!(message.json().unwrap()["a"], 1);
        assert!(session.next_message().await.unwrap().is_none());
        let sent = connector.sent();
        assert_eq!(sent[0][0], 0x10);
        assert_eq!(sent[1][0], 0x82);
        assert_eq!(sent[2], vec![0x40, 0x02, 0x00, 0x09], "PUBACK for QoS 1");
        session.disconnect().await;
        assert_eq!(connector.sent().last().unwrap(), &vec![0xE0, 0x00]);
    }

    #[tokio::test]
    async fn follows_redirects_and_limits_them() {
        let connector = MockConnector::default();
        let redirect = Properties {
            server_reference: Some("srv:1".into()),
            ..Default::default()
        };
        connector.push_script(vec![encode_connack_for_test(0x9D, &redirect)]);
        connector.push_script(vec![encode_connack_for_test(0, &Properties::default())]);
        let session = MqttSession::connect(&connector, target(), &ConnectOptions::new("c")).await.unwrap();
        assert_eq!(session.path(), "/ws/handshake/srv:1");
        assert_eq!(connector.targets.lock().unwrap()[1].path, "/ws/handshake/srv:1");

        let connector = MockConnector::default();
        for _ in 0..2 {
            connector.push_script(vec![encode_connack_for_test(0x9C, &redirect)]);
        }
        let mut options = ConnectOptions::new("c");
        options.max_redirects = 1;
        let err = MqttSession::connect(&connector, target(), &options).await.unwrap_err();
        assert!(matches!(err, MqttError::Redirect { reason: 0x9C, .. }));

        let connector = MockConnector::default();
        connector.push_script(vec![encode_connack_for_test(0x87, &Properties::default())]);
        let err = MqttSession::connect(&connector, target(), &ConnectOptions::new("c")).await.unwrap_err();
        assert_eq!(err, MqttError::ConnectRefused(0x87));
    }

    #[tokio::test]
    async fn suback_rejection_and_close() {
        let connector = MockConnector::default();
        let mut chunk = encode_connack_for_test(0, &Properties::default());
        chunk.extend(encode_suback_for_test(1, &[0x87]));
        connector.push_script(vec![chunk, Vec::new()]);
        let mut session = MqttSession::connect(&connector, target(), &ConnectOptions::new("c")).await.unwrap();
        let err = session.subscribe("t", &Properties::default()).await.unwrap_err();
        assert_eq!(err, MqttError::SubscribeRejected(vec![0x87]));
        assert_eq!(session.next_message().await.unwrap_err(), MqttError::Closed);

        let connector = MockConnector::default();
        let mut chunk = encode_connack_for_test(0, &Properties::default());
        chunk.extend(encode_disconnect_for_test(0x8E));
        connector.push_script(vec![chunk]);
        let mut session = MqttSession::connect(&connector, target(), &ConnectOptions::new("c")).await.unwrap();
        assert_eq!(session.next_message().await.unwrap_err(), MqttError::Disconnected(0x8E));
    }

    #[tokio::test(start_paused = true)]
    async fn keep_alive_pings_and_timeouts() {
        let connector = MockConnector::default();
        connector.push_script(vec![encode_connack_for_test(0, &Properties::default())]);
        let mut options = ConnectOptions::new("c");
        options.keep_alive = 2;
        let mut session = MqttSession::connect(&connector, target(), &options).await.unwrap();
        let res = timeout(Duration::from_secs(5), session.next_message()).await;
        assert!(res.is_err(), "no message arrives");
        let pings = connector.sent().iter().filter(|p| p.as_slice() == [0xC0, 0x00]).count();
        assert_eq!(pings, 2);

        // No CONNACK -> connect timeout.
        let connector = MockConnector::default();
        connector.push_script(vec![]);
        let err = MqttSession::connect(&connector, target(), &ConnectOptions::new("c")).await.unwrap_err();
        assert_eq!(err, MqttError::Timeout("connect"));
    }
}
