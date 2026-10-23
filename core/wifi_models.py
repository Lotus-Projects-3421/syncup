"""Pydantic models and TypedDicts for the Wi-Fi Backup API contracts.

Keeping the wire format in one module means the FastAPI routes, the mobile
page, and the storage helpers all agree on field names and value ranges.
"""

from __future__ import annotations

from typing import Literal, TypedDict

from pydantic import BaseModel, Field

WiFiStage = Literal["waiting", "uploading", "processing", "done", "error"]


# ---------------------------------------------------------------------------
# TypedDicts used by the state-diff helper (core.storage.get_missing_files)
# ---------------------------------------------------------------------------


class WiFiManifestEntry(TypedDict):
    """A file as described by the phone, relative to the selected folder."""

    rel_path: str
    size: int
    mtime_ns: int


class WiFiMissingFile(TypedDict):
    """A manifest entry that is not present (or is stale) in the PC state DB."""

    rel_path: str
    status: Literal["new", "modified"]
    size: int
    mtime_ns: int


# ---------------------------------------------------------------------------
# Manifest / diff
# ---------------------------------------------------------------------------


class ManifestEntry(BaseModel):
    """Single file entry in the phone's manifest."""

    rel_path: str = Field(
        ...,
        min_length=1,
        max_length=4096,
        description="Relative path from the selected phone folder",
    )
    size: int = Field(..., ge=0, description="File size in bytes")
    mtime_ns: int = Field(..., ge=0, description="Modification time in nanoseconds")


class ManifestRequest(BaseModel):
    """Request body for ``POST /manifest``."""

    device_id: str = Field(
        default="wifi-backup",
        max_length=128,
        description="Phone-generated identifier (for logs only)",
    )
    files: list[ManifestEntry] = Field(
        ...,
        description="All candidate files the phone is offering",
    )


class MissingFileResponse(BaseModel):
    """Single missing file entry returned by ``POST /manifest``."""

    rel_path: str
    status: Literal["new", "modified"]
    size: int = Field(..., ge=0)
    mtime_ns: int = Field(..., ge=0)


class ManifestResponse(BaseModel):
    """Response for ``POST /manifest``."""

    missing: list[MissingFileResponse] = Field(
        default_factory=list,
        description="Files that still need to be uploaded, in upload order",
    )
    total_bytes: int = Field(
        default=0,
        ge=0,
        description="Total number of bytes the phone should upload",
    )


# ---------------------------------------------------------------------------
# Upload
# ---------------------------------------------------------------------------


class UploadResponse(BaseModel):
    """Response for a successful ``POST /upload``."""

    ok: bool = True
    rel_path: str
    files_received: int = Field(..., ge=0)
    expected_files: int = Field(..., ge=0)


class UploadInitRequest(BaseModel):
    """Reserved for a future resumable-upload flow."""

    rel_path: str
    size: int = Field(..., ge=0)
    mtime_ns: int = Field(..., ge=0)


class UploadInitResponse(BaseModel):
    """Reserved for a future resumable-upload flow."""

    upload_url: str


# ---------------------------------------------------------------------------
# Status polling
# ---------------------------------------------------------------------------


class StatusResponse(BaseModel):
    """Response for ``GET /status`` (polled by the phone)."""

    stage: WiFiStage
    files_received: int = Field(default=0, ge=0)
    files_processed: int = Field(default=0, ge=0)
    total_files: int = Field(default=0, ge=0)
    current_file: str | None = None
    error: str | None = None


__all__ = [
    "WiFiStage",
    "WiFiManifestEntry",
    "WiFiMissingFile",
    "ManifestEntry",
    "ManifestRequest",
    "MissingFileResponse",
    "ManifestResponse",
    "UploadResponse",
    "UploadInitRequest",
    "UploadInitResponse",
    "StatusResponse",
]
