//! Kōrero 1.49 (D-014): Kōrero's own Ollama, so local clean-up works without
//! the user installing, starting or repairing anything.
//!
//! - **What runs:** Ollama's official Windows build, pinned to one version
//!   (`manifest.rs`), in `<local app data>\local-ai\runtime\ollama-<version>`,
//!   started as Kōrero's child process (`ollama serve`) on a private loopback
//!   port (11435 unless taken) with its own models folder. An Ollama the user
//!   installed (on 11434, models in `%USERPROFILE%\.ollama`) is never touched.
//! - **What downloads:** only the parts this computer can use (`plan_for`): the
//!   core and Vulkan (about 40 MB), plus one CUDA set on an NVIDIA card. Each
//!   file is taken out of the official zip by HTTP range request and checked
//!   against its pinned SHA-256 before it replaces anything.
//! - **Local app data, not roaming:** the runtime and models are GBs, and a
//!   roaming profile on a managed laptop would try to sync them.
//! - **Lifetime:** on Windows the child is in a job object that closes with
//!   Kōrero, so a crash cannot leave it running.

pub mod manifest;
mod zip;

use futures_util::StreamExt;
use manifest::{manifest, Entry, RuntimeGroup};
use once_cell::sync::{Lazy, OnceCell};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

pub const PREFERRED_PORT: u16 = 11435;
const PORT_SPAN: u16 = 10;
const START_WAIT: Duration = Duration::from_secs(20);
const DOWNLOAD_CONNECT: Duration = Duration::from_secs(15);
/// A stalled download is given up after this long without a byte.
const DOWNLOAD_IDLE: Duration = Duration::from_secs(60);
const ATTEMPTS: u32 = 3;
/// Space kept free beyond the runtime itself.
const DISK_MARGIN: u64 = 512 * 1024 * 1024;
pub const PROGRESS_EVENT: &str = "local-ai-runtime-progress";
pub const CANCELLED: &str = "Download cancelled.";

static APP: OnceCell<AppHandle> = OnceCell::new();
static INSTALLING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);
static CHILD: Lazy<Mutex<Option<Running>>> = Lazy::new(|| Mutex::new(None));

/// The runtime Kōrero started: the process, the folder it runs from, and
/// (on Windows) the job object holding it and the model runners it starts.
struct Running {
    child: std::process::Child,
    dir: PathBuf,
    #[cfg(windows)]
    _job: Option<job::Job>,
}
static START_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

/// Called once at startup so the runtime can find its folders and settings.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

// ------------------------------------------------------------------ folders

fn root() -> Option<PathBuf> {
    if let Some(d) = crate::portable::data_dir() {
        return Some(d.join("local-ai"));
    }
    use tauri::Manager;
    APP.get()?
        .path()
        .app_local_data_dir()
        .ok()
        .map(|d| d.join("local-ai"))
}

fn runtime_dir(root: &Path) -> PathBuf {
    root.join("runtime")
        .join(format!("ollama-{}", manifest().version))
}

/// A runtime from an earlier Kōrero (another pinned version) that was
/// installed in full: used until this version's is downloaded, so a Kōrero
/// update never stops clean-up (review M1, 17 Nov 2026).
fn old_runtime(root: &Path) -> Option<PathBuf> {
    let current = runtime_dir(root);
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(root.join("runtime"))
        .ok()?
        .flatten()
        .map(|d| d.path())
        .filter(|p| p != &current && p.is_dir())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("ollama-"))
        })
        .filter(|p| {
            std::fs::read_to_string(p.join("korero-installed.json"))
                .ok()
                .and_then(|s| serde_json::from_str::<Vec<RuntimeGroup>>(&s).ok())
                .is_some_and(|g| g.contains(&RuntimeGroup::Core))
                && p.join(exe_name()).is_file()
        })
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    found.sort();
    found.pop().map(|(_, p)| p)
}

/// Delete runtimes of other versions, except the one running now.
fn remove_old_runtimes(root: &Path) {
    let current = runtime_dir(root);
    let running = running_dir();
    for p in std::fs::read_dir(root.join("runtime"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|d| d.path())
    {
        if p == current || Some(&p) == running.as_ref() || !p.is_dir() {
            continue;
        }
        match std::fs::remove_dir_all(&p) {
            Ok(()) => log::info!("local AI runtime: removed an older version's folder"),
            Err(e) => log::warn!("local AI runtime: couldn't remove an older version: {e}"),
        }
    }
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    }
}

/// Total size of the files under `dir`, in bytes.
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

fn models_dir(root: &Path) -> PathBuf {
    root.join("models")
}

fn marker_path(root: &Path) -> PathBuf {
    runtime_dir(root).join("korero-installed.json")
}

fn state_path(root: &Path) -> PathBuf {
    root.join("state.json")
}

// -------------------------------------------------------------------- state

/// Whether clean-up was pointed at Kōrero's own runtime, on which port, and
/// what clean-up used before (put back by "Stop using").
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct State {
    managed: bool,
    port: Option<u16>,
    previous: Option<Previous>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Previous {
    provider_id: String,
    ollama_base_url: String,
}

/// Clean-up uses Kōrero's runtime now: it was pointed at it, the active
/// provider is still Ollama, and the address is still Kōrero's port. A user
/// who switched provider or edited the address has left it, whatever
/// `state.managed` says (review M2, M9).
fn in_use_with(state: &State, s: &crate::settings::AppSettings) -> bool {
    state.managed
        && s.post_process_provider_id == "ollama"
        && s.post_process_providers
            .iter()
            .find(|p| p.id == "ollama")
            .is_some_and(|p| {
                crate::commands::history::is_loopback_url(&p.base_url)
                    && port_of(&p.base_url) == Some(state.port.unwrap_or(PREFERRED_PORT))
            })
}

fn in_use_now(root: &Path) -> bool {
    APP.get()
        .is_some_and(|app| in_use_with(&read_state(root), &crate::settings::get_settings(app)))
}

fn read_state(root: &Path) -> State {
    std::fs::read_to_string(state_path(root))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("tmp");
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| format!("Couldn't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't save {}: {e}", path.display()))
}

/// Groups whose files are all present at their pinned sizes. Cheap (no
/// hashing): every file was hashed when it was written, and "Repair" hashes
/// them all again.
fn installed_groups(root: &Path) -> Vec<RuntimeGroup> {
    let listed: Vec<RuntimeGroup> = std::fs::read_to_string(marker_path(root))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let dir = runtime_dir(root);
    listed
        .into_iter()
        .filter(|g| {
            manifest().files(std::slice::from_ref(g)).all(|e| {
                manifest::safe_relative(&e.path)
                    .and_then(|rel| std::fs::metadata(dir.join(rel)).ok())
                    .is_some_and(|m| m.len() == e.size)
            })
        })
        .collect()
}

