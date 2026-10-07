//! 边下载边播放的内存缓冲区。
//!
//! 下载任务把数据追加进共享缓冲区，[`StreamReader`] 以阻塞的 `Read + Seek`
//! 提供给解码器；读到尚未下载的位置时等待。

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use futures::StreamExt;
use qqmusic_api::transport::{Method, Request, Transport};
use tokio::sync::watch;

/// 开始播放前至少缓冲的字节数。
const PREBUFFER: u64 = 256 * 1024;
/// 单个音频文件下载的总超时。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(3600);

#[derive(Default)]
struct Buffer {
    data: Vec<u8>,
    done: bool,
    cancelled: bool,
    error: Option<String>,
}

#[derive(Default)]
struct Shared {
    buffer: Mutex<Buffer>,
    cond: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Buffer> {
        self.buffer.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 下载句柄；drop 时取消下载并唤醒阻塞的读取方。
pub struct StreamHandle {
    shared: Arc<Shared>,
}

impl StreamHandle {
    /// 已下载字节数。
    pub fn downloaded(&self) -> usize {
        self.shared.lock().data.len()
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.shared.lock().cancelled = true;
        self.shared.cond.notify_all();
    }
}

/// 解码器使用的阻塞读取端。
pub struct StreamReader {
    shared: Arc<Shared>,
    pos: u64,
    len: Option<u64>,
}

impl Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut guard = self.shared.lock();
        loop {
            let available = guard.data.len() as u64;
            if self.pos < available {
                let start = self.pos as usize;
                let n = buf.len().min(guard.data.len() - start);
                buf[..n].copy_from_slice(&guard.data[start..start + n]);
                self.pos += n as u64;
                return Ok(n);
            }
            if guard.cancelled {
                return Ok(0);
            }
            if guard.done {
                return match &guard.error {
                    Some(e) => Err(io::Error::other(e.clone())),
                    None => Ok(0),
                };
            }
            guard =
                self.shared.cond.wait_timeout(guard, Duration::from_millis(500)).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

impl Seek for StreamReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
            SeekFrom::End(d) => {
                let len = match self.len {
                    Some(len) => len,
                    None => self.wait_done(),
                };
                len.checked_add_signed(d)
            }
        };
        self.pos = target.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek 位置无效"))?;
        Ok(self.pos)
    }
}

impl StreamReader {
    /// 长度未知时等待下载结束，返回总长度。
    fn wait_done(&self) -> u64 {
        let mut guard = self.shared.lock();
        while !guard.done && !guard.cancelled {
            guard = self.shared.cond.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
        guard.data.len() as u64
    }
}

/// 打开音频 URL：返回下载句柄、读取端和 Content-Length。
///
/// 返回前会等待预缓冲完成（或下载结束）。
pub async fn open(
    transport: Arc<dyn Transport>,
    url: &str,
) -> Result<(StreamHandle, StreamReader, Option<u64>), String> {
    let mut request = Request::new(Method::Get, url);
    request.timeout = Some(DOWNLOAD_TIMEOUT);
    let response = transport.send_streaming(request).await.map_err(|e| format!("下载失败: {e}"))?;
    if !(200..300).contains(&response.status) {
        return Err(format!("下载失败: HTTP {}", response.status));
    }
    let len = response.header("content-length").and_then(|v| v.trim().parse::<u64>().ok());

    let shared = Arc::new(Shared::default());
    let (progress_tx, mut progress_rx) = watch::channel(0u64);
    let writer = shared.clone();
    let mut body = response.body;
    tokio::spawn(async move {
        while let Some(chunk) = body.next().await {
            let mut guard = writer.lock();
            if guard.cancelled {
                return;
            }
            match chunk {
                Ok(bytes) => guard.data.extend_from_slice(&bytes),
                Err(e) => {
                    guard.error = Some(format!("下载中断: {e}"));
                    break;
                }
            }
            let downloaded = guard.data.len() as u64;
            drop(guard);
            writer.cond.notify_all();
            progress_tx.send_replace(downloaded);
        }
        writer.lock().done = true;
        writer.cond.notify_all();
        progress_tx.send_replace(u64::MAX);
    });

    let target = len.map_or(PREBUFFER, |len| len.min(PREBUFFER));
    // 发送端在下载结束后 drop，wait_for 返回错误同样意味着可以开始。
    let _ = progress_rx.wait_for(|n| *n >= target).await;
    {
        let guard = shared.lock();
        if let Some(e) = &guard.error
            && guard.data.is_empty()
        {
            return Err(e.clone());
        }
    }
    let reader = StreamReader { shared: shared.clone(), pos: 0, len };
    Ok((StreamHandle { shared }, reader, len))
}
