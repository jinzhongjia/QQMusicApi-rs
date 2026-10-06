//! File upload (`music.filesys.FileSystem`) models.

use serde::Serialize;
use serde_json::{Value, json};

use crate::FromJson;

/// File description sent to `InitUpload`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InitUploadFile {
    /// SHA-1 (hex).
    #[serde(rename = "FileSha1")]
    pub file_sha1: String,
    /// File name.
    #[serde(rename = "FileName")]
    pub file_name: String,
    /// Size in bytes.
    #[serde(rename = "FileSize")]
    pub file_size: u64,
}

/// Uploaded object reported to `FinishUpload`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishUploadResult {
    /// Bucket name.
    pub bucket: String,
    /// Bucket region.
    pub region: String,
    /// Object key.
    pub object_key: String,
    /// Upload result (`0` = success).
    pub upload_result: i64,
}

impl FinishUploadResult {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "Storage": {"Bucket": {"Name": self.bucket, "Region": self.region}, "ObjectKey": self.object_key},
            "UploadResult": self.upload_result,
        })
    }
}

/// COS bucket.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, FromJson)]
pub struct UploadBucketInfo {
    /// Name.
    #[json(alias = "Name")]
    pub name: String,
    /// Region.
    #[json(alias = "Region")]
    pub region: String,
}

/// Bucket plus upload state (`1` = already uploaded).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, FromJson)]
pub struct UploadBucketStatus {
    /// Bucket.
    #[json(alias = "Bucket")]
    pub bucket: UploadBucketInfo,
    /// Upload status.
    #[json(alias = "UploadStatus")]
    pub upload_status: i64,
}

/// Upload target of one file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, FromJson)]
pub struct UploadFileInfo {
    /// SHA-1.
    #[json(alias = "FileSha1")]
    pub file_sha1: String,
    /// Object key.
    #[json(alias = "ObjectKey")]
    pub object_key: String,
    /// Candidate buckets.
    #[json(alias = "Buckets")]
    pub buckets: Vec<UploadBucketStatus>,
}

/// Temporary COS credentials.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, FromJson)]
pub struct UploadAuthInfo {
    /// Secret id.
    #[json(alias = "SecretID")]
    pub secret_id: String,
    /// Secret key.
    #[json(alias = "SecretKey")]
    pub secret_key: String,
    /// Session token.
    #[json(alias = "Token")]
    pub token: String,
    /// Validity start (unix seconds).
    #[json(alias = "StartTime")]
    pub start_time: i64,
    /// Expiry (unix seconds).
    #[json(alias = "ExpiredTime")]
    pub expired_time: i64,
}

/// `InitUpload` response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, FromJson)]
pub struct InitUploadResponse {
    /// Credentials.
    #[json(alias = "AuthInfo")]
    pub auth_info: UploadAuthInfo,
    /// Targets (same order as the request).
    #[json(alias = "Files")]
    pub files: Vec<UploadFileInfo>,
}

/// Stored object.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, FromJson)]
pub struct UploadStorage {
    /// Bucket.
    #[json(alias = "Bucket")]
    pub bucket: UploadBucketInfo,
    /// Object key.
    #[json(alias = "ObjectKey")]
    pub object_key: String,
}

/// Access URLs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, FromJson)]
pub struct UploadUrlInfo {
    /// File id.
    #[json(alias = "FileId", default)]
    pub file_id: String,
    /// URL.
    #[json(alias = "URL")]
    pub url: String,
    /// CDN URL.
    #[json(alias = "CDNURL")]
    pub cdn_url: String,
    /// Presigned URL.
    #[json(alias = "PresignedURL", default)]
    pub presigned_url: String,
    /// Internal URL.
    #[json(alias = "InternalURL", default)]
    pub internal_url: String,
}

/// Uploaded object.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, FromJson)]
pub struct UploadObjectInfo {
    /// Storage location.
    #[json(alias = "Storage")]
    pub storage: UploadStorage,
    /// URLs.
    #[json(alias = "Url")]
    pub url: UploadUrlInfo,
}

/// `FinishUpload` response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, FromJson)]
#[json(default)]
pub struct FinishUploadResponse {
    /// Uploaded objects.
    #[json(alias = "Objects")]
    pub objects: Option<Vec<UploadObjectInfo>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_upload_models() {
        let init = InitUploadResponse::from_json(&json!({
            "AuthInfo": {"SecretID": "id", "SecretKey": "k", "Token": "t", "StartTime": 1, "ExpiredTime": 2},
            "Files": [{"FileSha1": "s", "ObjectKey": "o", "Buckets": [{"Bucket": {"Name": "b", "Region": "r"}, "UploadStatus": 1}]}]
        }))
        .unwrap();
        assert_eq!(init.auth_info.expired_time, 2);
        assert_eq!(init.files[0].buckets[0].bucket.region, "r");
        assert!(InitUploadResponse::from_json(&json!({"Files": []})).is_err(), "AuthInfo is required");

        let finish = FinishUploadResponse::from_json(&json!({
            "Objects": [{"Storage": {"Bucket": {"Name": "b", "Region": "r"}, "ObjectKey": "o"}, "Url": {"URL": "u", "CDNURL": "c"}}]
        }))
        .unwrap();
        let objects = finish.objects.unwrap();
        assert_eq!((objects[0].url.cdn_url.as_str(), objects[0].url.file_id.as_str()), ("c", ""));
        assert_eq!(FinishUploadResponse::from_json(&json!({})).unwrap().objects, None);

        let file = InitUploadFile { file_sha1: "s".into(), file_name: "a.png".into(), file_size: 3 };
        assert_eq!(serde_json::to_value(file).unwrap(), json!({"FileSha1": "s", "FileName": "a.png", "FileSize": 3}));
        let result =
            FinishUploadResult { bucket: "b".into(), region: "r".into(), object_key: "o".into(), upload_result: 0 };
        assert_eq!(result.to_json()["Storage"]["Bucket"]["Name"], "b");
    }
}
