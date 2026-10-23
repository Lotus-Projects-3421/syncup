"""Shared constants for SyncUp."""

APP_NAME = "SyncUp"
VERSION = "1.4.0"

# Media types handled by the app. Raw image formats are included so phone
# backups do not silently skip DNG/RAW files.
IMAGE_EXT = frozenset({
    ".jpg", ".jpeg", ".png", ".gif", ".bmp", ".tif", ".tiff",
    ".heic", ".heif", ".webp", ".avif",
    ".dng", ".cr2", ".nef", ".arw", ".orf", ".rw2", ".raf", ".srw", ".pef",
})

VIDEO_EXT = frozenset({
    ".mp4", ".mov", ".avi", ".mkv", ".3gp", ".m4v", ".webm",
    ".wmv", ".flv", ".ts", ".mts", ".m2ts", ".mpg", ".mpeg",
})

MEDIA_EXT = IMAGE_EXT | VIDEO_EXT

DEFAULT_SETTINGS = {
    "window_width": 1200,
    "window_height": 950,
    "theme": "dark",
}

# Name of the folder the duplicate finder moves files into.
DUP_PREFIX = "_Duplicates_"
