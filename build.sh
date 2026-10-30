#!/usr/bin/env bash
#
# Build a versioned AppImage from source.
#
# Only generated build output (build/, dist/, AppDir/) is removed.  Source
# files, user profiles, and existing AppImages in the project root are kept.
set -euo pipefail

cd "$(dirname "$0")"

if [[ -z "${PYTHON:-}" ]]; then
    if [[ -x .venv/bin/python ]]; then
        PYTHON=".venv/bin/python"
    else
        PYTHON="python3"
    fi
fi

if [[ ! -x ./appimagetool ]]; then
    echo "error: ./appimagetool is missing or not executable" >&2
    exit 1
fi
if ! "$PYTHON" -c 'import PyInstaller' >/dev/null 2>&1; then
    echo "error: PyInstaller is not installed for $PYTHON" >&2
    echo "       run: $PYTHON -m pip install -r requirements-build.txt" >&2
    exit 1
fi

VERSION="$("$PYTHON" -c 'from core.constants import VERSION; print(VERSION)')"
FLET_VERSION="$("$PYTHON" -c 'from flet_desktop.version import version; print(version)')"
DESKTOP_NAME="io.github.lotus_projects_3421.SyncUp.desktop"

echo "==> SyncUp version ${VERSION} (Flet ${FLET_VERSION})"
# echo "==> Generating icon"
# "$PYTHON" scripts/make_icon.py

echo "==> Running tests"
"$PYTHON" -m unittest discover -s tests -t . -v

echo "==> Building with PyInstaller"
"$PYTHON" -m PyInstaller --noconfirm --clean SyncUp.spec

echo "==> Assembling AppDir"
rm -rf AppDir
mkdir -p AppDir/usr/bin
cp -r dist/SyncUp/. AppDir/usr/bin/
cp assets/syncup.png AppDir/syncup.png
cp assets/syncup.png AppDir/.DirIcon
sed "s/@VERSION@/${VERSION}/" "packaging/${DESKTOP_NAME}" > "AppDir/${DESKTOP_NAME}"
mkdir -p AppDir/usr/share/applications AppDir/usr/share/metainfo
cp "AppDir/${DESKTOP_NAME}" "AppDir/usr/share/applications/${DESKTOP_NAME}"
sed "s/@VERSION@/${VERSION}/" \
    packaging/io.github.lotus_projects_3421.SyncUp.appdata.xml \
    > AppDir/usr/share/metainfo/io.github.lotus_projects_3421.SyncUp.appdata.xml
mkdir -p AppDir/usr/share/icons/hicolor/512x512/apps
cp assets/syncup.png AppDir/syncup.png
cp assets/syncup.png AppDir/.DirIcon
cp assets/syncup.png AppDir/usr/share/icons/hicolor/512x512/apps/syncup.png

# Bundle the cached Flet desktop runtime when available.  Without this the
# first launch may try to download it (and a frozen PyInstaller app cannot use
# pip to install the Python-side launcher).
FLET_RUNTIME="${FLET_VIEW_PATH:-$HOME/.flet/client/flet-desktop-light-${FLET_VERSION}/flet}"
if [[ -x "$FLET_RUNTIME/flet" ]]; then
    mkdir -p AppDir/usr/lib/syncup/flet
    cp -a "$FLET_RUNTIME/." AppDir/usr/lib/syncup/flet/
    echo "==> Bundled Flet desktop runtime from ${FLET_RUNTIME}"
else
    echo "warning: no cached Flet desktop runtime found at ${FLET_RUNTIME}" >&2
    echo "         the AppImage will download it on first launch" >&2
fi

cat > AppDir/AppRun <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
if [ -x "$HERE/usr/lib/syncup/flet/flet" ]; then
    export FLET_VIEW_PATH="$HERE/usr/lib/syncup/flet"
fi
exec "$HERE/usr/bin/SyncUp" "$@"
EOF
chmod +x AppDir/AppRun

OUTPUT="dist/SyncUp_v${VERSION}.AppImage"
echo "==> Packaging ${OUTPUT}"
ARCH=x86_64 ./appimagetool --appimage-extract-and-run AppDir "$OUTPUT"

echo "==> Done: ${OUTPUT}"