// ------------------------------------------------------------- machine plan

/// The NVIDIA driver version from `nvidia-smi`, which ships with the driver.
fn nvidia_driver() -> Option<String> {
    #[cfg(windows)]
    let exe = PathBuf::from(std::env::var_os("SystemRoot")?)
        .join("System32")
        .join("nvidia-smi.exe");
    #[cfg(not(windows))]
    let exe = PathBuf::from("/usr/bin/nvidia-smi");
    if !exe.exists() {
        return None;
    }
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["--query-gpu=driver_version", "--format=csv,noheader"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > Duration::from_secs(3) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    }
    let out = child.wait_with_output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RuntimePlan {
    pub version: String,
    pub groups: Vec<RuntimeGroup>,
    pub download_mb: u32,
    pub unpacked_mb: u32,
    pub nvidia: bool,
    pub nvidia_driver: Option<String>,
}

fn mb(bytes: u64) -> u32 {
    bytes.div_ceil(1024 * 1024).min(u64::from(u32::MAX)) as u32
}

fn plan_now() -> (RuntimePlan, Option<u32>) {
    let profile = crate::commands::machine::profile_now();
    let nvidia = profile.gpus.iter().any(|g| {
        g.kind == crate::commands::machine::GpuKind::Discrete
            && g.name.to_ascii_uppercase().contains("NVIDIA")
    });
    let driver = if nvidia { nvidia_driver() } else { None };
    let groups = manifest::plan_for(nvidia, driver.as_deref());
    let (d, u) = manifest().sizes(&groups);
    let free = root().and_then(|r| crate::commands::machine::free_disk_mb(&r));
    (
        RuntimePlan {
            version: manifest().version.clone(),
            groups,
            download_mb: mb(d),
            unpacked_mb: mb(u),
            nvidia,
            nvidia_driver: driver,
        },
        free,
    )
}

// ----------------------------------------------------------------- download

#[derive(Debug, Clone, Serialize, Type)]
pub struct RuntimeProgress {
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub file: String,
}

#[derive(Debug)]
enum FetchError {
    Cancelled,
    /// Worth another try (network, an expired download link, a bad transfer).
    Retry(String),
    /// Retrying will not help.
    Fatal(String),
}

impl From<FetchError> for String {
    fn from(e: FetchError) -> String {
        match e {
            FetchError::Cancelled => CANCELLED.to_string(),
            FetchError::Retry(m) | FetchError::Fatal(m) => m,
        }
    }
}

fn net(e: reqwest::Error) -> FetchError {
    FetchError::Retry(format!("The download stopped: {e}"))
}

/// GitHub answers the release URL with a short-lived signed link; follow it
/// once and use the link for the range requests. Returns (link, file size).
async fn resolve(client: &reqwest::Client, url: &str) -> Result<(String, u64), FetchError> {
    let r = client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .await
        .map_err(net)?;
    if r.status().as_u16() != 206 {
        return Err(FetchError::Retry(format!(
            "The download server answered HTTP {} instead of part of the file.",
            r.status().as_u16()
        )));
    }
    let len = r
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(|| {
            FetchError::Retry("The download server didn't say how big the file is.".into())
        })?;
    Ok((r.url().to_string(), len))
}

async fn range(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    len: u64,
) -> Result<reqwest::Response, FetchError> {
    let end = start + len.max(1) - 1;
    let want = format!("bytes {start}-{end}/");
    let r = client
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
        .send()
        .await
        .map_err(net)?;
    let ok = r.status().as_u16() == 206
        && r.headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with(&want));
    if !ok {
        return Err(FetchError::Retry(format!(
            "The download server answered HTTP {} to a part request.",
            r.status().as_u16()
        )));
    }
    Ok(r)
}

async fn range_bytes(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    len: u64,
) -> Result<Vec<u8>, FetchError> {
    let bytes = range(client, url, start, len)
        .await?
        .bytes()
        .await
        .map_err(net)?;
    if bytes.len() as u64 != len {
        return Err(FetchError::Retry(
            "The download server sent the wrong amount of a part.".into(),
        ));
    }
    Ok(bytes.to_vec())
}

/// Writes to a file and hashes what it writes.
struct HashFile {
    file: std::fs::File,
    hash: Sha256,
    len: u64,
    /// The pinned size: anything bigger is refused before it reaches disk.
    limit: u64,
}

impl Write for HashFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.len + buf.len() as u64 > self.limit {
            return Err(std::io::Error::other("bigger than the pinned file"));
        }
        self.file.write_all(buf)?;
        self.hash.update(buf);
        self.len += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).ok()?;
    Some(to_hex(&h.finalize()))
}

/// The file is there with its pinned size and hash.
fn file_ok(dir: &Path, e: &Entry) -> bool {
    let Some(rel) = manifest::safe_relative(&e.path) else {
        return false;
    };
    let p = dir.join(rel);
    std::fs::metadata(&p).is_ok_and(|m| m.len() == e.size)
        && sha256_file(&p).is_some_and(|h| h == e.sha256)
}

