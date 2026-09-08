#!/bin/sh
set -eu

repo="thalixinc/thalix-pstack"
install_dir="${PSTACK_INSTALL_DIR:-${HOME}/.local/bin}"
requested_version="${PSTACK_VERSION:-latest}"

case "$(uname -s)" in
  Darwin) platform="apple-darwin" ;;
  Linux) platform="unknown-linux-gnu" ;;
  *)
    printf 'pstack: unsupported operating system: %s\n' "$(uname -s)" >&2
    exit 1
    ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) architecture="x86_64" ;;
  arm64 | aarch64) architecture="aarch64" ;;
  *)
    printf 'pstack: unsupported architecture: %s\n' "$(uname -m)" >&2
    exit 1
    ;;
esac

target="${architecture}-${platform}"
archive="pstack-${target}.tar.gz"

if [ "$requested_version" = "latest" ]; then
  release_base="https://github.com/${repo}/releases/latest/download"
else
  case "$requested_version" in
    v*) release_tag="$requested_version" ;;
    *) release_tag="v${requested_version}" ;;
  esac
  release_base="https://github.com/${repo}/releases/download/${release_tag}"
fi

temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/pstack-install.XXXXXX")"
trap 'rm -rf "$temp_dir"' EXIT HUP INT TERM

curl --proto '=https' --tlsv1.2 -fsSL \
  "${release_base}/${archive}" -o "${temp_dir}/${archive}"
curl --proto '=https' --tlsv1.2 -fsSL \
  "${release_base}/SHA256SUMS" -o "${temp_dir}/SHA256SUMS"

expected="$(awk -v name="$archive" '$2 == name || $2 == "*" name { print $1; exit }' "${temp_dir}/SHA256SUMS")"
if [ -z "$expected" ]; then
  printf 'pstack: release checksum is missing for %s\n' "$archive" >&2
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "${temp_dir}/${archive}" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "${temp_dir}/${archive}" | awk '{print $1}')"
else
  printf 'pstack: sha256sum or shasum is required to verify the download\n' >&2
  exit 1
fi

if [ "$actual" != "$expected" ]; then
  printf 'pstack: checksum mismatch for %s\n' "$archive" >&2
  exit 1
fi

tar -xzf "${temp_dir}/${archive}" -C "$temp_dir"
if [ ! -f "${temp_dir}/pstack" ]; then
  printf 'pstack: release archive does not contain the pstack binary\n' >&2
  exit 1
fi

mkdir -p "$install_dir"
install -m 0755 "${temp_dir}/pstack" "${install_dir}/pstack"
"${install_dir}/pstack" version

case ":${PATH}:" in
  *":${install_dir}:"*) ;;
  *) printf 'Add %s to PATH to run pstack from any shell.\n' "$install_dir" ;;
esac
