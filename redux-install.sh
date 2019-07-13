#!/bin/bash

set -e

DIR=$(dirname "$0")

case $# in
0) ;;
1)
	cd "$1"
	;;
*)
	echo "Too many arguments" >&2
	exit 1
	;;
esac

if [[ ! -d cookbook ]]; then
	echo "You do not appear to be in a redox checkout (no 'cookbook' directory). " \
		"Please run this script from or specify as an argument the root of your redox checkout." >&2
	exit 1
fi

mkdir -p cookbook/recipes/rustpython

cp "$DIR"/redox_recipe.sh cookbook/recipes/rustpython/recipe.sh

if ! grep -q -w rustpython filesystem.toml; then
	sed -i 's/\[packages\]/[packages]\nrustpython = {}/' filesystem.toml
fi

echo "All done! Run 'make qemu' to rebuild and run with rustpython installed."