/// One file out of the remote zip, inflated, hashed and moved into place.
async fn fetch_file(
    client: &reqwest::Client,
    url: &str,
    zip_len: u64,
    at: &zip::CentralEntry,
    e: &Entry,
    dir: &Path,
    on_bytes: &mut (dyn FnMut(u64) + Send),
) -> Result<(), FetchError> {
    let rel = manifest::safe_relative(&e.path).ok_or_else(|| {
        FetchError::Fatal(format!("Refused an unsafe path in the runtime: {}", e.path))
    })?;
    let left = zip_len.checked_sub(at.header_offset).filter(|l| *l > 0).ok_or_else(|| {
        FetchError::Fatal("The runtime zip's directory points outside the file.".into())
    })?;
    let head_len = zip::LOCAL_HEADER_READ.min(left);
    let head = range_bytes(client, url, at.header_offset, head_len).await?;
    let start = at.header_offset + zip::local_header_len(&head).map_err(FetchError::Fatal)?;
    let resp = range(client, url, start, at.compressed_size).await?;

    let dest = dir.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|x| FetchError::Fatal(format!("Couldn't create {}: {x}", parent.display())))?;
    }
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let file = std::fs::File::create(&part)
        .map_err(|x| FetchError::Fatal(format!("Couldn't write {}: {x}", part.display())))?;
    let sink = HashFile {
        file,
        hash: Sha256::new(),
        len: 0,
        limit: e.size,
    };
    let write_err =
        |x: std::io::Error| FetchError::Fatal(format!("Couldn't write {}: {x}", part.display()));

    let mut stream = resp.bytes_stream();
    let outcome: Result<HashFile, FetchError> = async {
        match at.method {
            0 => {
                let mut sink = sink;
                while let Some(chunk) = stream.next().await {
                    if CANCEL.load(Ordering::SeqCst) {
                        return Err(FetchError::Cancelled);
                    }
                    let chunk = chunk.map_err(net)?;
                    sink.write_all(&chunk).map_err(write_err)?;
                    on_bytes(chunk.len() as u64);
                }
                Ok(sink)
            }
            8 => {
                let mut dec = flate2::write::DeflateDecoder::new(sink);
                while let Some(chunk) = stream.next().await {
                    if CANCEL.load(Ordering::SeqCst) {
                        return Err(FetchError::Cancelled);
                    }
                    let chunk = chunk.map_err(net)?;
                    dec.write_all(&chunk).map_err(|x| {
                        FetchError::Retry(format!("A runtime file arrived damaged: {x}"))
                    })?;
                    on_bytes(chunk.len() as u64);
                }
                dec.finish()
                    .map_err(|x| FetchError::Retry(format!("A runtime file arrived damaged: {x}")))
            }
            m => Err(FetchError::Fatal(format!(
                "The runtime zip uses compression {m}, which Kōrero does not read."
            ))),
        }
    }
    .await;

    let sink = match outcome {
        Ok(s) => s,
        Err(x) => {
            let _ = std::fs::remove_file(&part);
            return Err(x);
        }
    };
    let HashFile { file, hash, len, .. } = sink;
    drop(file);
    let got = to_hex(&hash.finalize());
    if len != e.size || got != e.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(FetchError::Retry(format!(
            "{} did not match the pinned release (size {len} of {}), so it was not used.",
            e.path, e.size
        )));
    }
    std::fs::rename(&part, &dest).map_err(|x| {
        let _ = std::fs::remove_file(&part);
        FetchError::Fatal(format!("Couldn't put {} in place: {x}", dest.display()))
    })
}

/// Download `wanted` from the zip at `url` into `dir`, skipping files that
/// are already right. Every file is checked against its pinned hash.
async fn download(
    url: &str,
    zip_size: u64,
    wanted: &[&Entry],
    dir: &Path,
    progress: &mut (dyn FnMut(RuntimeProgress) + Send),
) -> Result<(), String> {
    // Hashing what is already there reads up to a GB: off the async threads.
    let owned: Vec<Entry> = wanted.iter().map(|e| (*e).clone()).collect();
    let d = dir.to_path_buf();
    let ok: Vec<bool> = tauri::async_runtime::spawn_blocking(move || {
        owned.iter().map(|e| file_ok(&d, e)).collect()
    })
    .await
    .map_err(|e| e.to_string())?;
    let need: Vec<&Entry> = wanted
        .iter()
        .copied()
        .zip(ok)
        .filter(|(_, ok)| !ok)
        .map(|(e, _)| e)
        .collect();
    if need.is_empty() {
        return Ok(());
    }
    let client = crate::llm_client::direct_to_loopback(reqwest::Client::builder(), url)
        .connect_timeout(DOWNLOAD_CONNECT)
        .read_timeout(DOWNLOAD_IDLE)
        .build()
        .map_err(|e| format!("Couldn't start the download: {e}"))?;

    let (mut link, len) = resolve(&client, url).await?;
    if len != zip_size {
        return Err("The download server sent a different file from the release Kōrero expects, so nothing was used.".into());
    }
    let tail_start = len.saturating_sub(zip::TAIL_LEN);
    let tail = range_bytes(&client, &link, tail_start, len - tail_start).await?;
    let (cd_off, cd_len) = zip::central_directory_range(&tail, len)?;
    let directory =
        zip::parse_central_directory(&range_bytes(&client, &link, cd_off, cd_len).await?)?;

    let total: u64 = need.iter().map(|e| e.compressed_size).sum();
    let mut done = 0u64;
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    for e in need {
        let at = directory
            .iter()
            .find(|c| c.name == e.path)
            .filter(|c| {
                c.crc32 == e.crc32 && c.size == e.size && c.compressed_size == e.compressed_size
            })
            .ok_or_else(|| {
                format!(
                    "The runtime download doesn't match the pinned release ({}).",
                    e.path
                )
            })?;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let before = done;
            let mut on_bytes = |n: u64| {
                done += n;
                if last_emit.elapsed() >= Duration::from_millis(250) {
                    last_emit = Instant::now();
                    progress(RuntimeProgress {
                        done_bytes: done,
                        total_bytes: total,
                        file: e.path.clone(),
                    });
                }
            };
            match fetch_file(&client, &link, len, at, e, dir, &mut on_bytes).await {
                Ok(()) => break,
                Err(FetchError::Retry(m)) if attempt < ATTEMPTS => {
                    log::warn!("local AI runtime: {m} (try {attempt} of {ATTEMPTS})");
                    done = before;
                    tokio::time::sleep(Duration::from_secs(u64::from(attempt) * 2)).await;
                    // The signed link may have expired; ask for a fresh one.
                    if let Ok((fresh, l)) = resolve(&client, url).await {
                        if l == len {
                            link = fresh;
                        }
                    }
                }
                Err(x) => return Err(x.into()),
            }
        }
    }
    progress(RuntimeProgress {
        done_bytes: total,
        total_bytes: total,
        file: String::new(),
    });
    Ok(())
}

