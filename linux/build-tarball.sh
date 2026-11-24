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

PKG_NAME="rustify-${VERSION}-linux-x86_64"
PKG_DIR="${STAGE}/${PKG_NAME}"
mkdir -p "${PKG_DIR}"

install -Dm755 "${BIN_PATH}" "${PKG_DIR}/rustify"
install -Dm644 "${REPO_ROOT}/linux/rustify.desktop" "${PKG_DIR}/rustify.desktop"
install -Dm644 "${REPO_ROOT}/assets/rustify_256.png" "${PKG_DIR}/rustify.png"
install -Dm755 "${REPO_ROOT}/linux/install.sh" "${PKG_DIR}/install.sh"

cat > "${PKG_DIR}/uninstall.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

rm -f "${HOME}/.local/bin/rustify"
rm -f "${HOME}/.local/share/applications/rustify.desktop"
rm -f "${HOME}/.local/share/icons/hicolor/256x256/apps/rustify.png"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q "${HOME}/.local/share/applications" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t -f "${HOME}/.local/share/icons/hicolor" || true
fi

echo "Rustify has been removed from ~/.local."
EOF
chmod 0755 "${PKG_DIR}/uninstall.sh"

cat > "${PKG_DIR}/README.txt" <<EOF
Rustify v${VERSION} — Linux x86_64 Portable Package
==================================================

Quick Install (user-local, no sudo required):
  ./install.sh

Uninstall:
  ./uninstall.sh

Run directly without installing:
  ./rustify

Runtime Requirements:
  - ALSA (libasound2 / alsa-lib), GTK3, D-Bus, libxkbcommon, OpenGL (libgl1 / libglvnd), curl
  - Optional for Desktop System Audio Visualizer capture: pipewire-pulse or pulseaudio-utils

More info & updates:
  https://github.com/Ilay-dev/Rustify
EOF
chmod 0644 "${PKG_DIR}/README.txt"

mkdir -p "${REPO_ROOT}/dist"
OUT_TAR="${REPO_ROOT}/dist/${PKG_NAME}.tar.gz"
tar -czf "${OUT_TAR}" -C "${STAGE}" "${PKG_NAME}"

echo "Built portable Linux tarball: ${OUT_TAR}"
ls -lh "${OUT_TAR}"
