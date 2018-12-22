set -e
(cd lib; wasm-pack build)
(cd demo; npm install && node_modules/.bin/webpack --mode production)
echo "Output saved to demo/dist"
