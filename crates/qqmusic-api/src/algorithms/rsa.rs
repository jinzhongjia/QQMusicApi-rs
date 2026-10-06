//! Minimal RSA public-key encryption (PKCS#1 v1.5, RFC 8017 §7.2.1).
//!
//! QIMEI registration encrypts a random AES key with a fixed 1024-bit public
//! key. Only the public operation is needed, so this replaces the full `rsa`
//! crate (and its `num-bigint-dig` / `rand 0.8` / pkcs dependency tree) with a
//! tiny DER reader plus `num-bigint` modular exponentiation.

use num_bigint::BigUint;
use rand::RngExt;

/// RSA error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RsaError {
    /// Malformed `SubjectPublicKeyInfo` DER.
    #[error("invalid RSA public key DER: {0}")]
    InvalidKey(&'static str),
    /// Message longer than `k - 11` bytes.
    #[error("message too long for RSA key ({len} > {max})")]
    MessageTooLong {
        /// Message length.
        len: usize,
        /// Maximum length.
        max: usize,
    },
}

/// RSA public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsaPublicKey {
    n: BigUint,
    e: BigUint,
    /// Modulus length in bytes.
    k: usize,
}

/// Read one DER TLV, returning `(tag, value, rest)`.
fn read_tlv(input: &[u8]) -> Result<(u8, &[u8], &[u8]), RsaError> {
    let (&tag, rest) = input.split_first().ok_or(RsaError::InvalidKey("truncated tag"))?;
    let (&first, mut rest) = rest.split_first().ok_or(RsaError::InvalidKey("truncated length"))?;
    let len = if first < 0x80 {
        usize::from(first)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 || rest.len() < count {
            return Err(RsaError::InvalidKey("bad length"));
        }
        let len = rest[..count].iter().fold(0usize, |acc, &b| (acc << 8) | usize::from(b));
        rest = &rest[count..];
        len
    };
    if rest.len() < len {
        return Err(RsaError::InvalidKey("truncated value"));
    }
    Ok((tag, &rest[..len], &rest[len..]))
}

fn expect(input: &[u8], tag: u8) -> Result<(&[u8], &[u8]), RsaError> {
    match read_tlv(input)? {
        (t, value, rest) if t == tag => Ok((value, rest)),
        _ => Err(RsaError::InvalidKey("unexpected tag")),
    }
}

const SEQUENCE: u8 = 0x30;
const INTEGER: u8 = 0x02;
const BIT_STRING: u8 = 0x03;
const OBJECT_ID: u8 = 0x06;
/// OID 1.2.840.113549.1.1.1 (rsaEncryption).
const RSA_ENCRYPTION: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];

impl RsaPublicKey {
    /// Build from big-endian modulus and exponent.
    pub fn new(modulus: &[u8], exponent: &[u8]) -> Result<Self, RsaError> {
        let n = BigUint::from_bytes_be(modulus);
        let e = BigUint::from_bytes_be(exponent);
        if n.bits() < 512 || e < BigUint::from(3u8) {
            return Err(RsaError::InvalidKey("modulus or exponent too small"));
        }
        let k = usize::try_from(n.bits().div_ceil(8)).map_err(|_| RsaError::InvalidKey("modulus too large"))?;
        Ok(Self { n, e, k })
    }

    /// Parse a DER `SubjectPublicKeyInfo` holding an `rsaEncryption` key.
    pub fn from_spki_der(der: &[u8]) -> Result<Self, RsaError> {
        let (spki, _) = expect(der, SEQUENCE)?;
        let (algorithm, rest) = expect(spki, SEQUENCE)?;
        let (oid, _) = expect(algorithm, OBJECT_ID)?;
        if oid != RSA_ENCRYPTION {
            return Err(RsaError::InvalidKey("not an rsaEncryption key"));
        }
        let (bits, _) = expect(rest, BIT_STRING)?;
        let (&unused, key) = bits.split_first().ok_or(RsaError::InvalidKey("empty bit string"))?;
        if unused != 0 {
            return Err(RsaError::InvalidKey("unaligned bit string"));
        }
        let (key, _) = expect(key, SEQUENCE)?;
        let (modulus, rest) = expect(key, INTEGER)?;
        let (exponent, _) = expect(rest, INTEGER)?;
        Self::new(modulus, exponent)
    }

    /// Modulus length in bytes.
    pub fn size(&self) -> usize {
        self.k
    }

    /// RSAES-PKCS1-v1_5 encryption with random non-zero padding.
    pub fn encrypt_pkcs1v15(&self, message: &[u8]) -> Result<Vec<u8>, RsaError> {
        let max = self.k.saturating_sub(11);
        if message.len() > max {
            return Err(RsaError::MessageTooLong { len: message.len(), max });
        }
        // EM = 0x00 || 0x02 || PS (non-zero, >= 8 bytes) || 0x00 || M
        let mut em = vec![0u8; self.k];
        em[1] = 0x02;
        let ps_end = self.k - message.len() - 1;
        let mut rng = rand::rng();
        for byte in &mut em[2..ps_end] {
            *byte = rng.random_range(1..=u8::MAX);
        }
        em[ps_end + 1..].copy_from_slice(message);
        Ok(self.raw_public(&em))
    }

