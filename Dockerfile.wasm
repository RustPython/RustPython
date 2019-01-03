FROM rust:1.31-slim

RUN apt-get update && apt-get install curl gnupg -y && \
    curl -o- https://deb.nodesource.com/setup_10.x | bash && \
    apt-get install nodejs -y && \
    curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh && \
    npm i -g serve

WORKDIR /rustpython

COPY . .

RUN cd ./wasm/lib/ && \
    cargo build --release && \
    cd ../demo && \
    npm install && \
    npm run dist

CMD [ "serve", "/rustpython/wasm/demo/dist" ]