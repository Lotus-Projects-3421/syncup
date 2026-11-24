use blake2::digest::consts::U16;
use blake2::{Blake2b, Digest};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const QUICK_CHUNK: usize = 64 * 1024;

pub fn quick_hash(path: &Path, size: u64) -> std::io::Result<[u8; 16]> {
    let mut hasher = Blake2b::<U16>::new();
    hasher.update(size.to_string().as_bytes());
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; QUICK_CHUNK];
    let n = file.read(&mut buf)?;
    hasher.update(&buf[..n]);
    if size > QUICK_CHUNK as u64 {
        file.seek(SeekFrom::End(-(QUICK_CHUNK as i64)))?;
        let n = file.read(&mut buf)?;
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

pub fn path_depth(path: &Path, root: &Path) -> usize {
    path.strip_prefix(root)
        .ok()
        .map(|p| p.components().count())
        .unwrap_or(usize::MAX)
}

pub fn sort_retention_key(path: &Path, root: &Path, mtime_ns: i64) -> (usize, i64, String) {
    (
        path_depth(path, root),
        mtime_ns,
        path.to_string_lossy().to_string(),
    )
}
