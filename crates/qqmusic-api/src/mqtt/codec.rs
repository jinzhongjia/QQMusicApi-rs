//! MQTT v5 packet encoding / decoding (subset).

use super::MqttError;

/// Property identifiers used by this client.
pub mod property_id {
    /// Server keep alive (u16).
    pub const SERVER_KEEP_ALIVE: u8 = 0x13;
    /// Authentication method (string).
    pub const AUTH_METHOD: u8 = 0x15;
    /// Server reference (string).
    pub const SERVER_REFERENCE: u8 = 0x1C;
    /// Reason string (string).
    pub const REASON_STRING: u8 = 0x1F;
    /// User property (string pair).
    pub const USER_PROPERTY: u8 = 0x26;
}

/// Properties understood by this client (others are skipped).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Properties {
    /// Authentication method.
    pub auth_method: Option<String>,
    /// User properties (ordered, duplicates allowed).
    pub user: Vec<(String, String)>,
    /// Server keep alive.
    pub server_keep_alive: Option<u16>,
    /// Server reference (redirect target).
    pub server_reference: Option<String>,
    /// Reason string.
    pub reason_string: Option<String>,
}

impl Properties {
    /// Properties with user pairs.
    pub fn with_user<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self { user: pairs.into_iter().map(|(k, v)| (k.into(), v.into())).collect(), ..Self::default() }
    }

    fn encode(&self) -> Vec<u8> {
        let mut body = Vec::new();
        if let Some(value) = self.server_keep_alive {
            body.push(property_id::SERVER_KEEP_ALIVE);
            body.extend(value.to_be_bytes());
        }
        if let Some(value) = &self.auth_method {
            body.push(property_id::AUTH_METHOD);
            put_str(&mut body, value);
        }
        if let Some(value) = &self.server_reference {
            body.push(property_id::SERVER_REFERENCE);
            put_str(&mut body, value);
        }
        if let Some(value) = &self.reason_string {
            body.push(property_id::REASON_STRING);
            put_str(&mut body, value);
        }
        for (key, value) in &self.user {
            body.push(property_id::USER_PROPERTY);
            put_str(&mut body, key);
            put_str(&mut body, value);
        }
        let mut out = Vec::with_capacity(body.len() + 4);
        put_varint(&mut out, body.len());
        out.extend(body);
        out
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, MqttError> {
        let len = reader.varint()?;
        let mut props = Reader::new(reader.take(len)?);
        let mut out = Self::default();
        while !props.is_empty() {
            let id = props.u8()?;
            match id {
                property_id::SERVER_KEEP_ALIVE => out.server_keep_alive = Some(props.u16()?),
                property_id::AUTH_METHOD => out.auth_method = Some(props.string()?),
                property_id::SERVER_REFERENCE => out.server_reference = Some(props.string()?),
                property_id::REASON_STRING => out.reason_string = Some(props.string()?),
                property_id::USER_PROPERTY => {
                    let key = props.string()?;
                    let value = props.string()?;
                    out.user.push((key, value));
                }
                // Byte properties.
                0x01 | 0x17 | 0x19 | 0x24 | 0x25 | 0x28 | 0x29 | 0x2A => {
                    props.u8()?;
                }
                // Two byte integers.
                0x21..=0x23 => {
                    props.u16()?;
                }
                // Four byte integers.
                0x02 | 0x11 | 0x18 | 0x27 => {
                    props.take(4)?;
                }
                // Variable byte integer.
                0x0B => {
                    props.varint()?;
                }
                // Strings / binary data (both length prefixed).
                0x03 | 0x08 | 0x09 | 0x12 | 0x16 | 0x1A => {
                    let len = props.u16()?;
                    props.take(len.into())?;
                }
                other => return Err(MqttError::Protocol(format!("unknown property 0x{other:02x}"))),
            }
        }
        Ok(out)
    }
}

/// Decoded packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    /// CONNACK.
    ConnAck {
        /// Session present flag.
        session_present: bool,
        /// Reason code.
        reason: u8,
        /// Properties.
        properties: Properties,
    },
    /// PUBLISH.
    Publish {
        /// Topic.
        topic: String,
        /// Payload.
        payload: Vec<u8>,
        /// QoS.
        qos: u8,
        /// Packet id (QoS > 0).
        packet_id: Option<u16>,
        /// Properties.
        properties: Properties,
    },
    /// SUBACK.
    SubAck {
        /// Packet id.
        packet_id: u16,
        /// Reason codes.
        reason_codes: Vec<u8>,
    },
    /// PINGRESP.
    PingResp,
    /// DISCONNECT.
    Disconnect {
        /// Reason code.
        reason: u8,
        /// Properties.
        properties: Properties,
    },
    /// Any other packet type (ignored).
    Other(u8),
}

