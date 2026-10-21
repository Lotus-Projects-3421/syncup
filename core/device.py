import os
import re
import getpass
import subprocess
from pathlib import Path

def _trigger_mount_if_needed():
    try:
        # Get the list of all volumes (mounted and unmounted)
        result = subprocess.run(['gio', 'mount', '-li'], capture_output=True, text=True, timeout=5)
        
        # Find any MTP URIs in the output (both ActivationRoot and Mount lines)
        uris = re.findall(r'(mtp://[^\s]+)', result.stdout)
        
        # Force mount every URI we find
        for uri in uris:
            subprocess.run(['gio', 'mount', uri], capture_output=True, timeout=5)
            
    except Exception:
        pass

def get_connected_devices():
    devices = []
    uid = os.getuid()
    try:
        username = getpass.getuser()
    except Exception:
        username = "user"

    _trigger_mount_if_needed()
    
    search_paths = [
        Path(f"/run/user/{uid}/gvfs"),
        Path(f"/media/{username}"),
        Path(f"/run/media/{username}"),
    ]

    for base_path in search_paths:
        if not base_path.exists():
            continue
        try:
            for item in base_path.iterdir():
                if item.is_dir() and "mtp" in item.name.lower():
                    clean_name = item.name.replace("mtp:host=", "")
                    display_name = " ".join(clean_name.split("_")[:3]).title()
                    uri = f"mtp://{clean_name}/"
                    
                    if not any(d['path'] == str(item) for d in devices):
                        devices.append({
                            "name": display_name,
                            "id": uri,
                            "path": str(item)
                        })
        except Exception:
            pass
            
    return devices

def get_folders(device_path, relative_path=""):
    folders = []
    try:
        root = Path(device_path) / "Internal shared storage"
        if not root.exists():
            root = Path(device_path)
            
        target_dir = root / relative_path
        if not target_dir.exists():
            return []

        for item in target_dir.iterdir():
            if item.is_dir() and not item.name.startswith('.'):
                folders.append(item.name)
    except Exception:
        pass
    return sorted(folders)
