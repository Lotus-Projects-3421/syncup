import os
import hashlib
import shutil
import time
from pathlib import Path

def get_file_hash(filepath):
    hasher = hashlib.md5()
    with open(filepath, 'rb') as afile:
        buf = afile.read(65536)
        while len(buf) > 0:
            hasher.update(buf)
            buf = afile.read(65536)
    return hasher.hexdigest()

def run_dedup(scan_dir, progress_callback, cancel_event):
    scan_path = Path(scan_dir)
    if not scan_path.exists() or not scan_path.is_dir():
        return {"error": "Invalid folder."}

    progress_callback(0, 0, "Scanning files...")
    all_files = []
    for root, _, files in os.walk(scan_path):
        # Skip our own duplicate folder if it exists
        if "_Duplicates_" in root: continue
        for f in files:
            p = Path(root) / f
            if p.stat().st_size > 0:
                all_files.append(p)

    total_files = len(all_files)
    if total_files == 0: return {"error": "No files found to scan."}

    # Group by size first (much faster)
    by_size = {}
    for p in all_files:
        size = p.stat().st_size
        if size not in by_size: by_size[size] = []
        by_size[size].append(p)

    # Filter out unique sizes
    potential_dups = [files for size, files in by_size.items() if len(files) > 1]
    total_to_hash = sum(len(f) for f in potential_dups)
    
    if total_to_hash == 0:
        return {"processed": 0, "moved": 0, "time": "0m 0s"}

    # Hash them
    hashed_files = {}
    hashed_count = 0
    start_time = time.time()
    
    for group in potential_dups:
        for p in group:
            if cancel_event.is_set(): return {"error": "Cancelled by user."}
            hashed_count += 1
            progress_callback(hashed_count, total_to_hash, f"Hashing {p.name}...")
            try:
                h = get_file_hash(p)
                if h not in hashed_files: hashed_files[h] = []
                hashed_files[h].append(p)
            except: pass

    # Move duplicates
    dup_dir = scan_path / f"_Duplicates_{int(time.time())}"
    moved_count = 0
    
    for h, paths in hashed_files.items():
        if len(paths) > 1:
            # Keep the first one, move the rest
            keep_file = paths[0]
            for dup_file in paths[1:]:
                if cancel_event.is_set(): return {"error": "Cancelled by user."}
                try:
                    # Recreate relative path in dup_dir to avoid name collisions
                    rel_path = dup_file.relative_to(scan_path)
                    dest_dup = dup_dir / rel_path
                    dest_dup.parent.mkdir(parents=True, exist_ok=True)
                    
                    # Handle name conflicts in the dup folder
                    counter = 1
                    while dest_dup.exists():
                        dest_dup = dest_dup.parent / f"{dest_dup.stem}_{counter}{dest_dup.suffix}"
                        counter += 1
                        
                    shutil.move(str(dup_file), str(dest_dup))
                    moved_count += 1
                except Exception as e:
                    pass

    time_taken = time.time() - start_time
    return {"processed": hashed_count, "moved": moved_count, "time": f"{int(time_taken//60)}m {int(time_taken%60)}s"}