fn put_varint(out: &mut Vec<u8>, mut value: usize) {
    loop {
        let mut byte = (value % 128) as u8;
        value /= 128;
        if value > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn put_str(out: &mut Vec<u8>, value: &str) {
    let bytes = value.as_bytes();
    let len = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
    out.extend(len.to_be_bytes());
    out.extend(&bytes[..usize::from(len)]);
}

fn packet(header: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![header];
    put_varint(&mut out, body.len());
    out.extend(body);
    out
}

/// Encode CONNECT (MQTT 5, clean start).
pub fn encode_connect(client_id: &str, keep_alive: u16, properties: &Properties) -> Vec<u8> {
    let mut body = Vec::new();
    put_str(&mut body, "MQTT");
    body.push(5);
    body.push(0x02);
    body.extend(keep_alive.to_be_bytes());
    body.extend(properties.encode());
    put_str(&mut body, client_id);
    packet(0x10, &body)
}

/// Encode SUBSCRIBE with QoS 0.
pub fn encode_subscribe(packet_id: u16, topic: &str, properties: &Properties) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(packet_id.to_be_bytes());
    body.extend(properties.encode());
    put_str(&mut body, topic);
    body.push(0x00);
    packet(0x82, &body)
}

/// Encode PUBACK.
pub fn encode_puback(packet_id: u16) -> Vec<u8> {
    packet(0x40, &packet_id.to_be_bytes())
}

/// Encode PINGREQ.
pub fn encode_pingreq() -> Vec<u8> {
    vec![0xC0, 0x00]
}

/// Encode DISCONNECT (normal).
pub fn encode_disconnect() -> Vec<u8> {
    vec![0xE0, 0x00]
}

struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], MqttError> {
        if self.data.len() < n {
            return Err(MqttError::Protocol("truncated packet".into()));
        }
        let (head, tail) = self.data.split_at(n);
        self.data = tail;
        Ok(head)
    }

    fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.data)
    }

    fn u8(&mut self) -> Result<u8, MqttError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, MqttError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn varint(&mut self) -> Result<usize, MqttError> {
        let mut value = 0usize;
        for shift in 0..4 {
            let byte = self.u8()?;
            value |= usize::from(byte & 0x7F) << (7 * shift);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(MqttError::Protocol("malformed variable byte integer".into()))
    }

    fn string(&mut self) -> Result<String, MqttError> {
        let len = self.u16()?;
        let bytes = self.take(len.into())?;
        String::from_utf8(bytes.to_vec()).map_err(|_| MqttError::Protocol("invalid UTF-8 string".into()))
    }
}

/// Streaming decoder: feed WebSocket frames, pull complete packets.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    /// Append received bytes.
    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Decode the next complete packet, if any.
    pub fn next_packet(&mut self) -> Result<Option<Packet>, MqttError> {
        if self.buf.len() < 2 {
            return Ok(None);
        }
        // Remaining length: up to four bytes after the fixed header byte.
        let mut length = 0usize;
        let mut header_len = 1;
        loop {
            let Some(&byte) = self.buf.get(header_len) else {
                return Ok(None);
            };
            length |= usize::from(byte & 0x7F) << (7 * (header_len - 1));
            header_len += 1;
            if byte & 0x80 == 0 {
                break;
            }
            if header_len > 4 {
                return Err(MqttError::Protocol("malformed remaining length".into()));
            }
        }
        if self.buf.len() < header_len + length {
            return Ok(None);
        }
        let first = self.buf[0];
        let body: Vec<u8> = self.buf[header_len..header_len + length].to_vec();
        self.buf.drain(..header_len + length);
        decode_packet(first, &body).map(Some)
    }
}

fn decode_packet(first: u8, body: &[u8]) -> Result<Packet, MqttError> {
    let mut reader = Reader::new(body);
    Ok(match first >> 4 {
        2 => {
            let flags = reader.u8()?;
            let reason = reader.u8()?;
            let properties = if reader.is_empty() { Properties::default() } else { Properties::decode(&mut reader)? };
            Packet::ConnAck { session_present: flags & 1 == 1, reason, properties }
        }
        3 => {
            let qos = (first >> 1) & 0x03;
            let topic = reader.string()?;
            let packet_id = if qos > 0 { Some(reader.u16()?) } else { None };
            let properties = Properties::decode(&mut reader)?;
            Packet::Publish { topic, payload: reader.rest().to_vec(), qos, packet_id, properties }
        }
        9 => {
            let packet_id = reader.u16()?;
            Properties::decode(&mut reader)?;
            Packet::SubAck { packet_id, reason_codes: reader.rest().to_vec() }
        }
        13 => Packet::PingResp,
        14 => {
            let reason = if reader.is_empty() { 0 } else { reader.u8()? };
            let properties = if reader.is_empty() { Properties::default() } else { Properties::decode(&mut reader)? };
            Packet::Disconnect { reason, properties }
        }
        other => Packet::Other(other),
    })
}

/// Server side encoders used by tests.
#[cfg(test)]
pub(crate) fn encode_connack_for_test(reason: u8, properties: &Properties) -> Vec<u8> {
    let mut body = vec![0, reason];
    body.extend(properties.encode());
    packet(0x20, &body)
}

#[cfg(test)]
pub(crate) fn encode_suback_for_test(packet_id: u16, codes: &[u8]) -> Vec<u8> {
    let mut body = packet_id.to_be_bytes().to_vec();
    body.push(0);
    body.extend(codes);
    packet(0x90, &body)
}

