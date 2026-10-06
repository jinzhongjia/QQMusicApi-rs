//! Simulated Android device identity.
//!
//! QQ Music's risk control fingerprints the client device (`OpenUDID`,
//! `aid`, QIMEI …). Keeping a *stable* identity across restarts is important
//! – a new device on every launch looks suspicious and also changes the
//! high-quality `ct` value derived by [`crate::bypass`]. The JSON layout is
//! compatible with the upstream Python library so device files can be
//! shared between both implementations.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use rand::rngs::StdRng;
use rand::{Rng, RngCore, SeedableRng};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::Mutex;

use crate::error::{Error, Result};
use crate::utils::now_secs;

/// Android OS version information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsVersion {
    /// Build incremental.
    pub incremental: String,
    /// Release (e.g. `15`).
    pub release: String,
    /// Codename.
    pub codename: String,
    /// SDK level.
    pub sdk: u32,
}

/// Built-in device hardware profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceProfile {
    /// vivo X200 (`V2408A`).
    Vivo,
    /// Xiaomi (`24129PN74C`).
    Xiaomi,
    /// OPPO (`PKB110`).
    Oppo,
}

impl DeviceProfile {
    /// All built-in profiles.
    pub const ALL: [Self; 3] = [Self::Vivo, Self::Xiaomi, Self::Oppo];

    /// Profile name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vivo => "vivo",
            Self::Xiaomi => "xiaomi",
            Self::Oppo => "oppo",
        }
    }
}

impl fmt::Display for DeviceProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DeviceProfile {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "vivo" => Ok(Self::Vivo),
            "xiaomi" => Ok(Self::Xiaomi),
            "oppo" => Ok(Self::Oppo),
            other => Err(Error::invalid_argument(format!(
                "未知设备档案: {other}. 可选值: vivo, xiaomi, oppo"
            ))),
        }
    }
}

struct ProfileData {
    display: &'static str,
    product: &'static str,
    device: &'static str,
    board: &'static str,
    model: &'static str,
    fingerprint: &'static str,
    proc_version: &'static str,
    brand: &'static str,
    manufacturer: &'static str,
    host: &'static str,
    first_api_level: u32,
    incremental: &'static str,
    release: &'static str,
    sdk: u32,
    vendor_name: &'static str,
    vendor_os_name: &'static str,
}

fn profile_data(profile: DeviceProfile) -> ProfileData {
    match profile {
        DeviceProfile::Vivo => ProfileData {
            display: "PD2408D_A_16.1.18.2.W10",
            product: "PD2408",
            device: "PD2408",
            board: "sun",
            model: "V2408A",
            fingerprint: "vivo/PD2408/PD2408:15/AP3A.240905.015.A2/compiler250423182036:user/release-keys",
            proc_version: "Linux localhost 6.6.89-android15-8-g1f71897ac249-abogki467805059-4k #1 SMP PREEMPT Thu Dec 11 01:56:00 UTC 2025 aarch64",
            brand: "vivo",
            manufacturer: "vivo",
            host: "comdg01150014",
            first_api_level: 35,
            incremental: "compiler250423182036",
            release: "15",
            sdk: 35,
            vendor_name: "OriginOS",
            vendor_os_name: "OriginOS 5.0",
        },
        DeviceProfile::Xiaomi => ProfileData {
            display: "OS3.0.260511.1.WOCCNXM.STABLE-OS31",
            product: "dada",
            device: "dada",
            board: "sun",
            model: "24129PN74C",
            fingerprint: "Xiaomi/dada/dada:16/BP2A.250605.031.A3/OS3.0.260511.1.WOCCNXM.STABLE-OS31:user/release-keys",
            proc_version: "",
            brand: "Xiaomi",
            manufacturer: "Xiaomi",
            host: "",
            first_api_level: 35,
            incremental: "OS3.0.260511.1.WOCCNXM.STABLE-OS31",
            release: "16",
            sdk: 36,
            vendor_name: "Xiaomi",
            vendor_os_name: "HyperOS 3.0",
        },
        DeviceProfile::Oppo => ProfileData {
            display: "V.1ab312a_1-2a261",
            product: "PKB110",
            device: "OP5A3DL1",
            board: "mt6991",
            model: "PKB110",
            fingerprint: "OPPO/PKB110/OP5A3DL1:15/AP3A.240617.008/V.1ab312a_1-2a261:user/release-keys",
            proc_version: "",
            brand: "OPPO",
            manufacturer: "OPPO",
            host: "",
            first_api_level: 35,
            incremental: "V.1ab312a_1-2a261",
            release: "15",
            sdk: 35,
            vendor_name: "ColorOS",
            vendor_os_name: "ColorOS 15",
        },
    }
}

