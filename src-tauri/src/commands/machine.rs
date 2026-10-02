//! Kōrero 1.43: what this computer can run, so AI clean-up & notes can offer a
//! local model that fits.
//!
//! Read-only and local: nothing here is stored or sent anywhere, and the
//! profile never carries a path (only whether the models folder is custom).
//!
//! WHY NOT WMI. `Win32_VideoController.AdapterRAM` is a 32-bit field: legion's
//! 12 GB RTX 4080 Laptop reads as 4 GB there. DXGI's `DedicatedVideoMemory`
//! is the real figure.

use serde::Serialize;
use specta::Type;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GpuKind {
    /// Its own memory: models run there at full speed.
    Discrete,
    /// Shares system memory; Ollama mostly runs on the processor.
    Integrated,
    /// Apple Silicon: one pool of unified memory.
    Apple,
}

#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct GpuInfo {
    pub name: String,
    pub vram_mb: u32,
    pub kind: GpuKind,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MachineProfile {
    pub gpus: Vec<GpuInfo>,
    pub ram_mb: u32,
    /// Free space where Ollama keeps models. `None` when it couldn't be read
    /// (the fit rules then skip the disk check rather than refuse everything).
    pub free_disk_mb: Option<u32>,
    /// `OLLAMA_MODELS` points somewhere other than the default.
    pub models_dir_custom: bool,
    pub os: String,
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const MB: u64 = 1024 * 1024;

fn to_mb(bytes: u64) -> u32 {
    (bytes / MB).min(u64::from(u32::MAX)) as u32
}

/// Integrated or discrete, from the vendor and the name. Below 2 GiB of its
/// own memory a GPU can't hold any model in the catalogue, so it counts as
/// integrated whatever it is. AMD APUs report their BIOS carve-out (often
/// 2–8 GB) as dedicated memory, so the name decides for AMD.
pub fn classify_gpu(vendor_id: u32, name: &str, vram_mb: u32) -> GpuKind {
    if vram_mb < 2_048 {
        return GpuKind::Integrated;
    }
    let n = name.to_ascii_lowercase();
    match vendor_id {
        0x8086 => {
            if n.contains("arc") {
                GpuKind::Discrete
            } else {
                GpuKind::Integrated
            }
        }
        0x1002 => {
            // APUs are named "... Graphics" ("AMD Radeon(TM) RX Vega 10
            // Graphics", "AMD Radeon 780M Graphics"); cards aren't
            // (refuter DEF-07).
            if n.ends_with("graphics") {
                GpuKind::Integrated
            } else if ["radeon rx", " rx ", "radeon pro", "firepro", "instinct"]
                .iter()
                .any(|k| n.contains(k))
            {
                GpuKind::Discrete
            } else {
                GpuKind::Integrated
            }
        }
        _ => GpuKind::Discrete,
    }
}

/// `/proc/meminfo` → total MB.
#[cfg_attr(windows, allow(dead_code))] // used on macOS/Linux and in tests
pub fn parse_meminfo(text: &str) -> Option<u32> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(to_mb(kb * 1024))
}

/// `nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits`.
#[cfg_attr(windows, allow(dead_code))] // used on macOS/Linux and in tests
pub fn parse_nvidia_smi(text: &str) -> Vec<GpuInfo> {
    text.lines()
        .filter_map(|l| {
            let (name, mem) = l.rsplit_once(',')?;
            let vram_mb: u32 = mem.trim().parse().ok()?;
            let name = name.trim().to_string();
            Some(GpuInfo {
                kind: classify_gpu(0x10DE, &name, vram_mb),
                name,
                vram_mb,
            })
        })
        .collect()
}

/// `df -Pk -- <path>` → available MB (4th column of the second line).
#[cfg_attr(windows, allow(dead_code))] // used on macOS/Linux and in tests
pub fn parse_df(text: &str) -> Option<u32> {
    let line = text.lines().nth(1)?;
    let kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(to_mb(kb * 1024))
}

/// The closest folder that exists, so a models folder Ollama hasn't created
/// yet is measured on the right drive. `None` if nothing on the path exists
/// (an unplugged drive).
pub fn nearest_existing_ancestor(p: &Path) -> Option<PathBuf> {
    let mut cur = Some(p);
    while let Some(c) = cur {
        if c.exists() {
            return Some(c.to_path_buf());
        }
        cur = c.parent();
    }
    None
}

