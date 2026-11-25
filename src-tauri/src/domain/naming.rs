//! Date parsing, destination naming, and dependency-free EXIF date reading.
//!
//! Ported from the Python oracle `core/naming.py` (Preserve per PLAN.md §3):
//! prefix `%Y-%m-%d_%H-%M-%S_`, repeatable prefix stripping, midnight repair,
//! 140-char stem cap, lowercase extensions, year/month/day/flat layouts, and
//! the minimal JPEG/TIFF EXIF reader (2 MiB bound, same tags and formats).
//!
//! Documented deviation: JSON timestamps and mtimes convert in the local
//! timezone to match the oracle's `datetime.fromtimestamp`.

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use regex::Regex;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const MAX_STEM_LENGTH: usize = 140;
pub const EXIF_MAX_READ: usize = 2 * 1024 * 1024;

const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_DATETIME_ORIGINAL: u16 = 0x9003;
const TAG_DATETIME_DIGITIZED: u16 = 0x9004;
const TAG_DATETIME: u16 = 0x0132;
const TYPE_ASCII: u16 = 2;
const TYPE_LONG: u16 = 4;

static DATE_TIME_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?P<year>19\d{2}|20\d{2})[-._]?(?P<month>0[1-9]|1[0-2])[-._]?(?P<day>0[1-9]|[12]\d|3[01])(?:[T _.-]?(?P<hour>[01]\d|2[0-3])[-._:]?(?P<minute>[0-5]\d)[-._:]?(?P<second>[0-5]\d))?")
        .ok()
});

static GENERATED_PREFIX_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}_").ok());

/// Destination layout, matching the oracle's sort modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SortMode {
    Year,
    Month,
    Day,
    Flat,
}

/// Which source supplied the chosen date (oracle parity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateSource {
    Json,
    Filename,
    Exif,
    Mtime,
}

fn match_datetime(stem: &str) -> Option<NaiveDateTime> {
    let re = DATE_TIME_RE.as_ref()?;
    let caps = re.captures(stem)?;
    let year: i32 = caps.name("year")?.as_str().parse().ok()?;
    let month: u32 = caps.name("month")?.as_str().parse().ok()?;
    let day: u32 = caps.name("day")?.as_str().parse().ok()?;
    let hour: u32 = caps
        .name("hour")
        .map_or(0, |m| m.as_str().parse().unwrap_or(0));
    let minute: u32 = caps
        .name("minute")
        .map_or(0, |m| m.as_str().parse().unwrap_or(0));
    let second: u32 = caps
        .name("second")
        .map_or(0, |m| m.as_str().parse().unwrap_or(0));
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let time = NaiveTime::from_hms_opt(hour, minute, second)?;
    Some(NaiveDateTime::new(date, time))
}

pub fn strip_generated_prefix(stem: &str) -> String {
    let mut result = stem.to_string();
    if let Some(re) = GENERATED_PREFIX_RE.as_ref() {
        while re.is_match(&result) {
            result = re.replace(&result, "").to_string();
        }
    }
    if result.is_empty() {
        stem.to_string()
    } else {
        result
    }
}

pub fn parse_datetime_from_name(stem: &str) -> Option<NaiveDateTime> {
    let stripped = strip_generated_prefix(stem);
    if stripped != stem {
        let prefix_dt = match_datetime(stem)?;
        if prefix_dt.hour() == 0 && prefix_dt.minute() == 0 && prefix_dt.second() == 0 {
            let better = match_datetime(&stripped).filter(|b| b.date() == prefix_dt.date());
            if better.is_some() {
                return better;
            }
        }
        return Some(prefix_dt);
    }
    match_datetime(stem)
}

pub fn build_target_name(original_name: &str, date: NaiveDateTime) -> String {
    let path = Path::new(original_name);
    let stem = path.file_stem().and_then(|s| s.to_str());
    let stem = stem.unwrap_or("file");
    let mut base_stem = strip_generated_prefix(stem);
    if base_stem.chars().count() > MAX_STEM_LENGTH {
        base_stem = base_stem.chars().take(MAX_STEM_LENGTH).collect();
    }
    let ext = path.extension().and_then(|e| e.to_str());
    let ext = ext.unwrap_or("");
    let prefix = date.format("%Y-%m-%d_%H-%M-%S");
    if ext.is_empty() {
        format!("{prefix}_{base_stem}")
    } else {
        format!("{prefix}_{base_stem}.{}", ext.to_lowercase())
    }
}