/// Download and check the groups this computer needs (or `groups`).
async fn install(app: &AppHandle, groups: &[RuntimeGroup]) -> Result<(), String> {
    let root = root().ok_or("Kōrero couldn't find its data folder.")?;
    let dir = runtime_dir(&root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    let m = manifest();
    let wanted: Vec<&Entry> = m.files(groups).collect();
    let unpacked: u64 = wanted.iter().map(|e| e.size).sum();
    if let Some(free) = crate::commands::machine::free_disk_mb(&root) {
        if u64::from(free) * 1024 * 1024 < unpacked + DISK_MARGIN {
            return Err(format!(
                "Kōrero's local AI needs about {} GB free on this drive, and {:.1} GB is free.",
                (unpacked + DISK_MARGIN).div_ceil(1024 * 1024 * 1024),
                f64::from(free) / 1024.0
            ));
        }
    }
    // The core first, and try running it, so a computer that blocks
    // programs in this folder finds out after 28 MB, not 900 (review M8).
    let (core, rest): (Vec<&Entry>, Vec<&Entry>) = wanted
        .iter()
        .copied()
        .partition(|e| e.group == RuntimeGroup::Core);
    let core_bytes: u64 = core.iter().map(|e| e.compressed_size).sum();
    let all_bytes: u64 = wanted.iter().map(|e| e.compressed_size).sum();
    let handle = app.clone();
    let mut emit_core = |p: RuntimeProgress| {
        let _ = handle.emit(
            PROGRESS_EVENT,
            &RuntimeProgress {
                total_bytes: all_bytes,
                ..p
            },
        );
    };
    download(&m.url, m.zip_size, &core, &dir, &mut emit_core).await?;
    #[cfg(windows)]
    if !rest.is_empty() {
        let d = dir.clone();
        tauri::async_runtime::spawn_blocking(move || try_run(&d))
            .await
            .map_err(|e| e.to_string())??;
    }
    let handle = app.clone();
    let mut emit_rest = |p: RuntimeProgress| {
        let _ = handle.emit(
            PROGRESS_EVENT,
            &RuntimeProgress {
                done_bytes: core_bytes + p.done_bytes,
                total_bytes: all_bytes,
                file: p.file,
            },
        );
    };
    download(&m.url, m.zip_size, &rest, &dir, &mut emit_rest).await?;
    let mut done = installed_groups(&root);
    for g in groups {
        if !done.contains(g) {
            done.push(*g);
        }
    }
    done.sort();
    write_json(&marker_path(&root), &done)?;
    log::info!(
        "local AI runtime {}: installed {:?} (from the release zip with SHA-256 {})",
        m.version,
        done,
        m.zip_sha256
    );
    Ok(())
}

// ------------------------------------------------------------------ process

#[cfg(windows)]
mod job {
    //! One job object per runtime start. It holds `ollama serve` and the
    //! model runners it starts; closing it (on "Stop", or when Kōrero exits
    //! or crashes) ends all of them, so no runner keeps graphics memory after
    //! the runtime stops (review M7).
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct Job(HANDLE);
    // SAFETY: a job handle is a kernel handle, usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new() -> Option<Job> {
            // SAFETY: plain Win32 calls with a correctly sized struct.
            unsafe {
                let h = CreateJobObjectW(None, windows::core::PCWSTR::null()).ok()?;
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .is_err()
                {
                    let _ = CloseHandle(h);
                    return None;
                }
                Some(Job(h))
            }
        }

        pub fn assign(&self, child: &std::process::Child) -> bool {
            use std::os::windows::io::AsRawHandle;
            // SAFETY: the child's handle is valid while `child` lives.
            match unsafe { AssignProcessToJobObject(self.0, HANDLE(child.as_raw_handle())) } {
                Ok(()) => true,
                Err(e) => {
                    log::warn!("local AI runtime: couldn't tie the runtime to Kōrero: {e}");
                    false
                }
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle is ours and closed exactly once, here.
            unsafe {
                let _ = TerminateJobObject(self.0, 0);
                let _ = CloseHandle(self.0);
            }
        }
    }
}

fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn child_alive() -> bool {
    let mut g = CHILD.lock().unwrap_or_else(|p| p.into_inner());
    match g.as_mut() {
        Some(r) => matches!(r.child.try_wait(), Ok(None)),
        None => false,
    }
}

/// The folder the running runtime was started from.
fn running_dir() -> Option<PathBuf> {
    CHILD
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|r| r.dir.clone())
}

async fn reachable(port: u16) -> bool {
    crate::commands::ollama::check_ollama_connection(format!("http://127.0.0.1:{port}")).await
}

/// Plain words for a runtime that couldn't start. Never the folder path (it
/// holds the user name), and a policy block says so instead of a code (M8).
fn start_error(e: &std::io::Error) -> String {
    // 5 access denied, 225 blocked as unsafe, 1260 blocked by group policy
    // (AppLocker, software restriction), 4551 blocked by WDAC.
    if matches!(e.raw_os_error(), Some(5 | 225 | 1260 | 4551))
        || e.kind() == std::io::ErrorKind::PermissionDenied
    {
        "This computer's security settings don't let programs run from Kōrero's folder (common on work laptops). Ask your IT team to allow Kōrero's local AI, or install Ollama yourself instead.".to_string()
    } else {
        format!("Couldn't start Kōrero's local AI: {e}")
    }
}

/// Run `ollama --version` from `dir` once, to find a policy block before a
/// large download rather than after it. Windows only.
#[cfg(windows)]
fn try_run(dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new(dir.join(exe_name()));
    cmd.arg("--version")
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("OLLAMA_")
        {
            cmd.env_remove(k);
        }
    }
    let mut child = cmd.spawn().map_err(|e| start_error(&e))?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if let Ok(Some(_)) = child.try_wait() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(()) // it ran; slow to exit is not a block
}

fn spawn(root: &Path, dir: &Path, port: u16) -> Result<(), String> {
    let exe = dir.join(exe_name());
    let models = models_dir(root);
    std::fs::create_dir_all(&models)
        .map_err(|e| format!("Couldn't create Kōrero's models folder: {e}"))?;
    let log_dir = APP
        .get()
        .and_then(|a| crate::portable::app_log_dir(a).ok())
        .unwrap_or_else(|| root.to_path_buf());
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("ollama-runtime.log");
    if log_path.exists() {
        let _ = std::fs::rename(&log_path, log_dir.join("ollama-runtime.1.log"));
    }
    let log = std::fs::File::create(&log_path)
        .map_err(|e| format!("Couldn't write the local AI's log: {e}"))?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;

    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("serve").current_dir(dir);
    // The user's own Ollama settings must not leak in (a different port, a
    // models folder on a network drive, listening on every address).
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("OLLAMA_")
        {
            cmd.env_remove(k);
        }
    }
    cmd.env("OLLAMA_HOST", format!("127.0.0.1:{port}"))
        .env("OLLAMA_MODELS", &models)
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(log_err);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let child = cmd.spawn().map_err(|e| start_error(&e))?;
    #[cfg(windows)]
    let job = {
        let j = job::Job::new();
        match &j {
            Some(j) => {
                j.assign(&child);
            }
            None => log::warn!("local AI runtime: no job object; the runtime may outlive a crash"),
        }
        j
    };
    log::info!(
        "local AI runtime: started pid {} on 127.0.0.1:{port}",
        child.id()
    );
    *CHILD.lock().unwrap_or_else(|p| p.into_inner()) = Some(Running {
        child,
        dir: dir.to_path_buf(),
        #[cfg(windows)]
        _job: job,
    });
    Ok(())
}

/// Stop Kōrero's runtime and the model runners it started. Never touches
/// another Ollama.
pub fn stop() {
    if let Some(mut r) = CHILD.lock().unwrap_or_else(|p| p.into_inner()).take() {
        let _ = r.child.kill();
        let _ = r.child.wait();
        // Dropping `r` closes its job object, which ends the runners too.
        drop(r);
        log::info!("local AI runtime: stopped");
    }
}

