#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ -f "${SCRIPT_DIR}/rustify" ]]; then
  BIN_SRC="${SCRIPT_DIR}/rustify"
  DESKTOP_SRC="${SCRIPT_DIR}/rustify.desktop"
  ICON_SRC="${SCRIPT_DIR}/rustify.png"
elif [[ -f "${SCRIPT_DIR}/../target/release/Rustify" ]]; then
  BIN_SRC="${SCRIPT_DIR}/../target/release/Rustify"
  DESKTOP_SRC="${SCRIPT_DIR}/rustify.desktop"
  ICON_SRC="${SCRIPT_DIR}/../assets/rustify_256.png"
else
  echo "ERROR: Could not find compiled 'rustify' binary. Extract the release tarball or run 'cargo build --release' first." >&2
  exit 1
fi

BIN_DIR="${HOME}/.local/bin"
APP_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/256x256/apps"

install -Dm755 "${BIN_SRC}" "${BIN_DIR}/rustify"
install -Dm644 "${ICON_SRC}" "${ICON_DIR}/rustify.png"
mkdir -p "${APP_DIR}"
sed "s|^Exec=rustify$|Exec=${BIN_DIR}/rustify|" "${DESKTOP_SRC}" > "${APP_DIR}/rustify.desktop"
chmod 0644 "${APP_DIR}/rustify.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q "${APP_DIR}" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t -f "${HOME}/.local/share/icons/hicolor" || true
fi

echo "Rustify installed to ${BIN_DIR}/rustify"
echo "Desktop launcher installed to ${APP_DIR}/rustify.desktop"

case ":${PATH}:" in
  *":${BIN_DIR}:"*) ;;
  *)
    echo ""
    echo "NOTE: ${BIN_DIR} is not currently in your PATH."
    echo "You can launch Rustify from your application menu, or add this to ~/.bashrc / ~/.zshrc:"
    echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    ;;
esac
