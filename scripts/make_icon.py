#!/usr/bin/env python3
"""Generate a real SyncUp PNG icon using only the standard library."""

from __future__ import annotations

import math
import struct
import zlib
from pathlib import Path

SIZE = 512  # supersampled, downscaled to 256
FINAL = 256
CENTER = SIZE / 2
RADIUS = SIZE * 0.30
THICKNESS = SIZE * 0.085

BLUE = (21, 101, 192, 255)
BLUE_DARK = (13, 71, 161, 255)
WHITE = (255, 255, 255, 255)
TRANSPARENT = (0, 0, 0, 0)


def _in_rounded_rect(x: float, y: float, margin: float, radius: float) -> bool:
    left, top = margin, margin
    right, bottom = SIZE - margin, SIZE - margin
    if not (left <= x <= right and top <= y <= bottom):
        return False
    for cx, cy in (
        (left + radius, top + radius),
        (right - radius, top + radius),
        (left + radius, bottom - radius),
        (right - radius, bottom - radius),
    ):
        if x < cx - radius or x > cx + radius:
            continue
        if y < cy - radius or y > cy + radius:
            continue
        if (x - cx) ** 2 + (y - cy) ** 2 > radius ** 2:
            return False
    return True


def _point_in_triangle(
    px: float, py: float, a: tuple[float, float], b: tuple[float, float], c: tuple[float, float]
) -> bool:
    def sign(p1, p2, p3):
        return (p1[0] - p3[0]) * (p2[1] - p3[1]) - (p2[0] - p3[0]) * (p1[1] - p3[1])

    d1 = sign((px, py), a, b)
    d2 = sign((px, py), b, c)
    d3 = sign((px, py), c, a)
    has_neg = (d1 < 0) or (d2 < 0) or (d3 < 0)
    has_pos = (d1 > 0) or (d2 > 0) or (d3 > 0)
    return not (has_neg and has_pos)


def _arc_contains(angle_deg: float, start: float, end: float) -> bool:
    angle = angle_deg % 360
    start %= 360
    end %= 360
    if start <= end:
        return start <= angle <= end
    return angle >= start or angle <= end


def _render() -> list[list[tuple[int, int, int, int]]]:
    pixels: list[list[tuple[int, int, int, int]]] = []
    margin = SIZE * 0.035
    corner = SIZE * 0.19

    # Two white circular arrows.
    arcs = [(200.0, 340.0), (20.0, 160.0)]
    outer = RADIUS + THICKNESS / 2
    inner = RADIUS - THICKNESS / 2
    cap_radius = THICKNESS / 2

    arrowheads = []
    for start, end in arcs:
        angle = math.radians(end)
        px = CENTER + RADIUS * math.cos(angle)
        py = CENTER + RADIUS * math.sin(angle)
        tx, ty = -math.sin(angle), math.cos(angle)
        perp_x, perp_y = -ty, tx
        tip = (px + tx * THICKNESS * 1.25, py + ty * THICKNESS * 1.25)
        base_a = (
            px + perp_x * THICKNESS * 0.95,
            py + perp_y * THICKNESS * 0.95,
        )
        base_b = (
            px - perp_x * THICKNESS * 0.95,
            py - perp_y * THICKNESS * 0.95,
        )
        arrowheads.append((tip, base_a, base_b))

    for y in range(SIZE):
        row = []
        for x in range(SIZE):
            if not _in_rounded_rect(x, y, margin, corner):
                row.append(TRANSPARENT)
                continue

            # Subtle vertical gradient.
            ratio = y / SIZE
            color = tuple(
                int(BLUE[i] * (1 - ratio) + BLUE_DARK[i] * ratio) for i in range(3)
            ) + (255,)

            dx, dy = x - CENTER, y - CENTER
            distance = math.hypot(dx, dy)
            angle = math.degrees(math.atan2(dy, dx))

            white = False
            if inner <= distance <= outer:
                for start, end in arcs:
                    if _arc_contains(angle, start, end):
                        white = True
                        break

            if not white:
                # Round caps at both ends of both arcs.
                for start, end in arcs:
                    for endpoint in (start, end):
                        rad = math.radians(endpoint)
                        ex = CENTER + RADIUS * math.cos(rad)
                        ey = CENTER + RADIUS * math.sin(rad)
                        if (x - ex) ** 2 + (y - ey) ** 2 <= cap_radius ** 2:
                            white = True
                            break
                    if white:
                        break

            if not white:
                for triangle in arrowheads:
                    if _point_in_triangle(x, y, *triangle):
                        white = True
                        break

            row.append(WHITE if white else color)
        pixels.append(row)
    return pixels


def _downscale(pixels: list[list[tuple[int, int, int, int]]]) -> list[list[tuple[int, int, int, int]]]:
    output = []
    for y in range(0, SIZE, 2):
        row = []
        for x in range(0, SIZE, 2):
            block = [
                pixels[y][x],
                pixels[y][x + 1],
                pixels[y + 1][x],
                pixels[y + 1][x + 1],
            ]
            # Average only over visible-ish pixels so transparent edges stay clean.
            alpha = sum(pixel[3] for pixel in block)
            if alpha == 0:
                row.append(TRANSPARENT)
                continue
            weighted = [
                sum(pixel[channel] * pixel[3] for pixel in block) / alpha
                for channel in range(3)
            ]
            row.append(
                (
                    int(weighted[0]),
                    int(weighted[1]),
                    int(weighted[2]),
                    int(alpha / 4),
                )
            )
        output.append(row)
    return output


def _write_png(path: Path, pixels: list[list[tuple[int, int, int, int]]]) -> None:
    height = len(pixels)
    width = len(pixels[0])
    raw = bytearray()
    for row in pixels:
        raw.append(0)
        for pixel in row:
            raw.extend(pixel)

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png)


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    pixels = _downscale(_render())
    output = root / "assets" / "syncup.png"
    _write_png(output, pixels)
    print(f"Wrote {output} ({output.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