/// Start Kōrero's runtime (or confirm it is up). Returns its port.
pub async fn start() -> Result<u16, String> {
    let _guard = START_LOCK.lock().await;
    start_locked().await
}

/// `start` for callers that already hold `START_LOCK`.
async fn start_locked() -> Result<u16, String> {
    if !cfg!(windows) {
        return Err("Kōrero's own local AI runs on Windows for now.".into());
    }
    let root = root().ok_or("Kōrero couldn't find its data folder.")?;
    let current = runtime_dir(&root);
    let dir = if installed_groups(&root).contains(&RuntimeGroup::Core) {
        current.clone()
    } else if let Some(old) = old_runtime(&root) {
        log::info!("local AI runtime: this version isn't downloaded yet; using the earlier one");
        old
    } else {
        return Err("Kōrero's local AI isn't downloaded yet.".into());
    };
    let mut state = read_state(&root);
    let was_in_use = in_use_now(&root);
    let mut port = state.port.unwrap_or(PREFERRED_PORT);
    // Check our own process first: something else answering on the port is
    // never taken for Kōrero's runtime (review M4).
    if child_alive() {
        if running_dir().as_ref() == Some(&dir) && reachable(port).await {
            return Ok(port);
        }
        stop(); // not answering, or an older version now replaced: start again
    }
    if INSTALLING.load(Ordering::SeqCst) {
        return Err("Kōrero's local AI is still downloading. Clean-up works again when it finishes.".into());
    }
    if !port_free(port) {
        port = (PREFERRED_PORT..PREFERRED_PORT + PORT_SPAN)
            .find(|p| port_free(*p))
            .ok_or("Every port Kōrero's local AI can use (11435 to 11444) is taken by another program.")?;
    }
    spawn(&root, &dir, port)?;
    let started = Instant::now();
    while started.elapsed() < START_WAIT {
        if !child_alive() {
            return Err("Kōrero's local AI stopped as soon as it started. Its log is ollama-runtime.log in Kōrero's logs folder.".into());
        }
        if reachable(port).await {
            if state.port != Some(port) {
                state.port = Some(port);
                write_json(&state_path(&root), &state)?;
                // Follow the port only if clean-up was using the runtime;
                // never switch the provider from here.
                if was_in_use {
                    point_cleanup_at(port, false)?;
                }
            }
            return Ok(port);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    stop();
    Err(format!(
        "Kōrero's local AI didn't answer within {} s.",
        START_WAIT.as_secs()
    ))
}

/// Before a clean-up is sent to Kōrero's runtime: if it isn't Kōrero's own
/// process that would answer (it stopped, or never started this session),
/// start it, so text never goes to another program holding the port.
pub async fn ensure_ours_running(base_url: &str) -> Result<(), String> {
    if !is_ours(base_url) || child_alive() {
        return Ok(());
    }
    start().await.map(|_| ())
}

/// `base_url` is Kōrero's runtime: clean-up was pointed at it and the
/// address is its port on this computer.
pub fn is_ours(base_url: &str) -> bool {
    root().is_some_and(|root| {
        let state = read_state(&root);
        state.managed
            && crate::commands::history::is_loopback_url(base_url)
            && port_of(base_url) == Some(state.port.unwrap_or(PREFERRED_PORT))
    })
}

/// The port in an address such as `http://127.0.0.1:11435/v1`.
fn port_of(base_url: &str) -> Option<u16> {
    let rest = base_url
        .trim()
        .split_once("://")
        .map_or(base_url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    authority.rsplit_once(':')?.1.parse().ok()
}

/// For `commands::ollama::start_and_wait`: when clean-up points at Kōrero's
/// own runtime, start that instead of looking for a user-installed Ollama.
pub async fn start_if_ours(base_url: &str) -> Option<Result<bool, String>> {
    if !is_ours(base_url) {
        return None;
    }
    Some(start().await.map(|_| true))
}

/// Point the `ollama` provider at Kōrero's runtime, in one settings write;
/// with `switch`, also make it the clean-up provider.
fn point_cleanup_at(port: u16, switch: bool) -> Result<(), String> {
    let app = APP.get().ok_or("Kōrero isn't ready yet.")?;
    let mut s = crate::settings::get_settings(app);
    let p = s
        .post_process_providers
        .iter_mut()
        .find(|p| p.id == "ollama")
        .ok_or("The Ollama provider is missing from settings.")?;
    p.base_url = format!("http://127.0.0.1:{port}/v1");
    if switch {
        s.post_process_provider_id = "ollama".to_string();
    }
    crate::settings::write_settings(app, s);
    Ok(())
}

// ------------------------------------------------------------ doctor facts

/// Clean-up uses Kōrero's own runtime, and a runtime is downloaded. For
/// `ollama_status`, which otherwise looks only for a user-installed Ollama
/// and would tell the person to install it (Kōrero 1.49).
pub fn in_use_and_installed() -> bool {
    root().is_some_and(|r| {
        in_use_now(&r)
            && (installed_groups(&r).contains(&RuntimeGroup::Core) || old_runtime(&r).is_some())
    })
}

/// What the doctor needs to know about Kōrero's own runtime (Kōrero 1.49).
#[derive(Debug, Clone, Default)]
pub struct RuntimeFacts {
    pub supported: bool,
    /// Clean-up uses Kōrero's runtime now (`in_use_with`).
    pub in_use: bool,
    /// This version's runtime is downloaded.
    pub core_installed: bool,
    /// Only an earlier Kōrero's runtime is: it runs until this one is added.
    pub update_needed: bool,
    /// Kōrero's own runtime process is running.
    pub alive: bool,
    /// Still to download for this computer's plan, in MB.
    pub missing_mb: u32,
}

pub async fn facts() -> RuntimeFacts {
    let Some(root) = root() else {
        return RuntimeFacts {
            supported: cfg!(windows),
            ..Default::default()
        };
    };
    let installed = installed_groups(&root);
    let plan = tauri::async_runtime::spawn_blocking(plan_now)
        .await
        .ok()
        .map(|(p, _)| p);
    let missing: Vec<RuntimeGroup> = plan
        .map(|p| {
            p.groups
                .into_iter()
                .filter(|g| !installed.contains(g))
                .collect()
        })
        .unwrap_or_default();
    let core_installed = installed.contains(&RuntimeGroup::Core);
    RuntimeFacts {
        supported: cfg!(windows),
        in_use: in_use_now(&root),
        core_installed,
        update_needed: !core_installed && old_runtime(&root).is_some(),
        alive: child_alive(),
        missing_mb: mb(manifest().sizes(&missing).0),
    }
}

// ----------------------------------------------------------------- commands

#[derive(Debug, Clone, Serialize, Type)]
pub struct RuntimeStatus {
    /// False off Windows, where Kōrero does not ship its own runtime yet.
    pub supported: bool,
    pub version: String,
    pub installed: Vec<RuntimeGroup>,
    pub plan: RuntimePlan,
    /// What is still to download for the plan, in MB.
    pub missing_mb: u32,
    pub free_disk_mb: Option<u32>,
    pub running: bool,
    pub port: u16,
    /// Clean-up uses Kōrero's runtime now.
    pub in_use: bool,
    pub installing: bool,
    /// Only an earlier Kōrero's runtime is downloaded; it runs until this
    /// version's is added.
    pub update_needed: bool,
    /// Space the runtime and its models take, in MB ("Remove" frees it).
    pub disk_mb: u32,
}

/// Kōrero 1.49: what Kōrero's own local AI has, needs and is doing.
#[tauri::command]
#[specta::specta]
pub async fn local_ai_runtime_status(window: tauri::Window) -> Result<RuntimeStatus, String> {
    crate::commands::require_main_window(&window)?;
    let (plan, free) = tauri::async_runtime::spawn_blocking(plan_now)
        .await
        .map_err(|e| format!("Couldn't read this computer's hardware: {e}"))?;
    let root = root().ok_or("Kōrero couldn't find its data folder.")?;
    let installed = installed_groups(&root);
    let state = read_state(&root);
    let port = state.port.unwrap_or(PREFERRED_PORT);
    let missing: Vec<RuntimeGroup> = plan
        .groups
        .iter()
        .copied()
        .filter(|g| !installed.contains(g))
        .collect();
    let running = child_alive() && reachable(port).await;
    let core = installed.contains(&RuntimeGroup::Core);
    let r = root.clone();
    let disk = tauri::async_runtime::spawn_blocking(move || dir_size(&r))
        .await
        .unwrap_or(0);
    Ok(RuntimeStatus {
        supported: cfg!(windows),
        version: manifest().version.clone(),
        missing_mb: mb(manifest().sizes(&missing).0),
        installed,
        plan,
        free_disk_mb: free,
        running,
        port,
        in_use: in_use_now(&root),
        installing: INSTALLING.load(Ordering::SeqCst),
        update_needed: !core && old_runtime(&root).is_some(),
        disk_mb: mb(disk),
    })
}

struct Installing;
impl Drop for Installing {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::SeqCst);
    }
}

/// After this version's runtime is in place: move off an earlier version
/// that is still running, and delete earlier versions' folders.
async fn after_install() {
    let Some(root) = root() else {
        return;
    };
    let _guard = START_LOCK.lock().await;
    if child_alive() && running_dir() != Some(runtime_dir(&root)) {
        stop();
        if let Err(e) = start_locked().await {
            log::warn!("local AI runtime: couldn't restart on the new version: {e}");
        }
    }
    remove_old_runtimes(&root);
}

/// Kōrero 1.49: download (or finish downloading) the runtime this computer
/// needs. Progress arrives as `local-ai-runtime-progress` events. The size
/// is shown to the user before this is called; it never runs on its own.
#[tauri::command]
#[specta::specta]
pub async fn local_ai_runtime_install(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    crate::commands::require_main_window(&window)?;
    if !cfg!(windows) {
        return Err("Kōrero's own local AI runs on Windows for now.".into());
    }
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("The local AI is already downloading.".into());
    }
    let installing = Installing;
    CANCEL.store(false, Ordering::SeqCst);
    let (plan, _) = tauri::async_runtime::spawn_blocking(plan_now)
        .await
        .map_err(|e| e.to_string())?;
    install(&app, &plan.groups).await?;
    drop(installing);
    after_install().await;
    Ok(())
}

