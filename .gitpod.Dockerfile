FROM gitpod/workspace-full

USER gitpod

# Update Rust to the latest version
RUN rm -rf ~/.rustup && \
    export PATH=$HOME/.cargo/bin:$PATH && \
    rustup update stable && \
    rustup component add rls && \
    # Set up wasm-pack and wasm32-unknown-unknown for rustpython_wasm
    curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh && \
    rustup target add wasm32-unknown-unknown

USER root
