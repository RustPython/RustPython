import pvm_host


def time():
    return pvm_host.context()["timestamp_ms"] / 1000.0


def time_ns():
    return pvm_host.context()["timestamp_ms"] * 1_000_000
