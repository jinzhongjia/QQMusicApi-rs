//! API modules, accessed through [`Client`](crate::Client) methods.

pub mod song;

use crate::client::Client;

impl Client {
    /// Song APIs.
    pub fn song(&self) -> song::SongApi {
        song::SongApi::new(self)
    }
}