    /// `m^e mod n` as a `k`-byte big-endian string.
    pub(crate) fn raw_public(&self, message: &[u8]) -> Vec<u8> {
        let c = BigUint::from_bytes_be(message).modpow(&self.e, &self.n).to_bytes_be();
        let mut out = vec![0u8; self.k - c.len()];
        out.extend_from_slice(&c);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test key pair generated offline (1024-bit, e = 65537).
    const N: &str = "cef538950cc2062fdb2838cb72c46a0cc509b2245828be7c2eae9fea783206d1fd358778ad2a572a764cf24171eb807db0a37a1b57865f96368b949cf24f5db939a59f5f0cabd02e3246e50f530f211222ae3656b04b3e62e3644a0a10e62b8471bb56d5a99b65ef82357874ce39d27482f45eb6773af8651145af575b9ebc07";
    const D: &str = "bb61d82b4bf3d8ba37ca013263e566c7cd176c4defec641bf80070165161aa3f32ede68ce2ecf523231da432913cd387fa08ea241b6934dec2e03a5ca82028104e46afff1fe6f043dcbed538a739dd76cd2ace11003f8ce7edd61a7a1bc18c54c436d5ac76572d765361ff019ca054c36c3c773a5afbc3fae1454e8b7b31aeb1";

    fn decrypt_raw(cipher: &[u8]) -> Vec<u8> {
        let n = BigUint::from_bytes_be(&hex::decode(N).unwrap());
        let d = BigUint::from_bytes_be(&hex::decode(D).unwrap());
        let m = BigUint::from_bytes_be(cipher).modpow(&d, &n).to_bytes_be();
        let mut out = vec![0u8; 128 - m.len()];
        out.extend_from_slice(&m);
        out
    }

    #[test]
    fn pkcs1v15_round_trip() {
        let key = RsaPublicKey::new(&hex::decode(N).unwrap(), &[1, 0, 1]).unwrap();
        assert_eq!(key.size(), 128);
        for message in [&b""[..], b"0123456789abcdef", &[0u8; 117]] {
            let cipher = key.encrypt_pkcs1v15(message).unwrap();
            assert_eq!(cipher.len(), 128);
            let em = decrypt_raw(&cipher);
            assert_eq!(&em[..2], &[0x00, 0x02]);
            let sep = 2 + em[2..].iter().position(|&b| b == 0).unwrap();
            assert!(sep - 2 >= 8, "padding string is at least 8 bytes");
            assert_eq!(&em[sep + 1..], message);
        }
        // Randomized padding.
        assert_ne!(key.encrypt_pkcs1v15(b"x").unwrap(), key.encrypt_pkcs1v15(b"x").unwrap());
    }

    #[test]
    fn ciphertext_keeps_leading_zero_bytes() {
        // Work backwards from small ciphertexts: c = (c^d)^e mod n. A naive
        // `to_bytes_be()` would return fewer than k bytes here.
        let n = BigUint::from_bytes_be(&hex::decode(N).unwrap());
        let d = BigUint::from_bytes_be(&hex::decode(D).unwrap());
        let key = RsaPublicKey::new(&hex::decode(N).unwrap(), &[1, 0, 1]).unwrap();
        for (c, leading_zeros) in [(BigUint::from(5u8), 127), (&n >> 8u32, 1)] {
            let m = c.modpow(&d, &n).to_bytes_be();
            let cipher = key.raw_public(&m);
            assert_eq!(cipher.len(), 128);
            assert!(cipher[..leading_zeros].iter().all(|&b| b == 0));
            assert_ne!(cipher[leading_zeros], 0);
            assert_eq!(BigUint::from_bytes_be(&cipher), c);
        }
    }

    #[test]
    fn randomized_round_trip_over_all_lengths() {
        let key = RsaPublicKey::new(&hex::decode(N).unwrap(), &[1, 0, 1]).unwrap();
        let mut rng = rand::rng();
        for len in (0..=117).chain(0..=117) {
            let message: Vec<u8> = (0..len).map(|_| rng.random_range(0..=u8::MAX)).collect();
            let em = decrypt_raw(&key.encrypt_pkcs1v15(&message).unwrap());
            let ps = &em[2..128 - len - 1];
            assert_eq!(&em[..2], &[0x00, 0x02]);
            assert!(ps.len() >= 8 && ps.iter().all(|&b| b != 0), "non-zero padding of at least 8 bytes");
            assert_eq!(em[128 - len - 1], 0x00);
            assert_eq!(&em[128 - len..], &message[..]);
        }
    }

    #[test]
    fn rejects_long_messages_and_bad_keys() {
        let key = RsaPublicKey::new(&hex::decode(N).unwrap(), &[1, 0, 1]).unwrap();
        assert_eq!(key.encrypt_pkcs1v15(&[0; 118]), Err(RsaError::MessageTooLong { len: 118, max: 117 }));
        assert!(RsaPublicKey::new(&[0xff; 16], &[1, 0, 1]).is_err());
        assert!(RsaPublicKey::from_spki_der(&[0x30, 0x81]).is_err());
        assert!(RsaPublicKey::from_spki_der(&[0x30, 0x03, 0x02, 0x01, 0x00]).is_err());
    }

    #[test]
    fn parses_spki_der() {
        // Wrap the test modulus into SubjectPublicKeyInfo DER.
        let mut rsa_key = vec![0x02, 0x81, 0x81, 0x00];
        rsa_key.extend(hex::decode(N).unwrap());
        rsa_key.extend([0x02, 0x03, 0x01, 0x00, 0x01]);
        let mut bits = vec![0x30, 0x81, rsa_key.len() as u8];
        bits.extend(rsa_key);
        let mut body = vec![0x30, 0x0d, 0x06, 0x09];
        body.extend(RSA_ENCRYPTION);
        body.extend([0x05, 0x00, 0x03, 0x81, (bits.len() + 1) as u8, 0x00]);
        body.extend(bits);
        let mut der = vec![0x30, 0x81, body.len() as u8];
        der.extend(body);
        let key = RsaPublicKey::from_spki_der(&der).unwrap();
        assert_eq!(key, RsaPublicKey::new(&hex::decode(N).unwrap(), &[1, 0, 1]).unwrap());
    }
}
