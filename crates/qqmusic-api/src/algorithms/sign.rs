//! `zzc` request signature used by `musics.fcg`.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha1::{Digest, Sha1};

const PART_1_INDEXES: [usize; 7] = [23, 14, 6, 36, 16, 7, 19];
const PART_2_INDEXES: [usize; 8] = [16, 1, 32, 12, 19, 27, 8, 5];
const SCRAMBLE_VALUES: [u8; 20] = [
    89, 39, 179, 150, 218, 82, 58, 252, 177, 52, 186, 123, 120, 64, 242, 133, 143, 161, 121, 179,
];

/// Compute the `zzc` signature for a request payload.
///
/// The payload must be the exact bytes sent as the request body.
pub fn zzc_sign(payload: impl AsRef<[u8]>) -> String {
    let digest = Sha1::digest(payload.as_ref());
    let hash_hex = hex::encode_upper(digest);
    let hex_bytes = hash_hex.as_bytes();

    let part1: String = PART_1_INDEXES
        .iter()
        .map(|&i| char::from(hex_bytes[i]))
        .collect();
    let part2: String = PART_2_INDEXES
        .iter()
        .map(|&i| char::from(hex_bytes[i]))
        .collect();

    let mut part3 = [0u8; 20];
    for (i, (slot, scramble)) in part3.iter_mut().zip(SCRAMBLE_VALUES).enumerate() {
        *slot = scramble ^ digest[i];
    }
    let b64: String = STANDARD
        .encode(part3)
        .chars()
        .filter(|c| !matches!(c, '\\' | '/' | '+' | '='))
        .collect();

    format!("zzc{part1}{b64}{part2}").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_shape() {
        let sign = zzc_sign(b"{}");
        assert!(sign.starts_with("zzc"));
        assert_eq!(sign, sign.to_lowercase());
        // 3 + 7 + base64 (<= 28 chars) + 8
        assert!(sign.len() > 3 + 7 + 8);
    }

    #[test]
    fn deterministic() {
        assert_eq!(zzc_sign("abc"), zzc_sign(b"abc"));
        assert_ne!(zzc_sign("abc"), zzc_sign("abd"));
    }
}
