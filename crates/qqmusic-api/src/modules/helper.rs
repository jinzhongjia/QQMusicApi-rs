//! File upload helper: `InitUpload` / `FinishUpload` plus direct upload of
//! the files to Tencent COS with the returned temporary credentials.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::stream::{self, StreamExt, TryStreamExt};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};

use crate::credential::Credential;
use crate::error::{Error, Result};
use crate::models::helper::{
    FinishUploadResponse, FinishUploadResult, InitUploadFile, InitUploadResponse, UploadAuthInfo, UploadObjectInfo,
};
use crate::request::CgiRequest;
use crate::transport::{Body, Method, Request, Transport};
use crate::utils::now_secs;

const MODULE: &str = "music.filesys.FileSystem";
const UPLOAD_RETRIES: u32 = 3;
const RETRY_STATUSES: [u16; 6] = [408, 429, 500, 502, 503, 504];
const SIGN_EXPIRE_SECS: i64 = 3600;

/// Upload business.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UploadBusiness {
    /// Playlist cover.
    Songlist,
    /// Profile homepage.
    Homepage,
    /// AI playlist assistant.
    AiPlAssistant,
}

impl UploadBusiness {
    /// `BusID` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Songlist => "songlist",
            Self::Homepage => "homepage",
            Self::AiPlAssistant => "Aiplassistant",
        }
    }
}

api_module! {
    /// File upload APIs.
    HelperApi
}

impl HelperApi {
    /// Request upload targets and temporary COS credentials.
    pub fn init_upload(
        &self,
        bus_id: UploadBusiness,
        files: &[InitUploadFile],
        credential: Option<Credential>,
    ) -> CgiRequest<InitUploadResponse> {
        self.cgi(MODULE, "InitUpload", json!({"BusID": bus_id.as_str(), "Files": files}))
            .preserve_bool(false)
            .sign(true)
            .require_login(true)
            .credential_opt(credential)
    }

    /// Report uploaded objects and obtain their URLs.
    pub fn finish_upload(
        &self,
        bus_id: UploadBusiness,
        results: &[FinishUploadResult],
        credential: Option<Credential>,
    ) -> CgiRequest<FinishUploadResponse> {
        let results: Vec<Value> = results.iter().map(FinishUploadResult::to_json).collect();
        self.cgi(MODULE, "FinishUpload", json!({"BusID": bus_id.as_str(), "Results": results}))
            .preserve_bool(false)
            .sign(true)
            .require_login(true)
            .credential_opt(credential)
    }

    /// High level upload flow.
    pub fn upload_session(&self, bus_id: UploadBusiness) -> UploadFileSession {
        UploadFileSession {
            api: self.clone(),
            bus_id,
            credential: None,
            max_concurrency: 3,
            init_data: None,
            last_file_shas: None,
        }
    }
}

/// RFC 3986 percent-encoding keeping `-_.~` and alphanumerics (and `/` when
/// `keep_slash`).
fn cos_quote(input: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') || (keep_slash && byte == b'/') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn hmac_sha1_hex(key: &[u8], data: &[u8]) -> String {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..20].copy_from_slice(&Sha1::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha1::new();
    inner.update(block.map(|b| b ^ 0x36));
    inner.update(data);
    let mut outer = Sha1::new();
    outer.update(block.map(|b| b ^ 0x5C));
    outer.update(inner.finalize());
    hex::encode(outer.finalize())
}

/// COS `Authorization` header (`q-sign-algorithm=sha1`) for a request
/// without query parameters, signing the given headers.
pub(crate) fn cos_authorization(
    secret_id: &str,
    secret_key: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    now: i64,
) -> String {
    let key_time = format!("{};{}", now - 60, now + SIGN_EXPIRE_SECS);
    let mut encoded: Vec<(String, String)> =
        headers.iter().map(|(k, v)| (cos_quote(k, false).to_lowercase(), cos_quote(v, false))).collect();
    encoded.sort();
    let header_string = encoded.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let header_list = encoded.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let http_string = format!("{}\n{path}\n\n{header_string}\n", method.to_lowercase());
    let string_to_sign = format!("sha1\n{key_time}\n{}\n", hex::encode(Sha1::digest(http_string.as_bytes())));
    let sign_key = hmac_sha1_hex(secret_key.as_bytes(), key_time.as_bytes());
    let signature = hmac_sha1_hex(sign_key.as_bytes(), string_to_sign.as_bytes());
    format!(
        "q-sign-algorithm=sha1&q-ak={secret_id}&q-sign-time={key_time}&q-key-time={key_time}&q-header-list={header_list}&q-url-param-list=&q-signature={signature}"
    )
}

