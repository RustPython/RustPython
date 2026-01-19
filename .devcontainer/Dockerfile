FROM rust:bullseye

# Install clang
RUN apt-get update \
  && apt-get install -y clang \
  && rm -rf /var/lib/apt/lists/*
