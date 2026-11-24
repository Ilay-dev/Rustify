#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

VERSION="$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "([^"]+)"/\1/')"
if [[ -z "${VERSION}" ]]; then
  echo "ERROR: Could not determine package version from Cargo.toml" >&2
  exit 1
fi

BIN_PATH="${REPO_ROOT}/target/release/Rustify"
if [[ ! -f "${BIN_PATH}" ]]; then
  echo "Release binary not found at ${BIN_PATH}; building..."
  cargo build --release
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT

install -Dm755 "${BIN_PATH}" "${STAGE}/usr/bin/rustify"
install -Dm644 "${REPO_ROOT}/linux/rustify.desktop" "${STAGE}/usr/share/applications/rustify.desktop"
install -Dm644 "${REPO_ROOT}/assets/rustify_256.png" "${STAGE}/usr/share/icons/hicolor/256x256/apps/rustify.png"
install -Dm644 "${REPO_ROOT}/LICENSE" "${STAGE}/usr/share/doc/rustify/copyright"

INSTALLED_SIZE="$(du -sk "${STAGE}/usr" | awk '{print $1}')"

mkdir -p "${STAGE}/DEBIAN"
cat > "${STAGE}/DEBIAN/control" <<EOF
Package: rustify
Version: ${VERSION}
Section: sound
Priority: optional
Architecture: amd64
Maintainer: Ilay-dev <https://github.com/Ilay-dev/Rustify>
Homepage: https://github.com/Ilay-dev/Rustify
Depends: libasound2 | libasound2t64, libgtk-3-0 | libgtk-3-0t64, libdbus-1-3, libxkbcommon0, libgl1, curl
Recommends: pipewire-pulse | pulseaudio-utils
Installed-Size: ${INSTALLED_SIZE}
Description: Minimalist Dark Liquid-Glass Local Music Player & AI DJ Automixer
 Rustify is a fast, distraction-free desktop music player written in pure Rust
 featuring AI beat-matched DJ transitions, a 5-band Equalizer, -14 LUFS loudness
 normalization, synchronized lyrics, and a real-time AudioVisualizer.
EOF

cat > "${STAGE}/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi
exit 0
EOF

cat > "${STAGE}/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi
exit 0
EOF

chmod 0644 "${STAGE}/DEBIAN/control"
chmod 0755 "${STAGE}/DEBIAN/postinst" "${STAGE}/DEBIAN/postrm"

mkdir -p "${REPO_ROOT}/dist"
OUT_DEB="${REPO_ROOT}/dist/rustify_${VERSION}_amd64.deb"
dpkg-deb --build --root-owner-group "${STAGE}" "${OUT_DEB}"

echo "Built Debian/Ubuntu/Mint package: ${OUT_DEB}"
ls -lh "${OUT_DEB}"
