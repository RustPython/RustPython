virtualenv venv --python=python3
source venv/bin/activate
pip install bytecode

source ~/.cargo/env
rustup install nightly
rustup default nightly
