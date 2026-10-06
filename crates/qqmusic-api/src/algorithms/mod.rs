//! Cryptographic helpers: request signing and QRC lyric decryption.

pub mod qrc;
pub mod sign;
pub mod tripledes;

pub use qrc::{QrcError, qrc_decrypt, qrc_decrypt_bytes, qrc_encrypt};
pub use sign::zzc_sign;

#[cfg(test)]
mod tests {
    use super::zzc_sign;

    #[test]
    fn sign_matches_python_reference() {
        // Generated with upstream `zzc_sign`.
        let vectors = [
            ("{}", "zzcf8e26805gyafigxmxjehoe02mvsjjgtwzw6f1a05f9"),
            (
                r#"{"comm":{"ct":"11"}}"#,
                "zzc2a75107lfom5vfrr1wruwzj7j0vmg6zif415897d25",
            ),
            ("abc", "zzc163db6e8l6noj1uu5ylcp8kabaw6rnxos4b998e04e"),
            ("中文", "zzcced8026isvhrnzcvbk5anphu9nqfsgv9ga0ba66302"),
        ];
        for (payload, expected) in vectors {
            assert_eq!(zzc_sign(payload), expected, "payload {payload}");
        }
    }
}
