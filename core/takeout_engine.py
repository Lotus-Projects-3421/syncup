import os
import re
import json
import shutil
import zipfile
import tempfile
import time
from datetime import datetime
from pathlib import Path

IMAGE_EXT = {'.jpg','.jpeg','.png','.gif','.bmp','.tif','.tiff','.heic','.heif','.webp'}
VIDEO_EXT = {'.mp4','.mov','.avi','.mkv','.3gp','.m4v'}

def get_date_from_json(json_path):
    try:
        with open(json_path, 'r', encoding='utf-8') as f:
            data = json.load(f)
            for key in ('photoTakenTime', 'creationTime'):
                if key in data and 'timestamp' in data[key]:
                    return datetime.fromtimestamp(int(data[key]['timestamp']))
    except: pass
    return None

def get_date_from_name(stem):
    m = re.search(r'(19\d{2}|20\d{2})[\-\._]?(0[1-9]|1[0-2])[\-\._]?(0[1-9]|[12]\d|3[01])', stem)
    if m:
        try: return datetime(int(m.group(1)), int(m.group(2)), int(m.group(3)))
        except: pass
    return None

def get_target_path(file_path, dest_root, sort_mode, date_obj):
    year, month, day = date_obj.strftime('%Y'), date_obj.strftime('%m'), date_obj.strftime('%d')
    time_str = date_obj.strftime('%H-%M-%S')
    new_name = f"{year}-{month}-{day}_{time_str}_{file_path.stem}{file_path.suffix.lower()}"
    if sort_mode == "year": return dest_root / year / new_name
    elif sort_mode == "month": return dest_root / year / f"{year}-{month}" / new_name
    elif sort_mode == "day": return dest_root / year / f"{year}-{month}-{day}" / new_name
    elif sort_mode == "flat": return dest_root / new_name
    else: return dest_root / file_path.name

def run_takeout(input_items, dest_path, sort_mode, media_filter, conflict_mode, progress_callback, cancel_event):
    dest_root = Path(dest_path)
    dest_resolved = dest_root.resolve()
    loose_files, zips = [], []
    
    progress_callback(0, 0, "Scanning folders and ZIPs...")
    for item in input_items:
        if cancel_event.is_set(): return {"error": "Cancelled by user."}
        item = Path(item)
        if item.is_dir():
            for root, dirs, files in os.walk(item):
                root_path = Path(root).resolve()
                if root_path == dest_resolved or dest_resolved in root_path.parents: continue
                for f in files:
                    p = Path(root) / f
                    ext = p.suffix.lower()
                    if ext == '.zip': zips.append(p)
                    elif (media_filter == "photos" and ext in IMAGE_EXT) or (media_filter == "videos" and ext in VIDEO_EXT) or (media_filter == "both" and (ext in IMAGE_EXT or ext in VIDEO_EXT)):
                        loose_files.append(p)
        elif item.is_file():
            if item.suffix.lower() == '.zip': zips.append(item)
                
    if not loose_files and not zips: return {"error": "No matching media or .zip files found."}

    total_files = len(loose_files)
    for zip_path in zips:
        try:
            with zipfile.ZipFile(zip_path, 'r') as z:
                for f in z.namelist():
                    ext = Path(f).suffix.lower()
                    if (media_filter == "photos" and ext in IMAGE_EXT) or (media_filter == "videos" and ext in VIDEO_EXT) or (media_filter == "both" and (ext in IMAGE_EXT or ext in VIDEO_EXT)):
                        total_files += 1
        except: pass

    processed_files, total_copied, errors = 0, 0, 0
    start_time = time.time()

    def process_file(src_file):
        nonlocal total_copied, errors, processed_files
        json_path = src_file.parent / (src_file.name + ".json")
        date_obj = get_date_from_json(json_path) if json_path.exists() else None
        if not date_obj: date_obj = get_date_from_name(src_file.stem)
        if not date_obj: date_obj = datetime.fromtimestamp(src_file.stat().st_mtime)
        try:
            dst_file = get_target_path(src_file, dest_root, sort_mode, date_obj)
            dst_file.parent.mkdir(parents=True, exist_ok=True)
            
            if dst_file.exists():
                if conflict_mode == "overwrite": dst_file.unlink()
                else:
                    counter = 1
                    while dst_file.exists():
                        dst_file = dst_file.parent / f"{dst_file.stem}__{counter}{dst_file.suffix}"
                        counter += 1
                        
            shutil.copy2(src_file, dst_file)
            total_copied += 1
        except: errors += 1
        processed_files += 1
        progress_callback(processed_files, total_files, f"Organizing {src_file.name}")

    for src_file in loose_files:
        if cancel_event.is_set(): return {"error": "Cancelled by user."}
        process_file(src_file)

    for zip_path in zips:
        if cancel_event.is_set(): return {"error": "Cancelled by user."}
        progress_callback(processed_files, total_files, f"Extracting {zip_path.name}...")
        with tempfile.TemporaryDirectory(prefix="takeout_") as temp_dir:
            temp_path = Path(temp_dir)
            try:
                with zipfile.ZipFile(zip_path, 'r') as z: z.extractall(temp_path)
            except: errors += 1; continue
            for root, _, files in os.walk(temp_path):
                for f in files:
                    p = Path(root) / f
                    ext = p.suffix.lower()
                    if (media_filter == "photos" and ext in IMAGE_EXT) or (media_filter == "videos" and ext in VIDEO_EXT) or (media_filter == "both" and (ext in IMAGE_EXT or ext in VIDEO_EXT)):
                        if cancel_event.is_set(): return {"error": "Cancelled by user."}
                        process_file(p)

    time_taken = time.time() - start_time
    return {"total": total_files, "processed": total_copied, "errors": errors, "time": f"{int(time_taken//60)}m {int(time_taken%60)}s"}