/// Simulated Android device.
///
/// Every field is public so that callers can fully customise the identity
/// (e.g. to mimic an existing phone) – see [`crate::ClientBuilder::device`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct Device {
    pub display: String,
    pub product: String,
    pub device: String,
    pub board: String,
    pub model: String,
    pub fingerprint: String,
    pub proc_version: String,
    pub brand: String,
    pub manufacturer: String,
    pub host: String,
    pub first_api_level: u32,
    pub version: OsVersion,
    pub vendor_name: String,
    pub vendor_os_name: String,
    pub boot_id: String,
    pub imei: String,
    pub bootloader: String,
    pub base_band: String,
    pub sim_info: String,
    pub os_type: String,
    pub mac_address: String,
    pub wifi_bssid: String,
    pub wifi_ssid: String,
    pub imsi_md5: Vec<u8>,
    pub android_id: String,
    pub apn: String,
    pub open_udid: String,
    pub open_udid2: String,
}

/// Field names of [`Device`] (used to detect incomplete device files).
const DEVICE_FIELDS: [&str; 28] = [
    "display",
    "product",
    "device",
    "board",
    "model",
    "fingerprint",
    "proc_version",
    "brand",
    "manufacturer",
    "host",
    "first_api_level",
    "version",
    "vendor_name",
    "vendor_os_name",
    "boot_id",
    "imei",
    "bootloader",
    "base_band",
    "sim_info",
    "os_type",
    "mac_address",
    "wifi_bssid",
    "wifi_ssid",
    "imsi_md5",
    "android_id",
    "apn",
    "open_udid",
    "open_udid2",
];

/// Generate a Luhn-valid 15 digit IMEI.
pub fn random_imei(rng: &mut impl Rng) -> String {
    let digits: Vec<u32> = (0..14).map(|_| rng.random_range(0..=9)).collect();
    let sum: u32 = digits
        .iter()
        .enumerate()
        .map(|(idx, &digit)| {
            if idx % 2 == 1 {
                let doubled = digit * 2;
                if doubled > 9 { doubled - 9 } else { doubled }
            } else {
                digit
            }
        })
        .sum();
    let check = (10 - sum % 10) % 10;
    digits
        .iter()
        .chain(std::iter::once(&check))
        .map(|d| char::from_digit(*d, 10).unwrap_or('0'))
        .collect()
}

