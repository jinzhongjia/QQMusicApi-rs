//! Async Rust client for the QQ Music API.
//!
//! A port of the Python [QQMusicApi](https://github.com/L-1124/QQMusicApi)
//! with the playback region / risk-control bypass of
//! [decky-music](https://github.com/jinzhongjia/decky-music).
//!
//! # Quick start
//!
//! ```no_run
//! use qqmusic_api::Client;
//! use qqmusic_api::modules::search::SearchType;
//!
//! # async fn demo() -> qqmusic_api::Result<()> {
//! // Persist the emulated device so QQ Music sees one stable client.
//! let client = Client::builder().device_path("device.json").build()?;
//! let page = client.search().search_by_type("周杰伦", SearchType::Song).await?;
//! let detail = client.song().get_detail(page.song[0].song.mid.as_str()).await?;
//! # let _ = detail; Ok(()) }
//! ```
//!
//! # Customisation
//!
//! Every risk-control relevant parameter can be changed:
//!
//! * [`BypassConfig`] – `comm` used for playback URLs (`ct`, `cv`,
//!   credential, extra fields, CDN, User-Agent);
//! * [`VersionPolicy`] / [`VersionProfile`] – `ct`/`cv`/User-Agent and
//!   arbitrary `comm` overrides per [`Platform`];
//! * [`Device`] / [`DeviceSource`] – emulated Android device identity;
//! * [`ClientBuilder::transport`] – custom HTTP stack (TLS fingerprint,
//!   proxies, routing); [`ClientBuilder::header`] and `ClientBuilder::proxy`;
//! * per request: [`CgiRequest::platform`], [`CgiRequest::comm`],
//!   [`CgiRequest::param`], [`CgiRequest::bypass`].
//!
//! ```no_run
//! use qqmusic_api::modules::song::Quality;
//! use qqmusic_api::{BypassConfig, Client, Credential, VersionProfile};
//!
//! # async fn demo() -> qqmusic_api::Result<()> {
//! let client = Client::builder()
//!     .credential(Credential::new(123456, "Q_H_L_..."))
//!     .bypass(BypassConfig::default().with_fixed_ct(24).with_comm("tmeLoginType", "2"))
//!     .configure_version_policy(|policy| {
//!         policy.android = VersionProfile::android().with_cv(20_090_008).with_comm("chid", "10003505");
//!     })
//!     .build()?;
//! if let Some(url) = client.song().playable_url("0039MnYb0qxYhV", None, Quality::Lossless).await? {
//!     println!("{:?}: {}", url.quality, url.url);
//! }
//! # Ok(()) }
//! ```
//!
//! # Features
//!
//! * `reqwest-transport` (default) – `transport::ReqwestTransport`;
//! * `mobile-login` (default) – QQ Music App QR login over MQTT/WebSocket;
//! * `socks` – SOCKS proxy support for the reqwest transport.
//!
//! Without `reqwest-transport` a [`transport::Transport`] must be supplied.

extern crate self as qqmusic_api;

pub mod algorithms;
pub mod bypass;
pub mod client;
pub mod credential;
pub mod device;
pub mod error;
pub mod json;
pub mod models;
pub mod modules;
pub mod mqtt;
pub mod pagination;
pub mod qimei;
pub mod ratelimit;
pub mod request;
pub mod response;
pub mod session;
pub mod transport;
pub mod utils;
pub mod versioning;

#[cfg(test)]
pub(crate) mod testing;

/// Compile the README examples as doctests.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
pub struct ReadmeDoctests;

pub use bypass::BypassConfig;
pub use client::{Client, ClientBuilder, DeviceSource, Endpoints, QimeiMode};
pub use credential::Credential;
pub use device::{Device, DeviceProfile};
pub use error::{ApiError, ApiErrorKind, Error, Result};
pub use json::FromJson;
pub use pagination::Paged;
pub use request::{CgiRequest, HttpRequest};
pub use versioning::{Platform, VersionPolicy, VersionProfile};
