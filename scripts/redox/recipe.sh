GIT=https://github.com/RustPython/RustPython
BRANCH=redox-release
CARGOFLAGS=--no-default-features
export BUILDTIME_RUSTPYTHONPATH=/lib/rustpython/

function recipe_stage() {
  dest="$(realpath "$1")"
  mkdir -pv "$dest/lib/"
  cp -r Lib "$dest/lib/rustpython"
}
