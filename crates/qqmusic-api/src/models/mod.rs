//! Response models.

pub mod album;
pub mod base;
pub mod comment;
pub mod lyric;
pub mod mv;
pub mod recommend;
pub mod search;
pub mod singer;
pub mod song;
pub mod songlist;
pub mod sound_power;
pub mod top;
pub mod user;

pub use base::{Album, CoverSize, File, Mv, Pay, Singer, Song, SongList};
