FROM rust:1.31-slim

WORKDIR /rustpython

COPY . .

RUN cargo build --release

CMD [ "/rustpython/target/release/rustpython" ]
