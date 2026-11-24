use chrono::NaiveDateTime;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static DATE_REGEX: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?P<y>\d{4})[-_]?(?P<m>\d{2})[-_]?(?P<d>\d{2})[T_ ]?(?P<h>\d{2})?(?P<mi>\d{2})?(?P<s>\d{2})?")
        .ok()
});

pub fn extract_date_from_filename(name: &str) -> Option<NaiveDateTime> {
    let regex = DATE_REGEX.as_ref()?;
    if let Some(caps) = regex.captures(name) {
        let y: i32 = caps.name("y")?.as_str().parse().ok()?;
        let m: u32 = caps.name("m")?.as_str().parse().ok()?;
        let d: u32 = caps.name("d")?.as_str().parse().ok()?;
        let h: u32 = caps
            .name("h")
            .map_or("0", |m| m.as_str())
            .parse()
            .unwrap_or(0);
        let mi: u32 = caps
            .name("mi")
            .map_or("0", |m| m.as_str())
            .parse()
            .unwrap_or(0);
        let s: u32 = caps
            .name("s")
            .map_or("0", |m| m.as_str())
            .parse()
            .unwrap_or(0);
        let date = chrono::NaiveDate::from_ymd_opt(y, m, d)?;
        let time = chrono::NaiveTime::from_hms_opt(h, mi, s)?;
        Some(NaiveDateTime::new(date, time))
    } else {
        None
    }
}

pub fn compute_destination_name(
    original_name: &str,
    date: NaiveDateTime,
    dest_dir: &Path,
    conflict_mode: crate::domain::conflict::ConflictMode,
) -> PathBuf {
    let ext = Path::new(original_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let date_str = date.format("%Y%m%d_%H%M%S").to_string();
    let base_name = Path::new(original_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let clean_base = if base_name.starts_with(&date_str) {
        base_name.to_string()
    } else {
        format!("{}_{}", date_str, base_name)
    };
    let mut candidate = if ext.is_empty() {
        dest_dir.join(&clean_base)
    } else {
        dest_dir.join(format!("{}.{}", clean_base, ext))
    };
    if matches!(
        conflict_mode,
        crate::domain::conflict::ConflictMode::KeepBoth
    ) {
        let mut counter = 1;
        while candidate.exists() {
            candidate = if ext.is_empty() {
                dest_dir.join(format!("{} ({})", clean_base, counter))
            } else {
                dest_dir.join(format!("{} ({}).{}", clean_base, counter, ext))
            };
            counter += 1;
        }
    }
    candidate
}
