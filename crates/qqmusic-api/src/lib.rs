//! Async Rust client for the QQ Music API.

extern crate self as qqmusic_api;

pub mod algorithms;
pub mod bypass;
pub mod client;
pub mod credential;
pub mod device;
pub mod error;
pub mod json;
pub mod models;
pub mod mqtt;
pub mod modules;
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

pub use bypass::BypassConfig;
pub use client::{Client, ClientBuilder, DeviceSource, Endpoints, QimeiMode};
pub use credential::Credential;
pub use device::{Device, DeviceProfile};
pub use error::{ApiError, ApiErrorKind, Error, Result};
pub use json::FromJson;
pub use pagination::Paged;
pub use request::{CgiRequest, HttpRequest};
pub use versioning::{Platform, VersionPolicy, VersionProfile};
