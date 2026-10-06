//! API modules, accessed through [`Client`](crate::Client) methods.

pub mod search;
pub mod song;

use crate::client::Client;

impl Client {
    /// Search APIs.
    pub fn search(&self) -> search::SearchApi {
        search::SearchApi::new(self)
    }

    /// Song APIs.
    pub fn song(&self) -> song::SongApi {
        song::SongApi::new(self)
    }
}
