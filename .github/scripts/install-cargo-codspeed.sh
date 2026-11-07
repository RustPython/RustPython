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

if installed=$(cargo codspeed --version 2>/dev/null) && [ "${installed}" = "cargo-codspeed ${version#v}" ]; then
  echo "${installed} already installed"
  exit 0
fi

url="https://github.com/CodSpeedHQ/codspeed-rust/releases/download/${version}/cargo-codspeed-${target}"
bin="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "${bin}"
curl --fail --silent --show-error --location --output "${bin}/cargo-codspeed" "${url}"
chmod +x "${bin}/cargo-codspeed"
cargo codspeed --version