/// Kōrero 1.49: check every file of the runtime again and replace any that
/// is missing or damaged. Stops the runtime first, and starts it again after
/// (also when the repair fails, so a failed repair doesn't leave clean-up
/// stopped if the old files still run).
#[tauri::command]
#[specta::specta]
pub async fn local_ai_runtime_repair(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    crate::commands::require_main_window(&window)?;
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("The local AI is already downloading.".into());
    }
    let installing = Installing;
    CANCEL.store(false, Ordering::SeqCst);
    let was_running = {
        let _guard = START_LOCK.lock().await;
        let alive = child_alive();
        stop();
        alive
    };
    let (plan, _) = tauri::async_runtime::spawn_blocking(plan_now)
        .await
        .map_err(|e| e.to_string())?;
    let root = root().ok_or("Kōrero couldn't find its data folder.")?;
    let mut groups = installed_groups(&root);
    for g in plan.groups {
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    let result = install(&app, &groups).await;
    drop(installing);
    if result.is_ok() {
        after_install().await;
    }
    if was_running || in_use_now(&root) {
        if let Err(e) = start().await {
            log::warn!("local AI runtime: not running after repair: {e}");
        }
    }
    result
}

/// Kōrero 1.49: stop the download in progress.
#[tauri::command]
#[specta::specta]
pub fn local_ai_runtime_cancel(window: tauri::Window) -> Result<(), String> {
    crate::commands::require_main_window(&window)?;
    CANCEL.store(true, Ordering::SeqCst);
    Ok(())
}

/// Kōrero 1.49: start Kōrero's runtime and point clean-up at it. Returns
/// the port. What clean-up used before is kept, for "Stop using". The chosen
/// model is kept (it may need downloading into Kōrero's own models folder:
/// the setup guide does that next).
#[tauri::command]
#[specta::specta]
pub async fn local_ai_runtime_use(app: AppHandle, window: tauri::Window) -> Result<u16, String> {
    crate::commands::require_main_window(&window)?;
    let port = start().await?;
    let root = root().ok_or("Kōrero couldn't find its data folder.")?;
    let old = read_state(&root);
    let settings = crate::settings::get_settings(&app);
    let previous = if in_use_with(&old, &settings) {
        old.previous.clone()
    } else {
        Some(Previous {
            provider_id: settings.post_process_provider_id.clone(),
            ollama_base_url: settings
                .post_process_providers
                .iter()
                .find(|p| p.id == "ollama")
                .map(|p| p.base_url.clone())
                .unwrap_or_default(),
        })
    };
    write_json(
        &state_path(&root),
        &State {
            managed: true,
            port: Some(port),
            previous,
        },
    )?;
    point_cleanup_at(port, true)?;
    Ok(port)
}

