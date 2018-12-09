# The second cp is due to https://github.com/rustwasm/wasm-pack/issues/240
wasm-pack build --debug && \
cp app/html-console.js pkg \
cd pkg && \
npm link && \
cd ../app && \
npm install && \
npm link rustpython_wasm && \
node_modules/.bin/webpack-dev-server