#[cfg(test)]
pub(crate) fn encode_publish_for_test(
    topic: &str,
    payload: &[u8],
    qos: u8,
    packet_id: Option<u16>,
    user: &[(&str, &str)],
) -> Vec<u8> {
    let mut body = Vec::new();
    put_str(&mut body, topic);
    if let Some(id) = packet_id {
        body.extend(id.to_be_bytes());
    }
    body.extend(Properties::with_user(user.iter().copied()).encode());
    body.extend(payload);
    packet(0x30 | (qos << 1), &body)
}

#[cfg(test)]
pub(crate) fn encode_disconnect_for_test(reason: u8) -> Vec<u8> {
    packet(0xE0, &[reason, 0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip() {
        for value in [0usize, 127, 128, 16_383, 16_384, 2_097_151, 268_435_455] {
            let mut out = Vec::new();
            put_varint(&mut out, value);
            assert_eq!(Reader::new(&out).varint().unwrap(), value);
        }
        assert!(Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0x01]).varint().is_err());
    }

    #[test]
    fn connect_layout() {
        let props = Properties {
            auth_method: Some("pass".into()),
            user: vec![("a".into(), "b".into())],
            ..Properties::default()
        };
        let bytes = encode_connect("id", 45, &props);
        let expected_props = [0x15, 0, 4, b'p', b'a', b's', b's', 0x26, 0, 1, b'a', 0, 1, b'b'];
        let mut expected = vec![0, 4, b'M', b'Q', b'T', b'T', 5, 0x02, 0, 45, expected_props.len() as u8];
        expected.extend(expected_props);
        expected.extend([0, 2, b'i', b'd']);
        assert_eq!(bytes[0], 0x10);
        assert_eq!(usize::from(bytes[1]), expected.len());
        assert_eq!(&bytes[2..], expected.as_slice());
    }

    #[test]
    fn subscribe_layout() {
        let bytes = encode_subscribe(7, "t/x", &Properties::with_user([("pubsub", "unicast")]));
        assert_eq!(bytes[0], 0x82);
        assert_eq!(&bytes[2..4], &[0, 7]);
        assert!(bytes.ends_with(&[0, 3, b't', b'/', b'x', 0]));
        assert_eq!(encode_puback(258), vec![0x40, 2, 1, 2]);
    }

    #[test]
    fn decoder_handles_fragments_and_concatenation() {
        let mut stream = encode_connack_for_test(
            0,
            &Properties { server_keep_alive: Some(60), reason_string: Some("ok".into()), ..Properties::default() },
        );
        stream.extend(encode_publish_for_test("t", b"hello", 0, None, &[("type", "cookies")]));
        stream.extend([0xD0, 0x00]);
        stream.extend(encode_disconnect_for_test(0x8B));
        stream.extend([0xB0, 0x02, 0, 1]);
        let mut decoder = Decoder::default();
        let mut packets = Vec::new();
        for byte in stream {
            decoder.push(&[byte]);
            while let Some(packet) = decoder.next_packet().unwrap() {
                packets.push(packet);
            }
        }
        assert_eq!(packets.len(), 5);
        match &packets[0] {
            Packet::ConnAck { reason, properties, .. } => {
                assert_eq!(*reason, 0);
                assert_eq!(properties.server_keep_alive, Some(60));
                assert_eq!(properties.reason_string.as_deref(), Some("ok"));
            }
            other => panic!("{other:?}"),
        }
        match &packets[1] {
            Packet::Publish { topic, payload, qos, properties, .. } => {
                assert_eq!((topic.as_str(), payload.as_slice(), *qos), ("t", b"hello".as_slice(), 0));
                assert_eq!(properties.user, vec![("type".into(), "cookies".into())]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(packets[2], Packet::PingResp);
        assert!(matches!(packets[3], Packet::Disconnect { reason: 0x8B, .. }));
        assert_eq!(packets[4], Packet::Other(11));
    }

    #[test]
    fn skips_unknown_but_valid_properties() {
        // Publish with payload format (byte), message expiry (u32), content type (string), subscription id (varint).
        let mut body = Vec::new();
        put_str(&mut body, "t");
        let props = [0x01, 1, 0x02, 0, 0, 0, 9, 0x03, 0, 1, b'x', 0x0B, 0x81, 0x01, 0x23, 0, 5];
        body.push(props.len() as u8);
        body.extend(props);
        body.extend(b"p");
        let mut decoder = Decoder::default();
        decoder.push(&packet(0x30, &body));
        match decoder.next_packet().unwrap().unwrap() {
            Packet::Publish { payload, .. } => assert_eq!(payload, b"p"),
            other => panic!("{other:?}"),
        }
        let mut decoder = Decoder::default();
        decoder.push(&packet(0x20, &[0, 0, 2, 0x7F, 0]));
        assert!(decoder.next_packet().is_err());
        let mut decoder = Decoder::default();
        decoder.push(&packet(0x30, &[0, 5, b'a']));
        assert!(decoder.next_packet().is_err());
    }
}
