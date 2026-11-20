//! Kōrero 1.49 (D-014): the pinned Ollama runtime, and which parts of it a
//! computer needs.
//!
//! `ollama-runtime.json` is written by `scripts/ollama-runtime-manifest.py`
//! from Ollama's official `ollama-windows-amd64.zip`, after checking the zip
//! against the SHA-256 Ollama published for it. Every file Kōrero downloads
//! is checked against the hash recorded here, so nothing it runs can differ
//! from that release. `ollama.exe` in the pinned release is Authenticode-signed
//! by Ollama Inc. (checked when the manifest was made).

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Component, Path, PathBuf};

/// A part of the runtime, downloaded together.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeGroup {
    /// `ollama.exe` and its processor libraries. Always needed.
    Core,
    /// The Vulkan libraries: graphics cards from any maker (about 12 MB).
    Vulkan,
    /// NVIDIA's CUDA 12 libraries, for NVIDIA drivers 550 to 579.
    CudaV12,
    /// NVIDIA's CUDA 13 libraries, for NVIDIA driver 580 or later.
    CudaV13,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub path: String,
    pub group: RuntimeGroup,
    pub size: u64,
    pub compressed_size: u64,
    pub crc32: u32,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub url: String,
    pub zip_size: u64,
    pub zip_sha256: String,
    pub entries: Vec<Entry>,
}

static MANIFEST: Lazy<Manifest> = Lazy::new(|| {
    // Embedded at build time and checked by `manifest_tests`, so this cannot
    // fail in a build that passed its tests.
    serde_json::from_str(include_str!("ollama-runtime.json"))
        .expect("ollama-runtime.json is valid (checked by tests)")
});

pub fn manifest() -> &'static Manifest {
    &MANIFEST
}

impl Manifest {
    pub fn files<'a>(&'a self, groups: &'a [RuntimeGroup]) -> impl Iterator<Item = &'a Entry> + 'a {
        self.entries.iter().filter(|e| groups.contains(&e.group))
    }

    /// (download bytes, unpacked bytes) for these groups.
    pub fn sizes(&self, groups: &[RuntimeGroup]) -> (u64, u64) {
        self.files(groups)
            .fold((0, 0), |(d, u), e| (d + e.compressed_size, u + e.size))
    }
}

/// A zip path as a relative path under the runtime folder, or `None` if it
/// could escape it (`..`, an absolute path, a drive letter, a backslash).
pub fn safe_relative(path: &str) -> Option<PathBuf> {
    if path.is_empty() || path.contains('\\') || path.contains(':') || path.starts_with('/') {
        return None;
    }
    let p = Path::new(path);
    p.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then(|| p.to_path_buf())
}

/// Which CUDA libraries an NVIDIA driver can use, from `nvidia-smi`'s
/// `driver_version` ("621.24"). CUDA 13 needs driver 580 or later (NVIDIA's
/// release notes); Ollama supports NVIDIA from driver 550. Below that, or
/// unreadable: none, and the card runs through Vulkan.
pub fn cuda_for_driver(driver: &str) -> Option<RuntimeGroup> {
    let major: u32 = driver.trim().split('.').next()?.trim().parse().ok()?;
    match major {
        580.. => Some(RuntimeGroup::CudaV13),
        550..=579 => Some(RuntimeGroup::CudaV12),
        _ => None,
    }
}

/// The groups this computer needs: always the core and Vulkan; one CUDA set
/// when it has an NVIDIA graphics card whose driver can use it.
pub fn plan_for(has_nvidia: bool, nvidia_driver: Option<&str>) -> Vec<RuntimeGroup> {
    let mut g = vec![RuntimeGroup::Core, RuntimeGroup::Vulkan];
    if has_nvidia {
        if let Some(c) = nvidia_driver.and_then(cuda_for_driver) {
            g.push(c);
        }
    }
    g
}

#[cfg(test)]
mod manifest_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_pinned_manifest_is_whole_and_safe() {
        let m = manifest();
        assert!(!m.version.is_empty());
        assert!(m
            .url
            .starts_with("https://github.com/ollama/ollama/releases/download/v"));
        assert!(m.url.ends_with("/ollama-windows-amd64.zip"));
        assert_eq!(m.zip_sha256.len(), 64);
        let mut seen = HashSet::new();
        for e in &m.entries {
            assert!(seen.insert(&e.path), "duplicate {}", e.path);
            assert!(safe_relative(&e.path).is_some(), "unsafe {}", e.path);
            assert_eq!(e.sha256.len(), 64, "{}", e.path);
            assert!(
                e.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                "{}",
                e.path
            );
            assert!(e.compressed_size <= m.zip_size);
        }
        let exe = m
            .entries
            .iter()
            .find(|e| e.path == "ollama.exe")
            .expect("ollama.exe");
        assert_eq!(exe.group, RuntimeGroup::Core);
        for g in [
            RuntimeGroup::Core,
            RuntimeGroup::Vulkan,
            RuntimeGroup::CudaV12,
            RuntimeGroup::CudaV13,
        ] {
            assert!(m.entries.iter().any(|e| e.group == g), "{g:?} is empty");
        }
    }

    /// The point of fetching by group: a computer without NVIDIA downloads
    /// tens of MB, not the 1.5 GB zip (measured 17 Nov: 40 MB).
    #[test]
    fn without_nvidia_the_download_is_small() {
        let (d, _) = manifest().sizes(&plan_for(false, None));
        assert!(d < 80 * 1024 * 1024, "{d} bytes");
        let (d13, _) = manifest().sizes(&plan_for(true, Some("621.24")));
        assert!(d13 > d && d13 < 900 * 1024 * 1024, "{d13} bytes");
    }

    #[test]
    fn unsafe_paths_are_refused() {
        for bad in ["", "../x", "lib/../../x", "/etc/x", "C:/x", "lib\\x.dll"] {
            assert!(safe_relative(bad).is_none(), "{bad}");
        }
        assert!(safe_relative("lib/ollama/vulkan/ggml-vulkan.dll").is_some());
    }

    #[test]
    fn the_driver_picks_the_cuda_set() {
        assert_eq!(cuda_for_driver("621.24"), Some(RuntimeGroup::CudaV13));
        assert_eq!(cuda_for_driver("580.00"), Some(RuntimeGroup::CudaV13));
        assert_eq!(cuda_for_driver("579.99"), Some(RuntimeGroup::CudaV12));
        assert_eq!(cuda_for_driver("551.61"), Some(RuntimeGroup::CudaV12));
        assert_eq!(cuda_for_driver("537.13"), None);
        assert_eq!(cuda_for_driver(""), None);
        assert_eq!(cuda_for_driver("n/a"), None);
        assert_eq!(
            plan_for(true, None),
            vec![RuntimeGroup::Core, RuntimeGroup::Vulkan]
        );
        assert_eq!(
            plan_for(false, Some("621.24")),
            vec![RuntimeGroup::Core, RuntimeGroup::Vulkan]
        );
    }
}
