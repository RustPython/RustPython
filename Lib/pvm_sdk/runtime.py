import pvm_host


def mode():
    cfg = pvm_host.runtime_config()
    return cfg.get("continuation_mode", "fsm")