/// Put clean-up back the way it was before "Use Kōrero's local AI", if it
/// still points at Kōrero's runtime. A provider or address the user has
/// changed since is left alone.
fn restore_previous(app: &AppHandle, state: &State) -> Result<(), String> {
    let mut s = crate::settings::get_settings(app);
    if !in_use_with(state, &s) {
        return Ok(());
    }
    let default = crate::settings::get_default_settings()
        .post_process_providers
        .into_iter()
        .find(|p| p.id == "ollama")
        .map(|p| p.base_url)
        .ok_or("The Ollama provider is missing from the defaults.")?;
    let (provider_id, base) = match &state.previous {
        Some(p) => {
            // The earlier address could itself be Kōrero's port: never put
            // that back.
            let base = if p.ollama_base_url.is_empty()
                || port_of(&p.ollama_base_url) == Some(state.port.unwrap_or(PREFERRED_PORT))
            {
                default
            } else {
                p.ollama_base_url.clone()
            };
            (p.provider_id.clone(), base)
        }
        None => ("ollama".to_string(), default),
    };
    if let Some(p) = s
        .post_process_providers
        .iter_mut()
        .find(|p| p.id == "ollama")
    {
        p.base_url = base;
    }
    if s.post_process_providers.iter().any(|p| p.id == provider_id) {
        s.post_process_provider_id = provider_id;
    }
    crate::settings::write_settings(app, s);
    Ok(())
}

/// Kōrero 1.49: go back to what clean-up used before (often an Ollama the
/// user runs), stop Kōrero's runtime, and keep its files and models.
#[tauri::command]
#[specta::specta]
pub async fn local_ai_runtime_stop_using(
    app: AppHandle,
    window: tauri::Window,
) -> Result<(), String> {
    crate::commands::require_main_window(&window)?;
    let root = root().ok_or("Kōrero couldn't find its data folder.")?;
    let _guard = START_LOCK.lock().await;
    let state = read_state(&root);
    restore_previous(&app, &state)?;
    write_json(
        &state_path(&root),
        &State {
            managed: false,
            port: state.port,
            previous: None,
        },
    )?;
    stop();
    Ok(())
}

/// Kōrero 1.49: stop using Kōrero's local AI and delete it and its models,
/// to free the disk space. Asked for in the page, which says how much first.
#[tauri::command]
#[specta::specta]
pub async fn local_ai_runtime_remove(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    crate::commands::require_main_window(&window)?;
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("The local AI is downloading. Cancel it first.".into());
    }
    let _installing = Installing;
    let root = root().ok_or("Kōrero couldn't find its data folder.")?;
    let _guard = START_LOCK.lock().await;
    let state = read_state(&root);
    restore_previous(&app, &state)?;
    stop();
    for dir in [root.join("runtime"), models_dir(&root)] {
        if dir.exists() {
            let d = dir.clone();
            tauri::async_runtime::spawn_blocking(move || std::fs::remove_dir_all(&d))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("Couldn't remove all of Kōrero's local AI: {e}"))?;
        }
    }
    write_json(&state_path(&root), &State::default())?;
    log::info!("local AI runtime: removed");
    Ok(())
}

#[cfg(test)]
mod runtime_tests {
    use super::zip::zip_tests::{crc32, tiny_zip};
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::Arc;

