#!/usr/bin/env bash
# Install a released cargo-codspeed executable into ~/.cargo/bin without
# compiling it. Usage: install-cargo-codspeed.sh v5.0.3
set -euo pipefail

version="${1:?usage: install-cargo-codspeed.sh <tag>}"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-musl ;;
  Linux-aarch64) target=aarch64-unknown-linux-musl ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  *) echo "no cargo-codspeed release for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

# `cargo codspeed --version` prints the version to stderr and exits non-zero,
# so only its output can be trusted.
installed_version() { cargo codspeed --version 2>&1 || true; }

want="cargo-codspeed ${version#v}"
if [ "$(installed_version)" = "${want}" ]; then
  echo "${want} already installed"
  exit 0
fi

url="https://github.com/CodSpeedHQ/codspeed-rust/releases/download/${version}/cargo-codspeed-${target}"
bin="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "${bin}"
curl --fail --silent --show-error --location --output "${bin}/cargo-codspeed" "${url}"
chmod +x "${bin}/cargo-codspeed"
got="$(installed_version)"
if [ "${got}" != "${want}" ]; then
  echo "expected ${want}, got '${got}'" >&2
  exit 1
fi
echo "${got}"
