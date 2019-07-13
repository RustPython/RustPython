GIT=https://github.com/RustPython/RustPython
BRANCH=redox
CARGOFLAGS=--no-default-features

function recipe_stage() {
  dest="$(realpath "$1")"
  mkdir -pv "$dest/lib/"
  cp -r Lib "$dest/lib/rustpython"
}