pub fn build_target_path(
    original_name: &str,
    date: NaiveDateTime,
    dest_root: &Path,
    sort_mode: SortMode,
) -> PathBuf {
    let new_name = build_target_name(original_name, date);
    let year = date.format("%Y").to_string();
    let month = date.format("%Y-%m").to_string();
    let day = date.format("%Y-%m-%d").to_string();
    match sort_mode {
        SortMode::Year => dest_root.join(year).join(new_name),
        SortMode::Month => dest_root.join(year).join(month).join(new_name),
        SortMode::Day => dest_root.join(year).join(day).join(new_name),
        SortMode::Flat => dest_root.join(new_name),
    }
}

pub fn system_time_to_naive(time: Option<std::time::SystemTime>) -> NaiveDateTime {
    match time {
        Some(t) => {
            let dt: DateTime<Local> = t.into();
            dt.naive_local()
        }
        None => Local::now().naive_local(),
    }
}

pub fn naive_from_unix_timestamp(secs: i64) -> Option<NaiveDateTime> {
    let local = Local.timestamp_opt(secs, 0).single()?;
    Some(local.naive_local())
}

pub fn date_with_source(
    path: &Path,
    modified: Option<std::time::SystemTime>,
) -> (NaiveDateTime, DateSource) {
    let stem = path.file_stem().and_then(|s| s.to_str());
    let stem = stem.unwrap_or("");
    if let Some(dt) = parse_datetime_from_name(stem) {
        return (dt, DateSource::Filename);
    }
    if let Some(dt) = read_exif_datetime(path) {
        return (dt, DateSource::Exif);
    }
    (system_time_to_naive(modified), DateSource::Mtime)
}

pub fn exif_supported(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str());
    match ext {
        Some(e) => matches!(e.to_lowercase().as_str(), "jpg" | "jpeg" | "tif" | "tiff"),
        None => false,
    }
}

pub fn read_head(path: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    let mut limited = file.take(limit as u64);
    limited.read_to_end(&mut buf)?;
    Ok(buf)
}

pub fn read_exif_datetime(path: &Path) -> Option<NaiveDateTime> {
    if !exif_supported(path) {
        return None;
    }
    let data = read_head(path, EXIF_MAX_READ).ok()?;
    exif_datetime_from_bytes(&data)
}

pub fn exif_datetime_from_bytes(data: &[u8]) -> Option<NaiveDateTime> {
    if data.starts_with(b"II") || data.starts_with(b"MM") {
        return parse_tiff_datetime(data);
    }
    parse_jpeg_datetime(data)
}

#[derive(Clone, Copy)]
enum Endian {
    Little,
    Big,
}

fn read_u16(data: &[u8], offset: usize, endian: Endian) -> Option<u16> {
    let bytes = data.get(offset..offset + 2)?;
    let arr = [bytes[0], bytes[1]];
    Some(match endian {
        Endian::Little => u16::from_le_bytes(arr),
        Endian::Big => u16::from_be_bytes(arr),
    })
}

fn read_u32(data: &[u8], offset: usize, endian: Endian) -> Option<u32> {
    let bytes = data.get(offset..offset + 4)?;
    let arr = [bytes[0], bytes[1], bytes[2], bytes[3]];
    Some(match endian {
        Endian::Little => u32::from_le_bytes(arr),
        Endian::Big => u32::from_be_bytes(arr),
    })
}

fn ifd_pointer(data: &[u8], endian: Endian, ifd_offset: usize, wanted_tag: u16) -> Option<usize> {
    if ifd_offset == 0 {
        return None;
    }
    let count = read_u16(data, ifd_offset, endian)? as usize;
    let start = ifd_offset + 2;
    if start + count * 12 > data.len() {
        return None;
    }
    for index in 0..count {
        let entry = start + index * 12;
        let tag = read_u16(data, entry, endian)?;
        let value_type = read_u16(data, entry + 2, endian)?;
        if tag == wanted_tag && value_type == TYPE_LONG {
            return read_u32(data, entry + 8, endian).map(|v| v as usize);
        }
    }
    None
}