fn random_mac(rng: &mut impl RngCore) -> String {
    let mut octets = [0u8; 6];
    octets[0] = 2;
    rng.fill_bytes(&mut octets[1..]);
    octets
        .iter()
        .map(|o| format!("{o:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn random_uuid_v4(rng: &mut impl RngCore) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    rng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    bytes
}

fn uuid_hyphenated(bytes: &[u8; 16]) -> String {
    let hex = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

impl Device {
    /// Generate a random device.
    ///
    /// `profile = None` picks a random built-in profile. A `seed` makes the
    /// generation deterministic.
    pub fn generate(profile: Option<DeviceProfile>, seed: Option<u64>) -> Self {
        let mut rng = match seed {
            Some(seed) => StdRng::seed_from_u64(seed),
            None => StdRng::from_os_rng(),
        };
        let profile = profile.unwrap_or_else(|| {
            DeviceProfile::ALL[rng.random_range(0..DeviceProfile::ALL.len())]
        });
        let data = profile_data(profile);
        let boot_id = uuid_hyphenated(&random_uuid_v4(&mut rng));
        let imei = random_imei(&mut rng);
        let mac_address = random_mac(&mut rng);
        let wifi_bssid = random_mac(&mut rng);
        let mut imsi_md5 = vec![0u8; 16];
        rng.fill_bytes(&mut imsi_md5);
        let android_id = format!("{:016x}", rng.next_u64());
        let open_udid = hex::encode(random_uuid_v4(&mut rng));
        let open_udid2 = hex::encode(random_uuid_v4(&mut rng));
        Self {
            display: data.display.into(),
            product: data.product.into(),
            device: data.device.into(),
            board: data.board.into(),
            model: data.model.into(),
            fingerprint: data.fingerprint.into(),
            proc_version: data.proc_version.into(),
            brand: data.brand.into(),
            manufacturer: data.manufacturer.into(),
            host: data.host.into(),
            first_api_level: data.first_api_level,
            version: OsVersion {
                incremental: data.incremental.into(),
                release: data.release.into(),
                codename: "REL".into(),
                sdk: data.sdk,
            },
            vendor_name: data.vendor_name.into(),
            vendor_os_name: data.vendor_os_name.into(),
            boot_id,
            imei,
            bootloader: "U-boot".into(),
            base_band: String::new(),
            sim_info: "T-Mobile".into(),
            os_type: "android".into(),
            mac_address,
            wifi_bssid,
            wifi_ssid: "<unknown ssid>".into(),
            imsi_md5,
            android_id,
            apn: "wifi".into(),
            open_udid,
            open_udid2,
        }
    }

    /// Random device with a random profile.
    pub fn random() -> Self {
        Self::generate(None, None)
    }
}

/// Cached QIMEI device fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Qimei {
    /// 16 character QIMEI.
    pub q16: String,
    /// 36 character QIMEI.
    pub q36: String,
}

/// Cached Android session (`uid`/`sid`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Session uid.
    pub uid: String,
    /// Session sid.
    pub sid: String,
    /// Unix timestamp of the moment the session was obtained.
    pub saved_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct QimeiRecord {
    q16: String,
    q36: String,
    saved_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    qimei: Option<QimeiRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<SessionRecord>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// Write a file readable only by the owner (it contains identity data).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

/// Persistent cache for QIMEI and the Android session.
///
/// Stored next to the device file as `<stem>.cache.json`.
#[derive(Debug)]
pub struct DeviceCacheStore {
    path: Option<PathBuf>,
    data: Mutex<Option<CacheData>>,
}

impl DeviceCacheStore {
    /// Create a store backed by `path` (`None` keeps everything in memory).
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            data: Mutex::new(None),
        }
    }

    /// Cache path derived from the device path.
    pub fn path_for_device(device_path: &Path) -> PathBuf {
        let stem = device_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "device".to_string());
        device_path.with_file_name(format!("{stem}.cache.json"))
    }

    /// Cache file location.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn load(&self) -> CacheData {
        let Some(path) = &self.path else {
            return CacheData::default();
        };
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, data: &CacheData) {
        if let Some(path) = &self.path {
            match serde_json::to_vec(data) {
                Ok(bytes) => {
                    if let Err(err) = write_private(path, &bytes) {
                        tracing::warn!(path = %path.display(), error = %err, "failed to persist device cache");
                    }
                }
                Err(err) => tracing::warn!(error = %err, "failed to serialize device cache"),
            }
        }
    }

    /// Cached QIMEI with its timestamp.
    pub async fn qimei(&self) -> Option<(Qimei, i64)> {
        let mut guard = self.data.lock().await;
        let data = guard.get_or_insert_with(|| self.load());
        data.qimei
            .as_ref()
            .map(|r| (Qimei { q16: r.q16.clone(), q36: r.q36.clone() }, r.saved_at))
    }

    /// Store QIMEI.
    pub async fn set_qimei(&self, qimei: &Qimei, saved_at: i64) {
        let mut guard = self.data.lock().await;
        let data = guard.get_or_insert_with(|| self.load());
        data.qimei = Some(QimeiRecord {
            q16: qimei.q16.clone(),
            q36: qimei.q36.clone(),
            saved_at,
        });
        self.save(data);
    }

    /// Cached Android session.
    pub async fn session(&self) -> Option<SessionRecord> {
        let mut guard = self.data.lock().await;
        let data = guard.get_or_insert_with(|| self.load());
        data.session.clone()
    }

    /// Store the Android session.
    pub async fn set_session(&self, session: &SessionRecord) {
        let mut guard = self.data.lock().await;
        let data = guard.get_or_insert_with(|| self.load());
        data.session = Some(session.clone());
        self.save(data);
    }
}

/// Device persistence / provider.
#[derive(Debug)]
pub struct DeviceStore {
    path: Option<PathBuf>,
    device: Mutex<Option<Arc<Device>>>,
    cache: Arc<DeviceCacheStore>,
}

