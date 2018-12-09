wasm-pack build --debug && \
cd pkg && \
npm link && \
cd ../app && \
npm install && \
npm link rustpython_wasm && \
node_modules/.bin/webpack-dev-server
