//! 边下载边播放的内存缓冲区。
//!
//! 下载任务把数据追加进共享缓冲区，[`StreamReader`] 以阻塞的 `Read + Seek`
//! 提供给解码器；读到尚未下载的位置时等待。

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use futures::StreamExt;
use qqmusic_api::transport::{Method, Request, Transport};
use tokio::sync::watch;
use tokio::task::AbortHandle;

/// 开始播放前至少缓冲的字节数。
const PREBUFFER: u64 = 256 * 1024;
/// 单个音频文件下载的总超时。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(3600);
/// 等待响应头（首包）的超时。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// 下载途中连续多久收不到数据视为中断。
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

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
    /// 解码器当前读到的位置，用于判断缓冲是否充足。
    read_pos: AtomicU64,
    len: Option<u64>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Buffer> {
        self.buffer.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 下载句柄；drop 时取消下载任务并唤醒阻塞的读取方。
pub struct StreamHandle {
    shared: Arc<Shared>,
    task: Option<AbortHandle>,
}

impl StreamHandle {
    /// 已下载字节数。
    pub fn downloaded(&self) -> usize {
        self.shared.lock().data.len()
    }

    /// 已下载但尚未被解码器读取的字节数，以及下载是否已结束。
    pub fn ahead(&self) -> (u64, bool) {
        let guard = self.shared.lock();
        let read = self.shared.read_pos.load(Ordering::Relaxed);
        ((guard.data.len() as u64).saturating_sub(read), guard.done)
    }

    /// 文件总长度（Content-Length）。
    pub fn content_length(&self) -> Option<u64> {
        self.shared.len
    }

    /// 下载是否中途失败（断流、超时、数据不完整）；正常下载完为 `None`。
    ///
    /// 解码器读到失败处会像读到文件尾一样结束，调用方据此区分「播完」与「下载中断」。
    pub fn failure(&self) -> Option<String> {
        self.shared.lock().error.clone()
    }

    /// 已全部在内存中的数据（测试用，不涉及网络）。
    #[cfg(test)]
    pub(crate) fn ready(data: Vec<u8>, error: Option<String>) -> Self {
        let shared = Arc::new(Shared { len: Some(data.len() as u64), ..Default::default() });
        {
            let mut guard = shared.lock();
            guard.data = data;
            guard.done = true;
            guard.error = error;
        }
        Self { shared, task: None }
    }

    /// 从头创建一个新的读取端（数据保存在内存中，用于暂停后重建解码器）。
    pub fn reader(&self) -> StreamReader {
        self.shared.read_pos.store(0, Ordering::Relaxed);
        StreamReader { shared: self.shared.clone(), pos: 0, len: self.shared.len }
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.shared.lock().cancelled = true;
        self.shared.cond.notify_all();
        // 下载任务可能正卡在等待下一块数据上，直接中止，立即释放连接。
        if let Some(task) = &self.task {
            task.abort();
        }
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
                self.shared.read_pos.store(self.pos, Ordering::Relaxed);
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
        self.shared.read_pos.store(self.pos, Ordering::Relaxed);
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
/// 返回前会等待预缓冲完成（或下载结束）。等待响应头超过 20 秒、下载途中 30 秒收不到数据、
/// 连接提前断开或数据不足 Content-Length 都算失败：还没有任何数据时直接返回错误，
/// 否则记在句柄上（[`StreamHandle::failure`]）。本函数不重试；返回的 future 或句柄被 drop 时下载随即取消。
pub async fn open(
    transport: Arc<dyn Transport>,
    url: &str,
) -> Result<(StreamHandle, StreamReader, Option<u64>), String> {
    let mut request = Request::new(Method::Get, url);
    request.timeout = Some(DOWNLOAD_TIMEOUT);
    let response = tokio::time::timeout(CONNECT_TIMEOUT, transport.send_streaming(request))
        .await
        .map_err(|_| format!("下载失败: {} 秒内没有响应", CONNECT_TIMEOUT.as_secs()))?
        .map_err(|e| format!("下载失败: {e}"))?;
    if !(200..300).contains(&response.status) {
        return Err(format!("下载失败: HTTP {}", response.status));
    }
    let len = response.header("content-length").and_then(|v| v.trim().parse::<u64>().ok());

    let shared = Arc::new(Shared { len, ..Default::default() });
    // 按 Content-Length 一次性分配，避免边下边扩容时内存翻倍。
    if let Some(len) = len {
        shared.lock().data.reserve_exact(usize::try_from(len).unwrap_or(0).min(256 << 20));
    }
    let (progress_tx, mut progress_rx) = watch::channel(0u64);
    let writer = shared.clone();
    let mut body = response.body;
    let task = tokio::spawn(async move {
        let error = loop {
            let chunk = match tokio::time::timeout(STALL_TIMEOUT, body.next()).await {
                Err(_) => break Some(format!("下载中断: {} 秒内没有收到数据", STALL_TIMEOUT.as_secs())),
                Ok(None) => break None,
                Ok(Some(Err(e))) => break Some(format!("下载中断: {e}")),
                Ok(Some(Ok(bytes))) => bytes,
            };
            let mut guard = writer.lock();
            if guard.cancelled {
                return;
            }
            guard.data.extend_from_slice(&chunk);
            let downloaded = guard.data.len() as u64;
            drop(guard);
            writer.cond.notify_all();
            progress_tx.send_replace(downloaded);
        };
        let mut guard = writer.lock();
        let downloaded = guard.data.len() as u64;
        // 连接正常关闭但数据不足，同样是中断而不是文件结尾。
        guard.error = error.or_else(|| {
            writer.len.filter(|len| downloaded < *len).map(|len| format!("下载中断: 只收到 {downloaded} / {len} 字节"))
        });
        guard.done = true;
        drop(guard);
        writer.cond.notify_all();
        progress_tx.send_replace(u64::MAX);
    });
    // 先建句柄再等待：本 future 在预缓冲期间被取消时，句柄 drop 会中止下载任务。
    let handle = StreamHandle { shared: shared.clone(), task: Some(task.abort_handle()) };

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
    let reader = StreamReader { shared, pos: 0, len };
    Ok((handle, reader, len))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures::stream::{self, BoxStream};
    use qqmusic_api::transport::{Response, StreamingResponse, TransportError};

    use super::*;

    type Body = BoxStream<'static, Result<Vec<u8>, TransportError>>;

    /// 按脚本回放响应体的传输层。
    struct Scripted {
        headers: Vec<(String, String)>,
        body: Mutex<Option<Body>>,
        /// 响应头一直不来。
        hang: bool,
    }

    impl Scripted {
        fn transport(len: Option<u64>, body: Body) -> Arc<dyn Transport> {
            let headers = len.map(|n| ("Content-Length".to_string(), n.to_string())).into_iter().collect();
            Arc::new(Self { headers, body: Mutex::new(Some(body)), hang: false })
        }
    }

    #[async_trait::async_trait]
    impl Transport for Scripted {
        async fn send(&self, _request: Request) -> Result<Response, TransportError> {
            unreachable!("音频下载只走流式接口")
        }

        async fn send_streaming(&self, request: Request) -> Result<StreamingResponse, TransportError> {
            if self.hang {
                std::future::pending::<()>().await;
            }
            let body = self.body.lock().unwrap().take().expect("body requested once");
            Ok(StreamingResponse { status: 200, url: request.url, headers: self.headers.clone(), body })
        }
    }

    fn chunks(parts: Vec<Result<Vec<u8>, TransportError>>) -> Body {
        stream::iter(parts).boxed()
    }

    /// 先给出 `parts`，之后永远不再有数据也不结束。
    fn stalled(parts: Vec<Vec<u8>>) -> Body {
        stream::iter(parts.into_iter().map(Ok)).chain(stream::pending()).boxed()
    }

    fn read_all(mut reader: StreamReader) -> (Vec<u8>, io::Result<usize>) {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => return (out, Ok(0)),
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) => return (out, Err(e)),
            }
        }
    }

    #[tokio::test]
    async fn complete_download_with_and_without_content_length_is_a_clean_eof() {
        for len in [Some(6), None] {
            let transport = Scripted::transport(len, chunks(vec![Ok(vec![1, 2, 3]), Ok(vec![4, 5, 6])]));
            let (handle, reader, reported) = open(transport, "https://cdn/a.mp3").await.unwrap();
            assert_eq!(reported, len);
            let (data, end) = tokio::task::spawn_blocking(move || read_all(reader)).await.unwrap();
            assert_eq!(data, [1, 2, 3, 4, 5, 6]);
            assert!(end.is_ok());
            assert_eq!(handle.failure(), None);
            assert_eq!(handle.ahead(), (0, true));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn first_byte_timeout_is_an_error() {
        let transport: Arc<dyn Transport> =
            Arc::new(Scripted { headers: Vec::new(), body: Mutex::new(None), hang: true });
        let error = open(transport, "https://cdn/a.mp3").await.err().unwrap();
        assert!(error.contains("20 秒内没有响应"), "{error}");

        // 有响应头但一个字节都没有：同样在期限内失败，而不是无限等待。
        let error = open(Scripted::transport(Some(10), stalled(Vec::new())), "https://cdn/a.mp3").await.err().unwrap();
        assert!(error.contains("30 秒内没有收到数据"), "{error}");
    }

    #[tokio::test]
    async fn mid_stream_failures_are_reported_instead_of_looking_like_the_end() {
        // 连接报错。
        let broken = chunks(vec![Ok(vec![7; 100]), Err(TransportError::timeout("reset"))]);
        // 连接正常关闭但数据不足 Content-Length。
        let truncated = chunks(vec![Ok(vec![7; 100])]);
        for body in [broken, truncated] {
            let (handle, reader, _) = open(Scripted::transport(Some(1000), body), "https://cdn/a.mp3").await.unwrap();
            let (data, end) = tokio::task::spawn_blocking(move || read_all(reader)).await.unwrap();
            assert_eq!(data.len(), 100);
            assert!(end.is_err(), "读到断点应报错而不是 EOF");
            assert!(handle.failure().unwrap().starts_with("下载中断"), "{:?}", handle.failure());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stall_after_data_is_reported() {
        let body = stalled(vec![vec![7; PREBUFFER as usize]]);
        let (handle, _reader, _) =
            open(Scripted::transport(Some(PREBUFFER * 4), body), "https://cdn/a.mp3").await.unwrap();
        assert_eq!(handle.failure(), None);
        tokio::time::sleep(STALL_TIMEOUT + Duration::from_secs(1)).await;
        assert!(handle.failure().unwrap().contains("没有收到数据"));
        assert!(handle.ahead().1);
    }

    #[tokio::test]
    async fn dropping_the_handle_or_the_open_future_cancels_the_download() {
        // 句柄 drop：下载任务被中止，阻塞中的读取立即返回。
        let body = stalled(vec![vec![7; PREBUFFER as usize]]);
        let (handle, mut reader, _) = open(Scripted::transport(None, body), "https://cdn/a.mp3").await.unwrap();
        let task = handle.task.clone().unwrap();
        reader.seek(SeekFrom::Start(PREBUFFER)).unwrap();
        let blocked = tokio::task::spawn_blocking(move || reader.read(&mut [0u8; 16]));
        drop(handle);
        let read = tokio::time::timeout(Duration::from_secs(2), blocked).await.expect("读取端应被唤醒").unwrap();
        assert_eq!(read.unwrap(), 0);
        tokio::time::timeout(Duration::from_secs(2), async {
            while !task.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("下载任务应在期限内结束");

        // 预缓冲期间取消 open（切歌）：通过响应体被 drop 观察下载任务确实结束了。
        struct Flag(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Flag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Flag(dropped.clone());
        let body = stream::pending::<Result<Vec<u8>, TransportError>>()
            .map(move |chunk| {
                let _ = &flag;
                chunk
            })
            .boxed();
        let opening = tokio::spawn(open(Scripted::transport(Some(PREBUFFER * 4), body), "https://cdn/a.mp3"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        opening.abort();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !dropped.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("取消 open 后下载任务应结束");
    }
}
