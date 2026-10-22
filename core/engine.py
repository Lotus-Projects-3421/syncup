import os
import re
import shutil
import time
from datetime import datetime
from pathlib import Path
from core.storage import load_state, save_state

IMAGE_EXT = {'.jpg','.jpeg','.png','.gif','.bmp','.tif','.tiff','.heic','.heif','.webp'}
VIDEO_EXT = {'.mp4','.mov','.avi','.mkv','.3gp','.m4v'}

def get_media_files(folder_path, media_filter):
    media = []
    for root, _, files in os.walk(folder_path):
        for f in files:
            ext = Path(f).suffix.lower()
            if media_filter == "photos" and ext in IMAGE_EXT: media.append(Path(root) / f)
            elif media_filter == "videos" and ext in VIDEO_EXT: media.append(Path(root) / f)
            elif media_filter == "both" and (ext in IMAGE_EXT or ext in VIDEO_EXT): media.append(Path(root) / f)
    return media

def parse_date(file_path):
    stem = file_path.stem
    m = re.search(r'(19\d{2}|20\d{2})[\-\._]?(0[1-9]|1[0-2])[\-\._]?(0[1-9]|[12]\d|3[01])', stem)
    if m:
        try: return datetime(int(m.group(1)), int(m.group(2)), int(m.group(3)))
        except ValueError: pass
    try: return datetime.fromtimestamp(file_path.stat().st_mtime)
    except Exception: return datetime.now()

def get_target_path(file_path, dest_root, sort_mode):
    date = parse_date(file_path)
    year, month, day = date.strftime('%Y'), date.strftime('%m'), date.strftime('%d')
    time_str = date.strftime('%H-%M-%S')
    new_name = f"{year}-{month}-{day}_{time_str}_{file_path.stem}{file_path.suffix.lower()}"

    if sort_mode == "year": return dest_root / year / new_name
    elif sort_mode == "month": return dest_root / year / f"{year}-{month}" / new_name
    elif sort_mode == "day": return dest_root / year / f"{year}-{month}-{day}" / new_name
    elif sort_mode == "flat": return dest_root / new_name
    else: return dest_root / file_path.name

def run_backup(phone_id, phone_path, selected_folders, staging_path, dest_path, sort_mode, media_filter, conflict_mode, progress_callback, cancel_event):
    staging_root = Path(staging_path)
    dest_root = Path(dest_path)
    phone_root = Path(phone_path) / "Internal shared storage"
    if not phone_root.exists(): phone_root = Path(phone_path)

    processed_files = load_state(phone_id)
    total_files, processed, skipped, errors = 0, 0, 0, 0
    start_time = time.time()
    
    progress_callback(0, 0, "Scanning for new files...")
    files_to_copy = []
    for folder in selected_folders:
        folder_path = phone_root / folder
        if folder_path.exists():
            for src_file in get_media_files(folder_path, media_filter):
                rel_path = str(src_file.relative_to(phone_root))
                if rel_path not in processed_files:
                    files_to_copy.append((src_file, rel_path))
                else:
                    skipped += 1

    total_files = len(files_to_copy)
    if total_files == 0: return {"error": f"No new matching files found. ({skipped} already backed up)."}

    for src_file, rel_path in files_to_copy:
        if cancel_event.is_set(): return {"error": "Cancelled by user."}
            
        processed += 1
        progress_callback(processed, total_files, f"Copying {src_file.name}...")
        
        try:
            dst_file = get_target_path(src_file, dest_root, sort_mode)
            dst_file.parent.mkdir(parents=True, exist_ok=True)
            
            do_copy = True
            if dst_file.exists():
                if src_file.stat().st_size == dst_file.stat().st_size:
                    do_copy = False # Skip exact dup
                    processed_files.add(rel_path)
                elif conflict_mode == "overwrite":
                    dst_file.unlink() # Delete old file
                    do_copy = True
                else: # keep
                    counter = 1
                    while dst_file.exists():
                        dst_file = dst_file.parent / f"{dst_file.stem}__{counter}{dst_file.suffix}"
                        counter += 1
                    do_copy = True
            
            if do_copy:
                temp_dst = staging_root / f"tmp_{src_file.name}"
                shutil.copyfile(src_file, temp_dst)
                shutil.move(temp_dst, dst_file)
                processed_files.add(rel_path)
                if processed % 10 == 0: save_state(phone_id, processed_files)
        except Exception: errors += 1

    save_state(phone_id, processed_files)
    time_taken = time.time() - start_time
    return {"total": total_files, "processed": processed - errors, "skipped": skipped, "errors": errors, "time": f"{int(time_taken//60)}m {int(time_taken%60)}s"}
