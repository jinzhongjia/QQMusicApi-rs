//! Shared rustls configuration for the HTTP and WebSocket transports.
//!
//! The crypto provider is selected by cargo features:
//!
//! | feature                  | provider   | needs a C toolchain |
//! |--------------------------|------------|---------------------|
//! | `tls-graviola` (default) | graviola   | no (pure Rust + verified assembly) |
//! | `tls-aws-lc`             | aws-lc-rs  | yes (cmake + C compiler) |
//! | `tls-ring`               | ring       | yes (C compiler) |
//!
//! If several are enabled, aws-lc-rs takes precedence over ring, which takes
//! precedence over graviola. graviola only supports `x86_64` (AES-NI, PCLMULQDQ,
//! BMI1, ADX, AVX, AVX2 — roughly every CPU since 2014) and `aarch64` (NEON,
//! AES, PMULL, SHA2); the CPU is probed at runtime and graviola is skipped on
//! unsupported hardware instead of panicking. When no compiled-in provider is
//! usable, the process-wide default installed with
//! [`CryptoProvider::install_default`] is used.
//!
//! Trust anchors come from `webpki-roots` (the Mozilla root program, bundled
//! into the binary), so no system certificate store is required. Enable the
//! `native-roots` feature to also trust the operating system store.

use std::sync::{Arc, OnceLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

use crate::error::Error;

/// ALPN protocols for HTTP/1.1 only.
pub const ALPN_HTTP1: &[&[u8]] = &[b"http/1.1"];
/// ALPN protocols preferring HTTP/2.
pub const ALPN_H2_HTTP1: &[&[u8]] = &[b"h2", b"http/1.1"];

/// rustls crypto provider in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TlsProvider {
    /// aws-lc-rs (feature `tls-aws-lc`).
    AwsLc,
    /// ring (feature `tls-ring`).
    Ring,
    /// graviola (feature `tls-graviola`).
    Graviola,
    /// Process-wide default installed by the application.
    ProcessDefault,
}

impl TlsProvider {
    /// Human readable name.
    pub fn name(self) -> &'static str {
        match self {
            Self::AwsLc => "aws-lc-rs",
            Self::Ring => "ring",
            Self::Graviola => "graviola",
            Self::ProcessDefault => "process default",
        }
    }
}

/// Whether the graviola provider can run on this CPU.
pub fn graviola_supported() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::is_x86_feature_detected as has;
        has!("aes")
            && has!("pclmulqdq")
            && has!("ssse3")
            && has!("bmi1")
            && has!("bmi2")
            && has!("adx")
            && has!("avx")
            && has!("avx2")
    }
    #[cfg(target_arch = "aarch64")]
    {
        use std::arch::is_aarch64_feature_detected as has;
        has!("neon") && has!("aes") && has!("pmull") && has!("sha2")
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        false
    }
}

#[allow(unreachable_code)]
fn builtin_provider() -> Option<(TlsProvider, Arc<CryptoProvider>)> {
    #[cfg(feature = "tls-aws-lc")]
    return Some((TlsProvider::AwsLc, Arc::new(rustls::crypto::aws_lc_rs::default_provider())));
    #[cfg(feature = "tls-ring")]
    return Some((TlsProvider::Ring, Arc::new(rustls::crypto::ring::default_provider())));
    #[cfg(all(feature = "tls-graviola", any(target_arch = "x86_64", target_arch = "aarch64")))]
    if graviola_supported() {
        return Some((TlsProvider::Graviola, Arc::new(rustls_graviola::default_provider())));
    }
    tracing::debug!("no compiled-in rustls provider is usable, falling back to the process default");
    None
}

