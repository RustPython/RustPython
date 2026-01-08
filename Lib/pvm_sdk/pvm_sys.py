import pvm_host

_ctx = pvm_host.context()

chain_id = _ctx.get("chain_id")
pvm_version = _ctx.get("pvm_version")
stdlib_hash = _ctx.get("stdlib_hash")
