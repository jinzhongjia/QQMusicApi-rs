//! API modules, accessed through [`Client`](crate::Client) methods.

use serde_json::{Value, json};

use crate::client::Client;

/// Numeric id or mid of a resource.
///
/// Decimal strings are treated as ids (like upstream).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IdOrMid {
    /// Numeric id.
    Id(i64),
    /// Mid.
    Mid(String),
}

impl From<i64> for IdOrMid {
    fn from(id: i64) -> Self {
        Self::Id(id)
    }
}

impl From<&str> for IdOrMid {
    fn from(value: &str) -> Self {
        if !value.is_empty()
            && value.bytes().all(|b| b.is_ascii_digit())
            && let Ok(id) = value.parse()
        {
            return Self::Id(id);
        }
        Self::Mid(value.to_string())
    }
}

impl From<String> for IdOrMid {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<&String> for IdOrMid {
    fn from(value: &String) -> Self {
        Self::from(value.as_str())
    }
}

impl IdOrMid {
    /// `{id_key: id}` or `{mid_key: mid}`.
    pub(crate) fn param(&self, id_key: &str, mid_key: &str) -> Value {
        match self {
            Self::Id(id) => json!({ id_key: id }),
            Self::Mid(mid) => json!({ mid_key: mid }),
        }
    }

    /// Insert into an existing JSON object.
    pub(crate) fn insert(&self, param: &mut Value, id_key: &str, mid_key: &str) {
        match self {
            Self::Id(id) => param[id_key] = json!(id),
            Self::Mid(mid) => param[mid_key] = json!(mid),
        }
    }
}

/// `CgiRequest` builder shared by API modules.
macro_rules! api_module {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone)]
        pub struct $name {
            client: $crate::client::Client,
        }

        impl $name {
            pub(crate) fn new(client: &$crate::client::Client) -> Self {
                Self { client: client.clone() }
            }

            #[allow(dead_code)]
            fn cgi<T: $crate::FromJson + Send + 'static>(
                &self,
                module: &str,
                method: &str,
                param: serde_json::Value,
            ) -> $crate::request::CgiRequest<T> {
                $crate::request::CgiRequest::new(&self.client, $crate::request::CgiSpec::new(module, method, param))
            }
        }
    };
}
pub mod album;
pub mod comment;
pub mod lyric;
pub mod mv;
pub mod search;
pub mod singer;
pub mod song;
pub mod songlist;
pub mod top;

impl Client {
    /// Album APIs.
    pub fn album(&self) -> album::AlbumApi {
        album::AlbumApi::new(self)
    }

    /// Comment APIs.
    pub fn comment(&self) -> comment::CommentApi {
        comment::CommentApi::new(self)
    }

    /// Lyric APIs.
    pub fn lyric(&self) -> lyric::LyricApi {
        lyric::LyricApi::new(self)
    }

    /// MV APIs.
    pub fn mv(&self) -> mv::MvApi {
        mv::MvApi::new(self)
    }

    /// Search APIs.
    pub fn search(&self) -> search::SearchApi {
        search::SearchApi::new(self)
    }

    /// Singer APIs.
    pub fn singer(&self) -> singer::SingerApi {
        singer::SingerApi::new(self)
    }

    /// Song APIs.
    pub fn song(&self) -> song::SongApi {
        song::SongApi::new(self)
    }

    /// Playlist APIs.
    pub fn songlist(&self) -> songlist::SonglistApi {
        songlist::SonglistApi::new(self)
    }

    /// Top list APIs.
    pub fn top(&self) -> top::TopApi {
        top::TopApi::new(self)
    }
}