/// Crypto provider used for new connections.
pub fn crypto_provider() -> Result<(TlsProvider, Arc<CryptoProvider>), Error> {
    static BUILTIN: OnceLock<Option<(TlsProvider, Arc<CryptoProvider>)>> = OnceLock::new();
    if let Some((kind, provider)) = BUILTIN.get_or_init(builtin_provider) {
        return Ok((*kind, Arc::clone(provider)));
    }
    CryptoProvider::get_default().map(|provider| (TlsProvider::ProcessDefault, Arc::clone(provider))).ok_or_else(|| {
        Error::invalid_argument(
            "没有可用的 rustls 加密后端: 请启用 tls-graviola / tls-aws-lc / tls-ring 特性, \
             或调用 rustls::crypto::CryptoProvider::install_default()",
        )
    })
}

/// Root certificates (webpki-roots, plus the OS store with `native-roots`).
pub fn root_store() -> Arc<RootCertStore> {
    static ROOTS: OnceLock<Arc<RootCertStore>> = OnceLock::new();
    Arc::clone(ROOTS.get_or_init(|| {
        #[allow(unused_mut)]
        let mut roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        #[cfg(feature = "native-roots")]
        {
            let native = rustls_native_certs::load_native_certs();
            for err in &native.errors {
                tracing::debug!(error = %err, "failed to load a native root certificate");
            }
            let (added, ignored) = roots.add_parsable_certificates(native.certs);
            tracing::debug!(added, ignored, "loaded native root certificates");
        }
        Arc::new(roots)
    }))
}

/// Build a rustls client configuration.
///
/// `alpn` lists the ALPN protocols to offer (see [`ALPN_H2_HTTP1`] /
/// [`ALPN_HTTP1`]). `accept_invalid_certs` disables certificate validation
/// (handshake signatures are still verified); only use it for debugging.
pub fn client_config(alpn: &[&[u8]], accept_invalid_certs: bool) -> Result<ClientConfig, Error> {
    let (_, provider) = crypto_provider()?;
    let algorithms = provider.signature_verification_algorithms;
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::invalid_argument(format!("TLS 加密后端不可用: {e}")))?;
    let mut config = if accept_invalid_certs {
        builder.dangerous().with_custom_certificate_verifier(Arc::new(NoVerifier(algorithms))).with_no_client_auth()
    } else {
        builder.with_root_certificates(root_store()).with_no_client_auth()
    };
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(config)
}

/// Accepts any certificate chain but still checks handshake signatures.
#[derive(Debug)]
struct NoVerifier(WebPkiSupportedAlgorithms);

impl ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether a usable provider is compiled in for this build.
    fn builtin_available() -> bool {
        cfg!(any(feature = "tls-aws-lc", feature = "tls-ring"))
            || (cfg!(all(feature = "tls-graviola", any(target_arch = "x86_64", target_arch = "aarch64")))
                && graviola_supported())
    }

    #[test]
    fn provider_and_config() {
        if !builtin_available() && CryptoProvider::get_default().is_none() {
            let err = crypto_provider().unwrap_err();
            assert!(err.to_string().contains("tls-graviola"), "{err}");
            assert!(client_config(ALPN_HTTP1, false).is_err());
            return;
        }
        let (kind, _) = crypto_provider().unwrap();
        #[cfg(feature = "tls-aws-lc")]
        assert_eq!(kind, TlsProvider::AwsLc);
        #[cfg(all(feature = "tls-ring", not(feature = "tls-aws-lc")))]
        assert_eq!(kind, TlsProvider::Ring);
        #[cfg(all(feature = "tls-graviola", not(any(feature = "tls-aws-lc", feature = "tls-ring"))))]
        assert_eq!(kind == TlsProvider::Graviola, graviola_supported());
        assert!(!kind.name().is_empty());

        let config = client_config(ALPN_H2_HTTP1, false).unwrap();
        assert_eq!(config.alpn_protocols, vec![b"h2".to_vec(), b"http/1.1".to_vec()]);
        let config = client_config(ALPN_HTTP1, true).unwrap();
        assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()]);
    }

    #[test]
    fn webpki_roots_are_bundled() {
        let roots = root_store();
        assert!(roots.len() >= webpki_roots::TLS_SERVER_ROOTS.len());
        assert!(Arc::ptr_eq(&roots, &root_store()), "root store is built once");
    }
}
