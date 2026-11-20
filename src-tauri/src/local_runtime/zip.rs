//! Kōrero 1.49: just enough of the zip format to take single files out of a
//! remote zip by HTTP range request.
//!
//! Ollama's Windows zip is 1.5 GB, and most of it is NVIDIA's CUDA libraries.
//! Reading the zip's directory from its end tells where each file starts, so
//! a computer downloads only the files it needs. Every file is then checked
//! against the pinned SHA-256 (`manifest.rs`), so a parsing mistake here can
//! fail a download but cannot let a wrong file through.
//!
//! Only what Ollama's zip uses is supported: stored or deflated files, no
//! ZIP64 (the zip is under 4 GB), no encryption, no multi-part archives.

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;
/// End-of-central-directory record (22 bytes) plus the longest comment.
pub const TAIL_LEN: u64 = 22 + 0xFFFF;
/// A local header is 30 bytes plus the name and extra field; Ollama's are
/// short, so this covers them with room to spare in one request.
pub const LOCAL_HEADER_READ: u64 = 30 + 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CentralEntry {
    pub name: String,
    pub method: u16,
    pub crc32: u32,
    pub compressed_size: u64,
    pub size: u64,
    pub header_offset: u64,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// Find the central directory from the end of the file. `tail` is the last
/// bytes of a file `file_len` long. Returns (offset, size) of the directory.
pub fn central_directory_range(tail: &[u8], file_len: u64) -> Result<(u64, u64), String> {
    if tail.len() < 22 {
        return Err("The download is too short to be a zip.".into());
    }
    let mut i = tail.len() - 22;
    loop {
        if u32_at(tail, i) == Some(EOCD_SIG) {
            let size = u64::from(u32_at(tail, i + 12).unwrap_or(0));
            let offset = u64::from(u32_at(tail, i + 16).unwrap_or(0));
            if size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF {
                return Err("The runtime zip uses ZIP64, which Kōrero does not read.".into());
            }
            if offset.checked_add(size).is_none_or(|end| end > file_len) {
                return Err("The runtime zip's directory points past its end.".into());
            }
            return Ok((offset, size));
        }
        if i == 0 {
            return Err("No zip directory found at the end of the download.".into());
        }
        i -= 1;
    }
}

/// Parse the central directory.
pub fn parse_central_directory(cd: &[u8]) -> Result<Vec<CentralEntry>, String> {
    let bad = || "The runtime zip's directory is damaged.".to_string();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < cd.len() {
        if u32_at(cd, i) != Some(CENTRAL_SIG) {
            return Err(bad());
        }
        let method = u16_at(cd, i + 10).ok_or_else(bad)?;
        let crc32 = u32_at(cd, i + 16).ok_or_else(bad)?;
        let csize = u32_at(cd, i + 20).ok_or_else(bad)?;
        let size = u32_at(cd, i + 24).ok_or_else(bad)?;
        let n = usize::from(u16_at(cd, i + 28).ok_or_else(bad)?);
        let m = usize::from(u16_at(cd, i + 30).ok_or_else(bad)?);
        let k = usize::from(u16_at(cd, i + 32).ok_or_else(bad)?);
        let offset = u32_at(cd, i + 42).ok_or_else(bad)?;
        let name = cd.get(i + 46..i + 46 + n).ok_or_else(bad)?;
        if [csize, size, offset].contains(&0xFFFF_FFFF) {
            return Err("The runtime zip uses ZIP64, which Kōrero does not read.".into());
        }
        out.push(CentralEntry {
            name: String::from_utf8_lossy(name).into_owned(),
            method,
            crc32,
            compressed_size: u64::from(csize),
            size: u64::from(size),
            header_offset: u64::from(offset),
        });
        i = i + 46 + n + m + k;
    }
    Ok(out)
}

/// How many bytes the local header at the start of `h` takes, so the file's
/// data starts right after it.
pub fn local_header_len(h: &[u8]) -> Result<u64, String> {
    if u32_at(h, 0) != Some(LOCAL_SIG) {
        return Err("A file in the runtime zip has a damaged header.".into());
    }
    let n = u64::from(u16_at(h, 26).ok_or("short local header")?);
    let m = u64::from(u16_at(h, 28).ok_or("short local header")?);
    Ok(30 + n + m)
}

#[cfg(test)]
pub(crate) mod zip_tests {
    use super::*;
    use std::io::Write;

    /// A small zip built by hand: one stored file, one deflated, a comment.
    pub(crate) fn tiny_zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in files {
            let crc = crc32(data);
            let body = if *deflate {
                let mut e =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let method: u16 = if *deflate { 8 } else { 0 };
            let offset = out.len() as u32;
            out.extend_from_slice(&LOCAL_SIG.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0, 0, 0, 0]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&4u16.to_le_bytes()); // a 4-byte extra field
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&[0xCA, 0xFE, 0, 0]);
            out.extend_from_slice(&body);

            central.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0, 0, 0, 0]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            // extra length, comment length, disk, internal and external attributes
            central.extend_from_slice(&[0u8; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        let comment = b"kia ora";
        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        out.extend_from_slice(comment);
        out
    }

    pub(crate) fn crc32(data: &[u8]) -> u32 {
        let mut h = flate2::Crc::new();
        h.update(data);
        h.sum()
    }

    #[test]
    fn files_are_found_and_read_from_their_offsets() {
        let big = "whānau ".repeat(500);
        let z = tiny_zip(&[
            ("ollama.exe", b"MZ stored", false),
            ("lib/a.dll", big.as_bytes(), true),
        ]);
        let tail_start = z.len().saturating_sub(TAIL_LEN as usize);
        let (off, size) = central_directory_range(&z[tail_start..], z.len() as u64).unwrap();
        let entries = parse_central_directory(&z[off as usize..(off + size) as usize]).unwrap();
        assert_eq!(entries.len(), 2);
        for (e, want) in entries
            .iter()
            .zip([b"MZ stored".to_vec(), big.as_bytes().to_vec()])
        {
            let h = local_header_len(&z[e.header_offset as usize..]).unwrap();
            let start = (e.header_offset + h) as usize;
            let data = &z[start..start + e.compressed_size as usize];
            let got = if e.method == 8 {
                let mut d = flate2::write::DeflateDecoder::new(Vec::new());
                d.write_all(data).unwrap();
                d.finish().unwrap()
            } else {
                data.to_vec()
            };
            assert_eq!(got, want, "{}", e.name);
            assert_eq!(crc32(&got), e.crc32);
            assert_eq!(got.len() as u64, e.size);
        }
    }

    #[test]
    fn damaged_or_foreign_data_is_refused() {
        assert!(central_directory_range(b"short", 5).is_err());
        assert!(central_directory_range(&[0u8; 100], 100).is_err());
        let z = tiny_zip(&[("a", b"x", false)]);
        // A directory that claims to run past the end of the file.
        assert!(central_directory_range(&z, 10).is_err());
        assert!(parse_central_directory(b"PK\x01\x02 too short").is_err());
        assert!(local_header_len(b"not a header at all...........").is_err());
    }
}