impl DeviceStore {
    /// In-memory random device (a new identity per process).
    pub fn ephemeral() -> Self {
        Self {
            path: None,
            device: Mutex::new(None),
            cache: Arc::new(DeviceCacheStore::new(None)),
        }
    }

    /// Device persisted in a JSON file (created on first use).
    pub fn file(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let cache = DeviceCacheStore::new(Some(DeviceCacheStore::path_for_device(&path)));
        Self {
            path: Some(path),
            device: Mutex::new(None),
            cache: Arc::new(cache),
        }
    }

    /// Fixed device supplied by the caller (never written to disk).
    ///
    /// `cache_path` optionally persists QIMEI/session data.
    pub fn fixed(device: Device, cache_path: Option<PathBuf>) -> Self {
        Self {
            path: None,
            device: Mutex::new(Some(Arc::new(device))),
            cache: Arc::new(DeviceCacheStore::new(cache_path)),
        }
    }

    /// Device file location.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Shared QIMEI/session cache.
    pub fn cache(&self) -> &Arc<DeviceCacheStore> {
        &self.cache
    }

    /// Get (and lazily load/generate) the device.
    pub async fn get(&self) -> Result<Arc<Device>> {
        let mut guard = self.device.lock().await;
        if let Some(device) = guard.as_ref() {
            return Ok(Arc::clone(device));
        }
        let device = match &self.path {
            None => Device::random(),
            Some(path) if !path.exists() => {
                let device = Device::random();
                save_device(&device, path)?;
                device
            }
            Some(path) => self.load(path).await?,
        };
        let device = Arc::new(device);
        *guard = Some(Arc::clone(&device));
        Ok(device)
    }

    /// Replace the device (persisted if the store is file backed).
    pub async fn set(&self, device: Device) -> Result<()> {
        if let Some(path) = &self.path {
            save_device(&device, path)?;
        }
        *self.device.lock().await = Some(Arc::new(device));
        Ok(())
    }

    async fn load(&self, path: &Path) -> Result<Device> {
        let bytes = std::fs::read(path)?;
        let raw: Map<String, Value> = serde_json::from_slice(&bytes).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid device file {}: {err}", path.display()),
            )
        })?;
        self.migrate_legacy(&raw).await;

        let mut merged = match serde_json::to_value(Device::random()) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        for field in DEVICE_FIELDS {
            if let Some(value) = raw.get(field) {
                merged.insert(field.to_string(), value.clone());
            }
        }
        let device: Device = serde_json::from_value(Value::Object(merged)).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid device file {}: {err}", path.display()),
            )
        })?;
        let complete = raw.len() == DEVICE_FIELDS.len()
            && DEVICE_FIELDS.iter().all(|field| raw.contains_key(*field));
        if !complete {
            save_device(&device, path)?;
        }
        Ok(device)
    }

    /// Move fields written by old upstream versions into the cache store.
    async fn migrate_legacy(&self, raw: &Map<String, Value>) {
        let text = |key: &str| raw.get(key).and_then(Value::as_str).filter(|s| !s.is_empty());
        let ts = |key: &str| raw.get(key).and_then(Value::as_i64).unwrap_or_else(now_secs);
        if let (Some(q16), Some(q36)) = (text("qimei"), text("qimei36"))
            && self.cache.qimei().await.is_none()
        {
            let qimei = Qimei {
                q16: q16.to_string(),
                q36: q36.to_string(),
            };
            self.cache.set_qimei(&qimei, ts("qimei_save_time")).await;
        }
        if let (Some(uid), Some(sid)) = (text("session_uid"), text("session_sid"))
            && self.cache.session().await.is_none()
        {
            let session = SessionRecord {
                uid: uid.to_string(),
                sid: sid.to_string(),
                saved_at: ts("session_save_time"),
            };
            self.cache.set_session(&session).await;
        }
    }
}