fn ascii_tag(data: &[u8], endian: Endian, ifd_offset: usize, wanted_tag: u16) -> Option<String> {
    if ifd_offset == 0 {
        return None;
    }
    let count = read_u16(data, ifd_offset, endian)? as usize;
    let start = ifd_offset + 2;
    if start + count * 12 > data.len() {
        return None;
    }
    for index in 0..count {
        let entry = start + index * 12;
        let tag = read_u16(data, entry, endian)?;
        let value_type = read_u16(data, entry + 2, endian)?;
        let value_count = read_u32(data, entry + 4, endian)? as usize;
        if tag != wanted_tag || value_type != TYPE_ASCII || value_count == 0 {
            continue;
        }
        let raw: &[u8] = if value_count <= 4 {
            data.get(entry + 8..entry + 8 + value_count)?
        } else {
            let value_offset = read_u32(data, entry + 8, endian)? as usize;
            data.get(value_offset..value_offset + value_count)?
        };
        let text = raw.split(|b| *b == 0).next()?;
        let text = String::from_utf8_lossy(text).trim().to_string();
        return Some(text);
    }
    None
}

fn parse_exif_text(value: &str) -> Option<NaiveDateTime> {
    let text = value.trim();
    for fmt in [
        "%Y:%m:%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y:%m:%d %H:%M",
        "%Y:%m:%d",
    ] {
        let mut parsed = NaiveDateTime::parse_from_str(text, fmt).ok();
        if parsed.is_none() {
            if let Ok(d) = NaiveDate::parse_from_str(text, fmt) {
                parsed = d.and_hms_opt(0, 0, 0);
            }
        }
        if let Some(dt) = parsed {
            if (1900..=3000).contains(&dt.year()) {
                return Some(dt);
            }
        }
    }
    None
}

fn parse_tiff_datetime(data: &[u8]) -> Option<NaiveDateTime> {
    if data.len() < 8 {
        return None;
    }
    let endian = if data.starts_with(b"II") {
        Endian::Little
    } else if data.starts_with(b"MM") {
        Endian::Big
    } else {
        return None;
    };
    if read_u16(data, 2, endian)? != 42 {
        return None;
    }
    let ifd0 = read_u32(data, 4, endian)? as usize;
    let mut candidates: Vec<String> = Vec::new();
    if let Some(exif_ifd) = ifd_pointer(data, endian, ifd0, TAG_EXIF_IFD) {
        if let Some(v) = ascii_tag(data, endian, exif_ifd, TAG_DATETIME_ORIGINAL) {
            candidates.push(v);
        }
        if let Some(v) = ascii_tag(data, endian, exif_ifd, TAG_DATETIME_DIGITIZED) {
            candidates.push(v);
        }
    }
    if let Some(v) = ascii_tag(data, endian, ifd0, TAG_DATETIME) {
        candidates.push(v);
    }
    for candidate in &candidates {
        if let Some(dt) = parse_exif_text(candidate) {
            return Some(dt);
        }
    }
    None
}

