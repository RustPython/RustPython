wasm-pack build && \
cd pkg && \
npm link && \
cd ../app && \
npm install && \
npm link rustpython_wasm && \
node_modules/.bin/webpack --mode production && \
echo "Output saved to app/dist"
