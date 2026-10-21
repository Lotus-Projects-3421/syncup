import os
import json
from pathlib import Path

def get_app_data_dir():
    if os.name == 'nt':
        base = Path(os.getenv('APPDATA', Path.home() / 'AppData' / 'Roaming'))
    elif os.uname().sysname == 'Darwin':
        base = Path.home() / 'Library' / 'Application Support'
    else:
        base = Path(os.getenv('XDG_CONFIG_HOME', Path.home() / '.config'))
    app_dir = base / "SyncUp"
    app_dir.mkdir(parents=True, exist_ok=True)
    return app_dir

def load_global_settings():
    settings_file = get_app_data_dir() / "settings.json"
    if settings_file.exists():
        try:
            with open(settings_file, 'r') as f:
                return json.load(f)
        except: pass
    return {"window_width": 1200, "window_height": 950}

def save_global_settings(data):
    settings_file = get_app_data_dir() / "settings.json"
    with open(settings_file, 'w') as f:
        json.dump(data, f, indent=4)

# Keep the profile/state functions from before
def load_profile(device_id):
    safe_id = "".join([c for c in device_id if c.isalnum() or c in ('-', '_')])
    profile_file = get_app_data_dir() / f"profile_{safe_id}.json"
    if profile_file.exists():
        try:
            with open(profile_file, 'r') as f: return json.load(f)
        except: pass
    return None

def save_profile(device_id, profile_data):
    safe_id = "".join([c for c in device_id if c.isalnum() or c in ('-', '_')])
    profile_file = get_app_data_dir() / f"profile_{safe_id}.json"
    with open(profile_file, 'w') as f: json.dump(profile_data, f, indent=4)

def get_state_file(device_id):
    safe_id = "".join([c for c in device_id if c.isalnum() or c in ('-', '_')])
    return get_app_data_dir() / f"state_{safe_id}.json"

def load_state(device_id):
    state_file = get_state_file(device_id)
    if state_file.exists():
        try:
            with open(state_file, 'r') as f: return set(json.load(f))
        except: pass
    return set()

def save_state(device_id, processed_set):
    state_file = get_state_file(device_id)
    with open(state_file, 'w') as f: json.dump(list(processed_set), f)
