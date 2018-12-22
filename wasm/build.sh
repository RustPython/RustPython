set -e
(cd lib; wasm-pack build --debug)
(cd demo; npm install && node_modules/.bin/webpack-dev-server)
