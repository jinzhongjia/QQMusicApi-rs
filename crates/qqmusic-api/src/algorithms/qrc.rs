//! QRC lyric decryption.

use std::io::Read as _;

use flate2::read::ZlibDecoder;

use super::tripledes::{Mode, TripleKeySchedule, tripledes_crypt, tripledes_key_setup};

const QRC_3DES_KEY: &[u8; 24] = b"!@#)(*$%123ZXC!@!@#)(NHL";

/// Errors raised while decrypting QRC data.
#[derive(Debug, thiserror::Error)]
pub enum QrcError {
    /// The hex payload could not be decoded.
    #[error("invalid hex payload: {0}")]
    InvalidHex(#[from] hex::FromHexError),
    /// Decompression failed (wrong key or corrupted data).
    #[error("decompression failed: {0}")]
    Decompress(#[from] std::io::Error),
    /// The decrypted text is not valid UTF-8.
    #[error("decrypted lyric is not valid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

fn schedule(mode: Mode) -> TripleKeySchedule {
    tripledes_key_setup(QRC_3DES_KEY, mode)
}

/// Decrypt raw QRC bytes.
///
/// An empty input yields an empty string. A trailing partial block is padded
/// with zeros (matching the upstream behaviour).
pub fn qrc_decrypt_bytes(encrypted: &[u8]) -> Result<String, QrcError> {
    if encrypted.is_empty() {
        return Ok(String::new());
    }
    let key = schedule(Mode::Decrypt);
    let mut data = Vec::with_capacity(encrypted.len());
    for chunk in encrypted.chunks(8) {
        let mut block = [0u8; 8];
        block[..chunk.len()].copy_from_slice(chunk);
        data.extend_from_slice(&tripledes_crypt(&block, &key));
    }
    let mut decoder = ZlibDecoder::new(data.as_slice());
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(String::from_utf8(out)?)
}

/// Decrypt a hex encoded QRC string as returned by the lyric API.
pub fn qrc_decrypt(encrypted_hex: &str) -> Result<String, QrcError> {
    let trimmed = encrypted_hex.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    qrc_decrypt_bytes(&hex::decode(trimmed)?)
}

/// Encrypt text into the QRC hex format (inverse of [`qrc_decrypt`]).
///
/// Mainly useful for tests and tooling.
pub fn qrc_encrypt(plain: &str) -> String {
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write as _;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    // Writing into a Vec cannot fail.
    let _ = encoder.write_all(plain.as_bytes());
    let compressed = encoder.finish().unwrap_or_default();
    let key = schedule(Mode::Encrypt);
    let mut out = Vec::with_capacity(compressed.len() + 8);
    for chunk in compressed.chunks(8) {
        let mut block = [0u8; 8];
        block[..chunk.len()].copy_from_slice(chunk);
        out.extend_from_slice(&tripledes_crypt(&block, &key));
    }
    hex::encode_upper(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &str = "[ti:测试]\n[0,1000]你(0,500)好(500,500)\n";
    /// Generated with the upstream Python implementation.
    const ENCRYPTED: &str = "AF52CCCEDF3E0E06D78BBF0DF18CD524903831EB0E4AC189FC58964633A9532985FAB0CDA27F3B4554D033274CFEA816";

    #[test]
    fn decrypts_python_vector() {
        assert_eq!(qrc_decrypt(ENCRYPTED).unwrap(), PLAIN);
        assert_eq!(qrc_decrypt(&ENCRYPTED.to_lowercase()).unwrap(), PLAIN);
    }

    #[test]
    fn encrypt_roundtrip() {
        let text = "[00:01.00]逐字歌词 roundtrip";
        assert_eq!(qrc_decrypt(&qrc_encrypt(text)).unwrap(), text);
    }

    #[test]
    fn empty_and_invalid() {
        assert_eq!(qrc_decrypt("").unwrap(), "");
        assert_eq!(qrc_decrypt_bytes(&[]).unwrap(), "");
        assert!(matches!(qrc_decrypt("zz"), Err(QrcError::InvalidHex(_))));
        assert!(matches!(
            qrc_decrypt("00112233445566778899"),
            Err(QrcError::Decompress(_))
        ));
    }
}
