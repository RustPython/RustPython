FROM mcr.microsoft.com/vscode/devcontainers/rust:1-bullseye

# Install clang
RUN apt-get update \
  && apt-get install -y clang \
  && rm -rf /var/lib/apt/lists/*