/// Build a signed COS `PUT Object` request.
pub(crate) fn cos_put_request(
    auth: &UploadAuthInfo,
    bucket: &str,
    region: &str,
    object_key: &str,
    data: Vec<u8>,
    now: i64,
) -> Request {
    const CONTENT_TYPE: &str = "application/octet-stream";
    let host = format!("{bucket}.cos.{region}.myqcloud.com");
    let path = format!("/{}", object_key.trim_start_matches('/'));
    let authorization = cos_authorization(
        &auth.secret_id,
        &auth.secret_key,
        "PUT",
        &path,
        &[("Content-Type", CONTENT_TYPE), ("host", &host), ("x-cos-security-token", &auth.token)],
        now,
    );
    let mut request = Request::new(Method::Put, format!("https://{host}{}", cos_quote(&path, true)));
    request.set_header("Authorization", authorization);
    request.set_header("x-cos-security-token", auth.token.clone());
    request.body = Body::Raw { content_type: CONTENT_TYPE.into(), data };
    request.timeout = Some(Duration::from_secs(300));
    request
}

/// `PUT` with retries on transient failures (`2^attempt` seconds back-off).
async fn put_with_retries(transport: &dyn Transport, request: Request) -> Result<()> {
    let mut attempt = 0;
    loop {
        let retry = match transport.send(request.clone()).await {
            Ok(response) if response.status < 300 => return Ok(()),
            Ok(response) if attempt < UPLOAD_RETRIES && RETRY_STATUSES.contains(&response.status) => true,
            Ok(response) => {
                return Err(Error::Http {
                    status: response.status,
                    message: format!("COS upload failed: {}", String::from_utf8_lossy(&response.body)),
                });
            }
            Err(_) if attempt < UPLOAD_RETRIES => true,
            Err(err) => return Err(err.into()),
        };
        debug_assert!(retry);
        tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
        attempt += 1;
    }
}

async fn file_info(path: &Path) -> Result<(InitUploadFile, Vec<u8>)> {
    let metadata = tokio::fs::metadata(path).await;
    if !metadata.as_ref().is_ok_and(std::fs::Metadata::is_file) {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("文件不存在: {}", path.display()),
        )));
    }
    let data = tokio::fs::read(path).await?;
    let info = InitUploadFile {
        file_sha1: hex::encode(Sha1::digest(&data)),
        file_name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        file_size: data.len() as u64,
    };
    Ok((info, data))
}

/// Upload flow: hash files, `InitUpload` (cached while the credentials are
/// valid for the same files), PUT to COS, `FinishUpload`.
///
/// Unlike the upstream library (which uses multipart uploads above 5 MiB via
/// the COS SDK) files are sent with a single `PUT Object` (limit 5 GiB).
#[derive(Debug, Clone)]
pub struct UploadFileSession {
    api: HelperApi,
    bus_id: UploadBusiness,
    credential: Option<Credential>,
    max_concurrency: usize,
    init_data: Option<InitUploadResponse>,
    last_file_shas: Option<Vec<String>>,
}

impl UploadFileSession {
    /// Credential used for the CGI calls.
    #[must_use]
    pub fn credential(mut self, credential: Credential) -> Self {
        self.credential = Some(credential);
        self
    }

