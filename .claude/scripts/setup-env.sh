#!/bin/bash
# Claude Code web session startup script
# Sets up the development environment for RustPython

set -e

cd /home/user/RustPython

echo "=== RustPython dev environment setup ==="

# 1. Ensure python3 points to 3.13+ (needed for scripts/update_lib)
#    /usr/local/bin takes precedence over /usr/bin in PATH,
#    so we update the symlink there directly.
CURRENT_PY=$(python3 --version 2>&1 | grep -oP '\d+\.\d+')
if [ "$(printf '%s\n' "3.13" "$CURRENT_PY" | sort -V | head -1)" != "3.13" ]; then
    echo "Upgrading python3 default to 3.13..."
    # Find best available Python >= 3.13
    TARGET=""
    for ver in python3.14 python3.13; do
        if command -v "$ver" &>/dev/null; then
            TARGET=$(command -v "$ver")
            break
        fi
    done
    if [ -n "$TARGET" ]; then
        # Override /usr/local/bin/python3 if it exists and is outdated
        if [ -e /usr/local/bin/python3 ]; then
            sudo ln -sf "$TARGET" /usr/local/bin/python3
        fi
        # Also set /usr/bin via update-alternatives
        sudo update-alternatives --install /usr/bin/python3 python3 "$TARGET" 3 2>/dev/null || true
        sudo update-alternatives --set python3 "$TARGET" 2>/dev/null || true
        echo "python3 now: $(python3 --version)"
    else
        echo "WARNING: No Python 3.13+ found. scripts/update_lib may not work."
    fi
else
    echo "python3 already >= 3.13: $(python3 --version)"
fi

# 2. Clone CPython source if not present (needed for scripts/update_lib)
if [ ! -d "cpython" ]; then
    echo "Cloning CPython v3.14.3 (shallow)..."
    git clone --depth 1 --branch v3.14.3 https://github.com/python/cpython.git cpython
    echo "CPython source ready."
else
    echo "CPython source already present."
fi

echo "=== Setup complete ==="
