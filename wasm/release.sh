wasm-pack build && \
cd pkg && \
npm link && \
cd ../app && \
npm install && \
npm link rustpython_wasm && \
webpack --mode production && \
echo "Output saved to app/dist"
