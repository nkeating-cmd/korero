"""Kōrero 1.49 (D-014): write the pinned manifest for Kōrero's own Ollama runtime.

Kōrero downloads only the files a computer needs from Ollama's official Windows
zip, by HTTP range request, and checks each one against the SHA-256 written here.
This script is how those hashes are made. Run it when moving the pinned version:

  1. Download the release's `ollama-windows-amd64.zip` and `sha256sum.txt`
     (https://github.com/ollama/ollama/releases/download/<tag>/...).
  2. python -I scripts/ollama-runtime-manifest.py <tag> <zip> <sha256sum.txt> \
         src-tauri/src/local_runtime/ollama-runtime.json
  3. Check `ollama.exe` is Authenticode-signed by Ollama Inc. (signtool verify /pa,
     or the certificate table read in KORERO_LOCAL-AI-BUILD notes), then commit.

It refuses a zip whose SHA-256 is not the one Ollama published, so every hash in
the manifest traces back to Ollama's own release.
"""
import hashlib
import json
import sys
import zipfile


def group_of(name: str) -> str:
    for g in ("cuda_v12", "cuda_v13", "vulkan"):
        if f"/{g}/" in name:
            return g
    return "core"


def main(tag: str, zip_path: str, sums_path: str, out_path: str) -> None:
    published = None
    with open(sums_path, encoding="utf-8") as f:
        for line in f:
            parts = line.split()
            if len(parts) == 2 and parts[1].lstrip("./") == "ollama-windows-amd64.zip":
                published = parts[0].lower()
    if not published:
        sys.exit("FAIL: sha256sum.txt has no line for ollama-windows-amd64.zip")
    h = hashlib.sha256()
    size = 0
    with open(zip_path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
            size += len(chunk)
    if h.hexdigest() != published:
        sys.exit(f"FAIL: zip sha256 {h.hexdigest()} is not the published {published}")

    entries = []
    with zipfile.ZipFile(zip_path) as z:
        for i in z.infolist():
            if i.is_dir():
                continue
            if ".." in i.filename.split("/") or i.filename.startswith("/") or "\\" in i.filename:
                sys.exit(f"FAIL: unsafe path in zip: {i.filename!r}")
            if i.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED):
                sys.exit(f"FAIL: {i.filename} uses compression {i.compress_type}")
            data = z.read(i)
            entries.append({
                "path": i.filename,
                "group": group_of(i.filename),
                "size": i.file_size,
                "compressed_size": i.compress_size,
                "crc32": i.CRC,
                "sha256": hashlib.sha256(data).hexdigest(),
            })
    entries.sort(key=lambda e: e["path"])
    if not any(e["path"] == "ollama.exe" for e in entries):
        sys.exit("FAIL: no ollama.exe at the top of the zip")
    out = {
        "version": tag.lstrip("v"),
        "url": f"https://github.com/ollama/ollama/releases/download/{tag}/ollama-windows-amd64.zip",
        "zip_size": size,
        "zip_sha256": published,
        "entries": entries,
    }
    with open(out_path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(out, f, indent=1, sort_keys=False)
        f.write("\n")
    groups = {}
    for e in entries:
        g = groups.setdefault(e["group"], [0, 0, 0])
        g[0] += 1
        g[1] += e["compressed_size"]
        g[2] += e["size"]
    for g, (n, c, u) in sorted(groups.items()):
        print(f"{g:9} files={n:3} download_mb={c / 1048576:7.1f} unpacked_mb={u / 1048576:7.1f}")
    print(f"OK {out_path}: {len(entries)} files, zip {size} bytes, sha256 {published}")


if __name__ == "__main__":
    if len(sys.argv) != 5:
        sys.exit(__doc__)
    main(*sys.argv[1:])