    /// Maximum parallel COS uploads (default 3).
    #[must_use]
    pub fn max_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = max.max(1);
        self
    }

    /// Current `InitUpload` data.
    pub fn init_data(&self) -> Option<&InitUploadResponse> {
        self.init_data.as_ref()
    }

    async fn prepare_files(&mut self, paths: &[PathBuf]) -> Result<Vec<Vec<u8>>> {
        if paths.is_empty() {
            return Err(Error::invalid_argument("至少需要提供一个文件路径."));
        }
        let loaded: Vec<(InitUploadFile, Vec<u8>)> =
            stream::iter(paths.iter().map(|p| file_info(p))).buffered(self.max_concurrency).try_collect().await?;
        let (infos, contents): (Vec<_>, Vec<_>) = loaded.into_iter().unzip();
        let shas: Vec<String> = infos.iter().map(|i| i.file_sha1.clone()).collect();
        if self.last_file_shas.as_ref() != Some(&shas) {
            self.init_data = None;
            self.last_file_shas = Some(shas);
        }
        if let Some(init) = &self.init_data
            && now_secs() < init.auth_info.expired_time - 600
        {
            return Ok(contents);
        }
        self.init_data = Some(self.api.init_upload(self.bus_id, &infos, self.credential.clone()).await?);
        Ok(contents)
    }

    /// Hash the files and obtain (or reuse) upload credentials.
    pub async fn prepare<P: AsRef<Path>>(&mut self, paths: &[P]) -> Result<()> {
        let paths: Vec<PathBuf> = paths.iter().map(|p| p.as_ref().to_path_buf()).collect();
        self.prepare_files(&paths).await.map(drop)
    }

    /// Upload files and return the stored objects.
    pub async fn upload<P: AsRef<Path>>(&mut self, paths: &[P]) -> Result<Vec<UploadObjectInfo>> {
        let paths: Vec<PathBuf> = paths.iter().map(|p| p.as_ref().to_path_buf()).collect();
        let contents = self.prepare_files(&paths).await?;
        let init = self.init_data.clone().ok_or_else(|| Error::api_data("获取上传凭证失败: 服务器未返回凭证信息"))?;
        if init.files.is_empty() || init.files.len() != paths.len() {
            return Err(Error::invalid_argument("InitUpload 返回的文件目标数量不匹配."));
        }
        let auth = &init.auth_info;
        let now = now_secs();
        let mut finish = Vec::with_capacity(paths.len());
        let mut uploads = Vec::new();
        for ((file, path), data) in init.files.iter().zip(&paths).zip(contents) {
            let target = file.buckets.first().ok_or_else(|| {
                Error::api_data(format!("获取上传凭证失败: 文件 {} 未返回目标存储桶信息.", path.display()))
            })?;
            let (bucket, region) = (&target.bucket.name, &target.bucket.region);
            let complete = [&auth.secret_id, &auth.secret_key, &auth.token, &file.object_key, bucket, region]
                .iter()
                .all(|v| !v.is_empty());
            if !complete {
                return Err(Error::api_data(format!("获取上传凭证失败: 文件 {} 上传凭证信息不完整.", path.display())));
            }
            if target.upload_status != 1 {
                uploads.push(cos_put_request(auth, bucket, region, &file.object_key, data, now));
            }
            finish.push(FinishUploadResult {
                bucket: bucket.clone(),
                region: region.clone(),
                object_key: file.object_key.clone(),
                upload_result: 0,
            });
        }
        let transport = std::sync::Arc::clone(&self.api.client.inner.transport);
        stream::iter(uploads.into_iter().map(|req| put_with_retries(transport.as_ref(), req)))
            .buffer_unordered(self.max_concurrency)
            .try_collect::<Vec<()>>()
            .await?;
        let finished = self.api.finish_upload(self.bus_id, &finish, self.credential.clone()).await?;
        match finished.objects {
            Some(objects) if !objects.is_empty() => Ok(objects),
            _ => Err(Error::api_data("FinishUpload 未返回上传成功的文件对象.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::*;
    use crate::transport::Response;
    use crate::transport::mock::MockTransport;

    #[test]
    fn cos_signature_matches_sdk() {
        // Vector generated with cos-python-sdk-v5 `CosS3Auth` (time patched).
        let auth = cos_authorization(
            "AKIDtest",
            "secretKEY",
            "PUT",
            "/music/a b.png",
            &[
                ("Content-Type", "application/octet-stream"),
                ("host", "bucket-1250000000.cos.ap-guangzhou.myqcloud.com"),
                ("x-cos-security-token", "tok/+="),
            ],
            1_700_000_000,
        );
        assert_eq!(
            auth,
            "q-sign-algorithm=sha1&q-ak=AKIDtest&q-sign-time=1699999940;1700003600&q-key-time=1699999940;1700003600\
             &q-header-list=content-type;host;x-cos-security-token&q-url-param-list=\
             &q-signature=fad09ab9d5631b96b353a50bd7b5e525d1dba29d"
        );
        assert_eq!(cos_quote("a b/ü~", true), "a%20b/%C3%BC~");
        assert_eq!(
            hmac_sha1_hex(b"key", b"The quick brown fox jumps over the lazy dog"),
            "de7c9b85b8b78aa6bc8a7a36f70a90701c9db4d9"
        );
        assert_eq!(
            hmac_sha1_hex(&[0xAA; 80], b"Test Using Larger Than Block-Size Key - Hash Key First"),
            "aa4ae5e15272d00e95705637ce8a3b55ed402112"
        );
        assert_eq!(UploadBusiness::AiPlAssistant.as_str(), "Aiplassistant");
    }

    #[tokio::test]
    async fn cgi_requests_are_signed() {
        let (client, mock) = logged_in_client();
        push_cgi(&mock, json!({"AuthInfo": auth_json("i", "k", "t"), "Files": []}));
        let file = InitUploadFile { file_sha1: "s".into(), file_name: "a".into(), file_size: 1 };
        client.helper().init_upload(UploadBusiness::Songlist, &[file], None).await.unwrap();
        let request = mock.last_request().unwrap();
        assert!(request.url.contains("musics.fcg"), "{}", request.url);
        let req = last_req0(&mock);
        assert_eq!(
            req["param"],
            json!({"BusID": "songlist", "Files": [{"FileSha1": "s", "FileName": "a", "FileSize": 1}]})
        );

        let (anon, mock2) = mock_client();
        assert!(anon.helper().finish_upload(UploadBusiness::Homepage, &[], None).await.is_err());
        assert_eq!(mock2.request_count(), 0);
    }

    fn auth_json(id: &str, key: &str, token: &str) -> Value {
        json!({"SecretID": id, "SecretKey": key, "Token": token, "StartTime": 0, "ExpiredTime": 0})
    }

    fn init_response(expired: i64, status: [i64; 2]) -> Value {
        json!({
            "AuthInfo": {"SecretID": "sid", "SecretKey": "skey", "Token": "tok", "StartTime": 0, "ExpiredTime": expired},
            "Files": [
                {"FileSha1": "a", "ObjectKey": "obj/a.png", "Buckets": [{"Bucket": {"Name": "b-1", "Region": "ap-gz"}, "UploadStatus": status[0]}]},
                {"FileSha1": "b", "ObjectKey": "obj/b.png", "Buckets": [{"Bucket": {"Name": "b-1", "Region": "ap-gz"}, "UploadStatus": status[1]}]}
            ]
        })
    }

    fn finish_response() -> Value {
        json!({"Objects": [{"Storage": {"Bucket": {"Name": "b-1", "Region": "ap-gz"}, "ObjectKey": "obj/a.png"}, "Url": {"URL": "u", "CDNURL": "c"}}]})
    }

    #[tokio::test(start_paused = true)]
    async fn upload_session_flow() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.png");
        let b = dir.path().join("b.png");
        std::fs::write(&a, b"aaa").unwrap();
        std::fs::write(&b, b"bb").unwrap();
        let (client, mock) = logged_in_client();
        let cos_attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempts = std::sync::Arc::clone(&cos_attempts);
        mock.route_url("myqcloud.com", move |req| {
            let n = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(req.method, Method::Put);
            assert!(req.header("authorization").unwrap().starts_with("q-sign-algorithm=sha1&q-ak=sid&"));
            assert_eq!(req.header("x-cos-security-token"), Some("tok"));
            Ok(Response::new(if n == 0 { 503 } else { 200 }, ""))
        });
        let far = now_secs() + 7200;
        let init = std::sync::Arc::new(std::sync::Mutex::new(init_response(far, [0, 1])));
        let finish = std::sync::Arc::new(std::sync::Mutex::new(finish_response()));
        let (init_data, finish_data) = (std::sync::Arc::clone(&init), std::sync::Arc::clone(&finish));
        mock.route_url("musics.fcg", move |req| {
            let body = req.json_body().unwrap();
            let data = match body["req_0"]["method"].as_str() {
                Some("InitUpload") => init_data.lock().unwrap().clone(),
                _ => finish_data.lock().unwrap().clone(),
            };
            Ok(Response::json(&json!({"code": 0, "req_0": {"code": 0, "data": data}})))
        });
        let calls = |method: &str| {
            mock.requests_to("musics.fcg")
                .iter()
                .filter(|r| r.json_body().unwrap()["req_0"]["method"] == method)
                .count()
        };
        let mut session = client.helper().upload_session(UploadBusiness::Songlist).max_concurrency(2);
        let objects = session.upload(&[&a, &b]).await.unwrap();
        assert_eq!(objects[0].url.cdn_url, "c");
        // Only the first file needed an upload; one 503 retry.
        assert_eq!(cos_attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
        let cos = mock.requests_to("myqcloud.com");
        assert_eq!(cos[0].url, "https://b-1.cos.ap-gz.myqcloud.com/obj/a.png");
        let Body::Raw { data, .. } = &cos[0].body else { panic!() };
        assert_eq!(data, b"aaa");
        let init_req = mock.requests_to("musics.fcg")[0].clone();
        let files = init_req.json_body().unwrap()["req_0"]["param"]["Files"].clone();
        assert_eq!(
            files[0],
            json!({"FileSha1": hex::encode(Sha1::digest(b"aaa")), "FileName": "a.png", "FileSize": 3})
        );
        let last = last_req0(&mock);
        assert_eq!(last["method"], "FinishUpload");
        assert_eq!(last["param"]["Results"].as_array().unwrap().len(), 2);
        assert_eq!((calls("InitUpload"), calls("FinishUpload")), (1, 1));

        // Same files and valid credentials: InitUpload is reused.
        session.upload(&[&a, &b]).await.unwrap();
        assert_eq!((calls("InitUpload"), calls("FinishUpload")), (1, 2));

        // Changed file -> new InitUpload; empty FinishUpload is an error.
        std::fs::write(&b, b"changed").unwrap();
        *init.lock().unwrap() = init_response(far, [1, 1]);
        *finish.lock().unwrap() = json!({"Objects": []});
        assert!(matches!(session.upload(&[&a, &b]).await, Err(Error::ApiData { .. })));
        assert_eq!((calls("InitUpload"), calls("FinishUpload")), (2, 3));
        assert_eq!(cos_attempts.load(std::sync::atomic::Ordering::SeqCst), 3, "already uploaded files are skipped");

        // Expiring credentials are refreshed.
        *init.lock().unwrap() = init_response(now_secs() + 100, [1, 1]);
        std::fs::write(&b, b"again").unwrap();
        let _ = session.upload(&[&a, &b]).await;
        let _ = session.upload(&[&a, &b]).await;
        assert_eq!(calls("InitUpload"), 4);
    }

    #[tokio::test]
    async fn upload_session_errors() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.png");
        std::fs::write(&a, b"aaa").unwrap();
        let (client, mock) = logged_in_client();
        let mut session = client.helper().upload_session(UploadBusiness::Homepage);
        assert!(matches!(session.upload::<&Path>(&[]).await, Err(Error::InvalidArgument(_))));
        assert!(matches!(session.upload(&[dir.path().join("missing")]).await, Err(Error::Io(_))));
        assert!(matches!(session.upload(&[dir.path()]).await, Err(Error::Io(_))));

        push_cgi(&mock, init_response(0, [0, 0]));
        assert!(matches!(session.upload(&[&a]).await, Err(Error::InvalidArgument(_))), "target count mismatch");

        let bucket = json!({"Bucket": {"Name": "b", "Region": "r"}, "UploadStatus": 0});
        push_cgi(
            &mock,
            json!({"AuthInfo": auth_json("i", "k", ""), "Files": [{"FileSha1": "s", "ObjectKey": "o", "Buckets": [bucket]}]}),
        );
        assert!(matches!(session.upload(&[&a]).await, Err(Error::ApiData { .. })), "incomplete credentials");
        push_cgi(
            &mock,
            json!({"AuthInfo": auth_json("i", "k", "t"), "Files": [{"FileSha1": "s", "ObjectKey": "o", "Buckets": []}]}),
        );
        assert!(matches!(session.upload(&[&a]).await, Err(Error::ApiData { .. })), "missing bucket");
        assert!(session.init_data().is_some());

        // Non retryable COS status.
        let mock3 = MockTransport::new();
        mock3.route_url("myqcloud.com", |_| Ok(Response::new(403, "denied")));
        let request = cos_put_request(&UploadAuthInfo::default(), "b", "r", "o", vec![], 0);
        let err = put_with_retries(&mock3, request).await.unwrap_err();
        assert!(matches!(err, Error::Http { status: 403, .. }));
        assert_eq!(mock3.request_count(), 1);
    }
}
