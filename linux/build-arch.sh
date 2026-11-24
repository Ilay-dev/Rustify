#!/usr/bin/env bash
set -euo pipefail

if [[ "$(id -u)" -eq 0 ]]; then
  echo "ERROR: build-arch.sh must be run as a non-root user (makepkg refuses root)." >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

VERSION="$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "([^"]+)"/\1/')"
if [[ -z "${VERSION}" ]]; then
  echo "ERROR: Could not determine package version from Cargo.toml" >&2
  exit 1
fi

BUILD_DIR="$(mktemp -d)"
trap 'rm -rf "${BUILD_DIR}"' EXIT

cp "${REPO_ROOT}/linux/PKGBUILD" "${BUILD_DIR}/PKGBUILD"
sed -i -E "s/^pkgver=.*/pkgver=${VERSION}/" "${BUILD_DIR}/PKGBUILD"

# Package current repository snapshot into the exact source tarball expected by PKGBUILD
tar -czf "${BUILD_DIR}/rustify-${VERSION}.tar.gz" \
  --exclude='./.git' \
  --exclude='./target' \
  --exclude='./dist' \
  --exclude='./Covers' \
  --exclude='*.exe' \
  --exclude='*.pdb' \
  --transform="s,^\.,Rustify-${VERSION}," \
  .

(
  cd "${BUILD_DIR}"
  makepkg -f --noconfirm --clean
)

mkdir -p "${REPO_ROOT}/dist"
cp -f "${BUILD_DIR}"/rustify-"${VERSION}"-*-x86_64.pkg.tar.zst "${REPO_ROOT}/dist/"

echo "Built Arch Linux package(s):"
ls -lh "${REPO_ROOT}/dist"/rustify-"${VERSION}"-*-x86_64.pkg.tar.zst