    /// A loopback server for one file that answers byte-range requests the
    /// way GitHub's asset store does (206 + Content-Range).
    fn serve_ranges(body: Vec<u8>) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/ollama-windows-amd64.zip",
            l.local_addr().unwrap().port()
        );
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h = hits.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                h.fetch_add(1, Ordering::SeqCst);
                let mut s = s;
                let mut buf = vec![0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let range = req.lines().find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("range: bytes=")
                        .map(str::to_string)
                });
                let (a, b) = range
                    .and_then(|r| {
                        let (a, b) = r.trim().split_once('-')?;
                        Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
                    })
                    .unwrap_or((0, body.len() - 1));
                let b = b.min(body.len() - 1);
                let part = &body[a..=b];
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {a}-{b}/{}\r\nConnection: close\r\n\r\n",
                    part.len(),
                    body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(part);
            }
        });
        (url, hits)
    }

    fn entry(path: &str, data: &[u8], compressed: u64) -> Entry {
        Entry {
            path: path.into(),
            group: RuntimeGroup::Core,
            size: data.len() as u64,
            compressed_size: compressed,
            crc32: crc32(data),
            sha256: to_hex(&Sha256::digest(data)),
        }
    }

    fn compressed_sizes(z: &[u8]) -> Vec<u64> {
        let (o, n) = zip::central_directory_range(z, z.len() as u64).unwrap();
        zip::parse_central_directory(&z[o as usize..(o + n) as usize])
            .unwrap()
            .iter()
            .map(|e| e.compressed_size)
            .collect()
    }

    #[test]
    fn files_are_taken_from_a_remote_zip_and_checked() {
        let exe = b"MZ pretend ollama".to_vec();
        let lib = "ggml ".repeat(4000).into_bytes();
        let z = tiny_zip(&[
            ("ollama.exe", &exe, false),
            ("lib/ollama/ggml.dll", &lib, true),
        ]);
        let sizes = compressed_sizes(&z);
        let (url, _) = serve_ranges(z.clone());
        let dir = tempfile::tempdir().unwrap();
        let e1 = entry("ollama.exe", &exe, sizes[0]);
        let e2 = entry("lib/ollama/ggml.dll", &lib, sizes[1]);
        let mut seen = 0u64;
        tauri::async_runtime::block_on(async {
            download(&url, z.len() as u64, &[&e1, &e2], dir.path(), &mut |p| {
                seen = p.done_bytes
            })
            .await
            .expect("download");
        });
        assert_eq!(std::fs::read(dir.path().join("ollama.exe")).unwrap(), exe);
        assert_eq!(
            std::fs::read(dir.path().join("lib/ollama/ggml.dll")).unwrap(),
            lib
        );
        assert_eq!(seen, sizes[0] + sizes[1]);
        assert!(!dir.path().join("ollama.exe.part").exists());
    }

    #[test]
    fn a_file_that_does_not_match_its_pinned_hash_is_never_used() {
        let exe = b"MZ pretend ollama".to_vec();
        let z = tiny_zip(&[("ollama.exe", &exe, false)]);
        let sizes = compressed_sizes(&z);
        let (url, _) = serve_ranges(z.clone());
        let dir = tempfile::tempdir().unwrap();
        let mut bad = entry("ollama.exe", &exe, sizes[0]);
        bad.sha256 = "0".repeat(64);
        let err = tauri::async_runtime::block_on(async {
            download(&url, z.len() as u64, &[&bad], dir.path(), &mut |_| {}).await
        })
        .unwrap_err();
        assert!(err.contains("did not match the pinned release"), "{err}");
        assert!(!dir.path().join("ollama.exe").exists());
        assert!(!dir.path().join("ollama.exe.part").exists());
    }

    #[test]
    fn a_different_zip_is_refused_before_anything_downloads() {
        let z = tiny_zip(&[("ollama.exe", b"MZ", false)]);
        let (url, hits) = serve_ranges(z.clone());
        let dir = tempfile::tempdir().unwrap();
        let e = entry("ollama.exe", b"MZ", 2);
        let err = tauri::async_runtime::block_on(async {
            download(&url, z.len() as u64 + 1, &[&e], dir.path(), &mut |_| {}).await
        })
        .unwrap_err();
        assert!(err.contains("different file"), "{err}");
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "only the size check was asked"
        );
    }

    #[test]
    fn files_already_right_are_not_downloaded_again() {
        let exe = b"MZ already here".to_vec();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ollama.exe"), &exe).unwrap();
        let e = entry("ollama.exe", &exe, 0);
        // No server at all: nothing may be fetched.
        tauri::async_runtime::block_on(async {
            download(
                "http://127.0.0.1:9/none.zip",
                1,
                &[&e],
                dir.path(),
                &mut |_| {},
            )
            .await
            .expect("nothing to do");
        });
    }

    #[test]
    fn ports_are_read_from_addresses() {
        assert_eq!(port_of("http://127.0.0.1:11435/v1"), Some(11435));
        assert_eq!(port_of("http://localhost:11434"), Some(11434));
        assert_eq!(port_of("http://localhost/v1"), None);
        assert_eq!(port_of("127.0.0.1:11436"), Some(11436));
    }

    #[test]
    fn state_reads_back_and_a_damaged_file_means_not_managed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_state(dir.path()), State::default());
        let s = State {
            managed: true,
            port: Some(11437),
            previous: Some(Previous {
                provider_id: "openai".into(),
                ollama_base_url: "http://localhost:11500/v1".into(),
            }),
        };
        write_json(&state_path(dir.path()), &s).unwrap();
        assert_eq!(read_state(dir.path()), s);
        std::fs::write(state_path(dir.path()), "{not json").unwrap();
        assert_eq!(read_state(dir.path()), State::default());
        // A state file from before `previous` existed still reads.
        std::fs::write(state_path(dir.path()), r#"{"managed":true,"port":11435}"#).unwrap();
        assert!(read_state(dir.path()).managed);
    }

    fn settings_at(provider: &str, base: &str) -> crate::settings::AppSettings {
        let mut s = crate::settings::get_default_settings();
        s.post_process_provider_id = provider.into();
        if let Some(p) = s.post_process_providers.iter_mut().find(|p| p.id == "ollama") {
            p.base_url = base.into();
        }
        s
    }

    #[test]
    fn in_use_means_pointed_at_it_and_still_pointing_at_it() {
        let st = State {
            managed: true,
            port: Some(11435),
            previous: None,
        };
        assert!(in_use_with(&st, &settings_at("ollama", "http://127.0.0.1:11435/v1")));
        // The user switched provider, or edited the address back to their own.
        assert!(!in_use_with(&st, &settings_at("openai", "http://127.0.0.1:11435/v1")));
        assert!(!in_use_with(&st, &settings_at("ollama", "http://localhost:11434/v1")));
        assert!(!in_use_with(&st, &settings_at("ollama", "http://192.168.1.9:11435/v1")));
        let off = State {
            managed: false,
            ..st
        };
        assert!(!in_use_with(&off, &settings_at("ollama", "http://127.0.0.1:11435/v1")));
    }

    #[test]
    fn an_earlier_version_is_used_until_this_one_is_downloaded_then_removed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(old_runtime(root), None);
        let old = root.join("runtime").join("ollama-0.12.0");
        std::fs::create_dir_all(&old).unwrap();
        // Half-installed: no marker, so not used.
        std::fs::write(old.join(exe_name()), b"x").unwrap();
        assert_eq!(old_runtime(root), None);
        std::fs::write(old.join("korero-installed.json"), r#"["core","vulkan"]"#).unwrap();
        assert_eq!(old_runtime(root), Some(old.clone()));
        // This version's own folder is never "earlier".
        let current = runtime_dir(root);
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join(exe_name()), b"x").unwrap();
        std::fs::write(current.join("korero-installed.json"), r#"["core"]"#).unwrap();
        assert_eq!(old_runtime(root), Some(old.clone()));
        remove_old_runtimes(root);
        assert!(!old.exists() && current.exists());
        assert_eq!(old_runtime(root), None);
    }

    #[test]
    fn a_file_bigger_than_pinned_never_reaches_disk_whole() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = HashFile {
            file: std::fs::File::create(dir.path().join("f.part")).unwrap(),
            hash: Sha256::new(),
            len: 0,
            limit: 4,
        };
        assert!(h.write_all(b"abcd").is_ok());
        assert!(h.write_all(b"e").is_err());
        assert_eq!(h.len, 4);
    }

    #[test]
    fn a_blocked_program_is_explained_without_its_path() {
        let e = std::io::Error::from_raw_os_error(1260);
        let m = start_error(&e);
        assert!(m.contains("IT team") && !m.contains("Users"), "{m}");
        let e = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        assert!(start_error(&e).starts_with("Couldn't start"));
    }

    /// Run by hand when the pinned version moves: the real zip's directory,
    /// read by this module's parser, must agree with the manifest file for
    /// file. `KORERO_RUNTIME_ZIP=<path to ollama-windows-amd64.zip>`.
    #[test]
    #[ignore]
    fn the_real_zip_matches_the_manifest() {
        let path = std::env::var("KORERO_RUNTIME_ZIP").expect("set KORERO_RUNTIME_ZIP");
        let z = std::fs::read(path).unwrap();
        assert_eq!(z.len() as u64, manifest().zip_size);
        let tail = &z[z.len() - zip::TAIL_LEN as usize..];
        let (o, n) = zip::central_directory_range(tail, z.len() as u64).unwrap();
        let dir = zip::parse_central_directory(&z[o as usize..(o + n) as usize]).unwrap();
        for e in &manifest().entries {
            let c = dir.iter().find(|c| c.name == e.path).expect(&e.path);
            assert_eq!(
                (c.crc32, c.size, c.compressed_size),
                (e.crc32, e.size, e.compressed_size),
                "{}",
                e.path
            );
            let h = zip::local_header_len(&z[c.header_offset as usize..]).unwrap();
            assert!(c.method == 0 || c.method == 8);
            assert!(c.header_offset + h + c.compressed_size <= o);
        }
    }
}
