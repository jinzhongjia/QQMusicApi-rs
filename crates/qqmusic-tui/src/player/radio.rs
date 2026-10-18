//! 推荐队列的续播、去重和随机播放进度。

use std::collections::HashSet;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::protocol::{PlayMode, RecommendationSource, Track};
use crate::recommendation::Batch;

pub(super) const MAX_FAILURES: u32 = 3;

pub(super) struct Radio {
    pub source: RecommendationSource,
    pub seen: HashSet<String>,
    pub played: HashSet<String>,
    pub pending: Option<JoinHandle<Result<Batch, String>>>,
    pub retry_at: Instant,
    pub failures: u32,
    pub waiting: bool,
}

impl Radio {
    pub fn new(source: RecommendationSource, tracks: &[Track]) -> Self {
        let mut radio = Self {
            source,
            seen: HashSet::new(),
            played: HashSet::new(),
            pending: None,
            retry_at: Instant::now(),
            failures: 0,
            waiting: false,
        };
        radio.remember(tracks);
        radio
    }

    pub fn remember(&mut self, tracks: &[Track]) {
        self.seen.extend(tracks.iter().map(|t| t.mid.clone()));
    }

    pub fn cancel(&mut self) {
        if let Some(task) = self.pending.take() {
            task.abort();
        }
    }

    pub fn next_index(&self, tracks: &[Track], index: Option<usize>, mode: PlayMode, auto: bool) -> Option<usize> {
        let current = index.filter(|i| *i < tracks.len());
        if mode == PlayMode::Single && auto {
            return current;
        }
        if mode == PlayMode::Shuffle {
            let candidates: Vec<_> = tracks.iter().enumerate().filter(|(_, t)| !self.played.contains(&t.mid)).collect();
            return (!candidates.is_empty()).then(|| candidates[fastrand::usize(..candidates.len())].0);
        }
        let next = current.map_or(0, |i| i + 1);
        (next < tracks.len()).then_some(next)
    }
}

impl Drop for Radio {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// select! 取消等待时保留任务；切换队列时由 Radio::drop 取消任务。
pub(super) async fn next_batch(radio: &mut Option<Radio>) -> Result<Batch, String> {
    match radio.as_mut().and_then(|r| r.pending.as_mut()) {
        Some(task) => task.await.map_err(|e| format!("推荐请求任务失败: {e}"))?,
        None => std::future::pending().await,
    }
}

pub(super) const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