/// Where Ollama keeps models, and whether that's a custom location. A
/// relative `OLLAMA_MODELS` is ignored rather than resolved against our cwd.
fn models_dir() -> (Option<PathBuf>, bool) {
    if let Some(v) = std::env::var_os("OLLAMA_MODELS") {
        let p = PathBuf::from(v);
        if p.is_absolute() {
            return (Some(p), true);
        }
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    (
        home.map(|h| PathBuf::from(h).join(".ollama").join("models")),
        false,
    )
}

/// Run `f` on its own thread; give up after PROBE_TIMEOUT. A disk query on a
/// sleeping network drive can block for tens of seconds.
fn with_timeout<T: Send + 'static>(f: impl FnOnce() -> Option<T> + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(PROBE_TIMEOUT).ok().flatten()
}

/// Run a tool by absolute path with a timeout, killing it if it hangs.
#[cfg(not(windows))]
fn run_tool(path: &str, args: &[&str]) -> Option<String> {
    use std::process::{Command, Stdio};
    if !Path::new(path).exists() {
        return None;
    }
    let mut child = Command::new(path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > PROBE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    }
    let out = child.wait_with_output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
fn gpus() -> Vec<GpuInfo> {
    use std::collections::HashSet;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND,
    };
    let mut out = Vec::new();
    // SAFETY: plain DXGI factory creation; no COM apartment needed.
    let factory: IDXGIFactory1 = match unsafe { CreateDXGIFactory1() } {
        Ok(f) => f,
        Err(e) => {
            log::warn!("machine: DXGI unavailable: {e}");
            return out;
        }
    };
    let mut seen = HashSet::new();
    for i in 0..16u32 {
        // SAFETY: index-based enumeration; DXGI_ERROR_NOT_FOUND ends the list.
        let adapter = match unsafe { factory.EnumAdapters1(i) } {
            Ok(a) => a,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(_) => break,
        };
        // SAFETY: GetDesc1 fills a plain struct.
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        if desc.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 || desc.VendorId == 0x1414 {
            continue; // software rasteriser / Microsoft Basic Render Driver
        }
        let luid =
            (i64::from(desc.AdapterLuid.HighPart) << 32) | i64::from(desc.AdapterLuid.LowPart);
        if !seen.insert(luid) {
            continue;
        }
        let len = desc
            .Description
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(desc.Description.len());
        let name = String::from_utf16_lossy(&desc.Description[..len])
            .trim()
            .to_string();
        let vram_mb = to_mb(desc.DedicatedVideoMemory as u64);
        out.push(GpuInfo {
            kind: classify_gpu(desc.VendorId, &name, vram_mb),
            name,
            vram_mb,
        });
    }
    out
}

#[cfg(windows)]
fn ram_mb() -> Option<u32> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: dwLength is set as the API requires.
    unsafe { GlobalMemoryStatusEx(&mut m) }.ok()?;
    Some(to_mb(m.ullTotalPhys))
}

#[cfg(windows)]
fn free_disk_mb(p: &Path) -> Option<u32> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let dir = nearest_existing_ancestor(p)?;
    with_timeout(move || {
        let h = HSTRING::from(dir.as_os_str());
        let mut free: u64 = 0;
        // SAFETY: valid NUL-terminated path; only the first out-parameter is used.
        unsafe { GetDiskFreeSpaceExW(&h, Some(&mut free as *mut u64), None, None) }.ok()?;
        Some(to_mb(free))
    })
}

// ---------------------------------------------------------------- macOS / Linux

#[cfg(target_os = "macos")]
fn ram_mb() -> Option<u32> {
    let out = run_tool("/usr/sbin/sysctl", &["-n", "hw.memsize"])?;
    Some(to_mb(out.trim().parse().ok()?))
}