fn save_device(device: &Device, path: &Path) -> Result<()> {
    let bytes = serde_json::to_vec(device).map_err(|err| Error::api_data(err.to_string()))?;
    write_private(path, &bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imei_is_luhn_valid() {
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..50 {
            let imei = random_imei(&mut rng);
            assert_eq!(imei.len(), 15);
            let sum: u32 = imei
                .chars()
                .rev()
                .enumerate()
                .map(|(i, c)| {
                    let d = c.to_digit(10).unwrap();
                    if i % 2 == 1 {
                        let x = d * 2;
                        if x > 9 { x - 9 } else { x }
                    } else {
                        d
                    }
                })
                .sum();
            assert_eq!(sum % 10, 0, "{imei}");
        }
    }

    #[test]
    fn generation_is_deterministic_with_seed() {
        let a = Device::generate(Some(DeviceProfile::Xiaomi), Some(42));
        let b = Device::generate(Some(DeviceProfile::Xiaomi), Some(42));
        assert_eq!(a, b);
        assert_eq!(a.model, "24129PN74C");
        assert_eq!(a.version.sdk, 36);
        assert_eq!(a.open_udid.len(), 32);
        assert_eq!(&a.open_udid[12..13], "4", "uuid v4");
        assert_eq!(a.boot_id.len(), 36);
        assert_eq!(a.android_id.len(), 16);
        assert!(a.mac_address.starts_with("02:"));
        assert_eq!(a.imsi_md5.len(), 16);
        assert_ne!(a, Device::generate(Some(DeviceProfile::Xiaomi), Some(43)));
    }

    #[test]
    fn profile_parsing() {
        assert_eq!("VIVO".parse::<DeviceProfile>().unwrap(), DeviceProfile::Vivo);
        assert!("nokia".parse::<DeviceProfile>().is_err());
        assert_eq!(DeviceProfile::Oppo.to_string(), "oppo");
    }

    #[tokio::test]
    async fn file_store_persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.json");
        let first = DeviceStore::file(&path).get().await.unwrap();
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let second = DeviceStore::file(&path).get().await.unwrap();
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn file_store_completes_partial_files_and_migrates_legacy_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dev.json");
        std::fs::write(
            &path,
            r#"{"android_id": "abcdef0123456789", "model": "Custom",
                "qimei": "q16v", "qimei36": "q36v", "qimei_save_time": 100,
                "session_uid": "u", "session_sid": "s"}"#,
        )
        .unwrap();
        let store = DeviceStore::file(&path);
        let device = store.get().await.unwrap();
        assert_eq!(device.android_id, "abcdef0123456789");
        assert_eq!(device.model, "Custom");

        let rewritten: Map<String, Value> =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(rewritten.len(), DEVICE_FIELDS.len());
        assert!(!rewritten.contains_key("qimei"));

        let (qimei, saved_at) = store.cache().qimei().await.unwrap();
        assert_eq!(qimei.q36, "q36v");
        assert_eq!(saved_at, 100);
        assert_eq!(store.cache().session().await.unwrap().sid, "s");
        assert!(dir.path().join("dev.cache.json").exists());
    }

    #[tokio::test]
    async fn invalid_device_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(DeviceStore::file(&path).get().await.is_err());
        std::fs::write(&path, r#"{"first_api_level": "x"}"#).unwrap();
        assert!(DeviceStore::file(&path).get().await.is_err());
    }

    #[tokio::test]
    async fn fixed_and_ephemeral_stores() {
        let device = Device::generate(Some(DeviceProfile::Oppo), Some(7));
        let store = DeviceStore::fixed(device.clone(), None);
        assert_eq!(*store.get().await.unwrap(), device);
        let replacement = Device::generate(Some(DeviceProfile::Vivo), Some(8));
        store.set(replacement.clone()).await.unwrap();
        assert_eq!(*store.get().await.unwrap(), replacement);

        let eph = DeviceStore::ephemeral();
        assert_eq!(eph.get().await.unwrap(), eph.get().await.unwrap());
    }

    #[tokio::test]
    async fn cache_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.cache.json");
        let store = DeviceCacheStore::new(Some(path.clone()));
        assert!(store.qimei().await.is_none());
        store
            .set_qimei(&Qimei { q16: "a".into(), q36: "b".into() }, 5)
            .await;
        store
            .set_session(&SessionRecord { uid: "u".into(), sid: "s".into(), saved_at: 6 })
            .await;
        let reloaded = DeviceCacheStore::new(Some(path));
        assert_eq!(reloaded.qimei().await.unwrap().1, 5);
        assert_eq!(reloaded.session().await.unwrap().saved_at, 6);
        assert_eq!(
            DeviceCacheStore::path_for_device(Path::new("/a/b/device.json")),
            PathBuf::from("/a/b/device.cache.json")
        );
    }
}
