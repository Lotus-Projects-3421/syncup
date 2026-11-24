use std::collections::HashSet;
use std::sync::LazyLock;

pub static MEDIA_EXTENSIONS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let exts = [
        "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "heic", "heif", "avif", "cr2",
        "cr3", "nef", "arw", "dng", "orf", "rw2", "pef", "srw", "raf", "mp4", "mov", "avi", "mkv",
        "wmv", "flv", "webm", "m4v", "3gp", "mpg", "mpeg", "mts", "m2ts",
    ];
    exts.iter()
        .map(|s| format!(".{}", s.to_lowercase()))
        .collect()
});

pub fn is_media(path: &std::path::Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext_with_dot = format!(".{}", ext.to_lowercase());
        MEDIA_EXTENSIONS.contains(&ext_with_dot)
    } else {
        false
    }
}

pub const MAX_ARCHIVE_UNCOMPRESSED: u64 = 50 * 1024 * 1024 * 1024;
pub const MAX_FILES_PER_JOB: usize = 500_000;