#[cfg(target_os = "macos")]
fn gpus() -> Vec<GpuInfo> {
    if cfg!(target_arch = "aarch64") {
        let ram = ram_mb().unwrap_or(0);
        vec![GpuInfo {
            name: "Apple Silicon (unified memory)".to_string(),
            vram_mb: ram,
            kind: GpuKind::Apple,
        }]
    } else {
        Vec::new()
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn ram_mb() -> Option<u32> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn gpus() -> Vec<GpuInfo> {
    run_tool(
        "/usr/bin/nvidia-smi",
        &[
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ],
    )
    .map(|s| parse_nvidia_smi(&s))
    .unwrap_or_default()
}

#[cfg(not(windows))]
fn free_disk_mb(p: &Path) -> Option<u32> {
    let dir = nearest_existing_ancestor(p)?;
    let dir = dir.to_str()?.to_string();
    with_timeout(move || parse_df(&run_tool("/bin/df", &["-Pk", "--", &dir])?))
}

/// The profile, gathered synchronously (call it off the main thread).
pub fn profile_now() -> MachineProfile {
    let (dir, custom) = models_dir();
    MachineProfile {
        gpus: gpus(),
        ram_mb: ram_mb().unwrap_or(0),
        free_disk_mb: dir.as_deref().and_then(free_disk_mb),
        models_dir_custom: custom,
        os: std::env::consts::OS.to_string(),
    }
}

/// Kōrero 1.43: this computer's graphics memory, memory and free disk, for
/// the "Run AI clean-up on this computer" card. Async and off the main
/// thread: a sync command would run on it and could freeze the window.
#[tauri::command]
#[specta::specta]
pub async fn get_machine_profile(window: tauri::Window) -> Result<MachineProfile, String> {
    super::require_main_window(&window)?;
    tauri::async_runtime::spawn_blocking(profile_now)
        .await
        .map_err(|e| format!("Couldn't read this computer's hardware: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_gpus() {
        assert_eq!(
            classify_gpu(0x10DE, "NVIDIA GeForce RTX 4080 Laptop GPU", 12_282),
            GpuKind::Discrete
        );
        assert_eq!(
            classify_gpu(0x8086, "Intel(R) UHD Graphics", 128),
            GpuKind::Integrated
        );
        assert_eq!(
            classify_gpu(0x8086, "Intel(R) Iris(R) Xe Graphics", 2_048),
            GpuKind::Integrated
        );
        assert_eq!(
            classify_gpu(0x8086, "Intel(R) Arc(TM) A770 Graphics", 16_032),
            GpuKind::Discrete
        );
        assert_eq!(
            classify_gpu(0x1002, "AMD Radeon(TM) Graphics", 4_096),
            GpuKind::Integrated
        );
        assert_eq!(
            classify_gpu(0x1002, "AMD Radeon 780M", 8_192),
            GpuKind::Integrated
        );
        assert_eq!(
            classify_gpu(0x1002, "AMD Radeon RX 7600", 8_176),
            GpuKind::Discrete
        );
        assert_eq!(
            classify_gpu(0x1002, "AMD Radeon(TM) RX Vega 10 Graphics", 2_048),
            GpuKind::Integrated
        );
        assert_eq!(
            classify_gpu(0x1002, "AMD Radeon RX Vega 64", 8_176),
            GpuKind::Discrete
        );
        assert_eq!(
            classify_gpu(0x10DE, "NVIDIA GeForce MX150", 2_000),
            GpuKind::Integrated
        );
    }

    #[test]
    fn parses_tools() {
        assert_eq!(
            parse_meminfo("MemTotal:       32734104 kB\nMemFree: 1 kB\n"),
            Some(31_966)
        );
        assert_eq!(parse_meminfo("nothing here"), None);
        let g = parse_nvidia_smi("NVIDIA GeForce RTX 4060 Laptop GPU, 8188\nbad line\n");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].vram_mb, 8_188);
        assert_eq!(g[0].kind, GpuKind::Discrete);
        let df = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 100 50 2097152 50% /\n";
        assert_eq!(parse_df(df), Some(2_048));
        assert_eq!(parse_df("header only\n"), None);
    }

    #[test]
    fn nearest_ancestor_finds_the_drive() {
        let base = std::env::temp_dir();
        let missing = base.join("korero-143-none").join("models").join("blobs");
        assert_eq!(nearest_existing_ancestor(&missing), Some(base.clone()));
        assert_eq!(nearest_existing_ancestor(&base), Some(base));
    }

    #[test]
    #[ignore = "live: reads this computer's hardware (legion)"]
    fn live_profile() {
        let p = profile_now();
        println!("LIVE profile: {:?}", p);
        assert!(p.ram_mb >= 30_000, "ram {}", p.ram_mb);
        assert!(p.free_disk_mb.is_some(), "disk unknown");
        assert!(
            p.gpus
                .iter()
                .any(|g| g.kind == GpuKind::Discrete && g.vram_mb >= 11_000),
            "no 11 GB+ discrete GPU: {:?}",
            p.gpus
        );
        assert!(
            p.gpus.iter().any(|g| g.kind == GpuKind::Integrated),
            "the Intel iGPU should read as integrated: {:?}",
            p.gpus
        );
    }
}