fn parse_jpeg_datetime(data: &[u8]) -> Option<NaiveDateTime> {
    if !data.starts_with(b"\xff\xd8") {
        return None;
    }
    let size = data.len();
    let mut offset = 2usize;
    while offset + 1 < size {
        if data[offset] != 0xFF {
            offset += 1;
            continue;
        }
        while offset < size && data[offset] == 0xFF {
            offset += 1;
        }
        if offset >= size {
            break;
        }
        let marker = data[offset];
        offset += 1;
        if matches!(marker, 0xD8 | 0xD9 | 0x01) || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        if marker == 0xDA {
            break;
        }
        if offset + 2 > size {
            break;
        }
        let segment_length = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        if segment_length < 2 {
            break;
        }
        let segment_start = offset + 2;
        let segment_end = (offset + segment_length).min(size);
        let is_exif = marker == 0xE1
            && data.get(segment_start..segment_start + 6) == Some(b"Exif\x00\x00".as_slice());
        if is_exif {
            let block = data.get(segment_start + 6..segment_end)?;
            return parse_tiff_datetime(block);
        }
        offset = segment_end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiff_with_datetime(text: &str) -> Vec<u8> {
        let text_len = (text.len() + 1) as u32;
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x8769u16.to_le_bytes());
        tiff.extend_from_slice(&4u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&26u32.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x9003u16.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes());
        tiff.extend_from_slice(&text_len.to_le_bytes());
        tiff.extend_from_slice(&44u32.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff.extend_from_slice(text.as_bytes());
        tiff.push(0);
        tiff
    }

    #[test]
    fn exif_tiff_datetime_original() -> Result<(), Box<dyn std::error::Error>> {
        let data = tiff_with_datetime("2021:05:04 03:02:01");
        let dt = exif_datetime_from_bytes(&data).ok_or("exif date")?;
        let expected = NaiveDate::from_ymd_opt(2021, 5, 4)
            .ok_or("date")?
            .and_hms_opt(3, 2, 1)
            .ok_or("time")?;
        assert_eq!(dt, expected);
        Ok(())
    }

    #[test]
    fn malformed_exif_ignored() {
        assert!(exif_datetime_from_bytes(b"garbage data here").is_none());
        assert!(exif_datetime_from_bytes(b"II").is_none());
        let mut truncated = tiff_with_datetime("2021:05:04 03:02:01");
        truncated.truncate(12);
        assert!(exif_datetime_from_bytes(&truncated).is_none());
        assert!(exif_datetime_from_bytes(&[0xFF, 0xD8, 0xFF, 0xD9]).is_none());
    }

    #[test]
    fn prefix_strip_and_midnight_repair() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            strip_generated_prefix("2024-01-01_00-00-00_IMG_0001"),
            "IMG_0001"
        );
        assert_eq!(
            strip_generated_prefix("2024-01-01_00-00-00_2023-12-31_23-59-59_photo"),
            "photo"
        );
        let repaired = parse_datetime_from_name("2024-01-01_00-00-00_IMG_20240101_123456")
            .ok_or("repaired")?;
        assert_eq!(repaired.time().format("%H:%M:%S").to_string(), "12:34:56");
        Ok(())
    }

    #[test]
    fn target_name_cap_and_lowercase_ext() -> Result<(), Box<dyn std::error::Error>> {
        let long_stem = "a".repeat(200);
        let name = format!("{long_stem}.JPG");
        let date = NaiveDate::from_ymd_opt(2020, 2, 29)
            .ok_or("date")?
            .and_hms_opt(1, 2, 3)
            .ok_or("time")?;
        let target = build_target_name(&name, date);
        assert!(target.starts_with("2020-02-29_01-02-03_"));
        assert!(target.ends_with(".jpg"));
        let stem_len = target
            .trim_start_matches("2020-02-29_01-02-03_")
            .trim_end_matches(".jpg")
            .chars()
            .count();
        assert_eq!(stem_len, MAX_STEM_LENGTH);
        Ok(())
    }

    #[test]
    fn filename_regex_bounds() {
        assert!(parse_datetime_from_name("IMG_18991231_120000").is_none());
        assert!(parse_datetime_from_name("IMG_20241301_120000").is_none());
        assert!(parse_datetime_from_name("IMG_20240132_120000").is_none());
        assert!(parse_datetime_from_name("no digits here").is_none());
        assert!(parse_datetime_from_name("IMG_20240101_120000").is_some());
    }

    #[test]
    fn sort_layouts() -> Result<(), Box<dyn std::error::Error>> {
        let date = NaiveDate::from_ymd_opt(2022, 7, 8)
            .ok_or("date")?
            .and_hms_opt(9, 10, 11)
            .ok_or("time")?;
        let root = Path::new("/tmp/root");
        let year = build_target_path("p.jpg", date, root, SortMode::Year);
        assert_eq!(year, Path::new("/tmp/root/2022/2022-07-08_09-10-11_p.jpg"));
        let month = build_target_path("p.jpg", date, root, SortMode::Month);
        assert_eq!(
            month,
            Path::new("/tmp/root/2022/2022-07/2022-07-08_09-10-11_p.jpg")
        );
        let day = build_target_path("p.jpg", date, root, SortMode::Day);
        assert_eq!(
            day,
            Path::new("/tmp/root/2022/2022-07-08/2022-07-08_09-10-11_p.jpg")
        );
        let flat = build_target_path("p.jpg", date, root, SortMode::Flat);
        assert_eq!(flat, Path::new("/tmp/root/2022-07-08_09-10-11_p.jpg"));
        Ok(())
    }

    #[test]
    fn json_timestamp_local() -> Result<(), Box<dyn std::error::Error>> {
        let dt = naive_from_unix_timestamp(0).ok_or("epoch")?;
        let expected = system_time_to_naive(Some(std::time::SystemTime::UNIX_EPOCH));
        assert_eq!(dt, expected);
        Ok(())
    }
}
