import pvm_host
from . import continuation as _continuation
from . import runtime

try:
    import rustpython_checkpoint as _checkpoint
except Exception:
    _checkpoint = None


RUNNER_ADDRESS = b"__runner__"


def continuation(*_args, **_kwargs):
    def decorator(func):
        return func
    return decorator


def _send_job(job_type, cid, reply_handler, *args, **kwargs):
    payload = {
        "kind": "runner_job",
        "job_type": job_type,
        "payload": {
            "args": list(args),
            "kwargs": kwargs,
        },
        "cid": cid,
        "reply_handler": reply_handler,
    }
    pvm_host.send_message(RUNNER_ADDRESS, _continuation.encode_payload(payload))


def _result_key(cid):
    return b"__runner_result:" + cid


def _try_get_result(cid):
    raw = pvm_host.get_state(_result_key(cid))
    if raw is None:
        return None
    pvm_host.delete_state(_result_key(cid))
    return _continuation.decode_payload(raw)


class _RunnerAwaitable:
    def __init__(self, job_type, *args, **kwargs):
        self.job_type = job_type
        self.args = args
        self.kwargs = kwargs
        self.cid = _continuation.new_cid(None, job_type)

    def __await__(self):
        if runtime.mode() != "checkpoint":
            raise RuntimeError("runner await is only supported in checkpoint mode without FSM")
        if False:
            yield None
        try_get_result = _try_get_result
        send_job = _send_job
        checkpoint = _checkpoint
        while True:
            result = try_get_result(self.cid)
            if result is not None:
                return result
            send_job(self.job_type, self.cid, "", *self.args, **self.kwargs)
            if checkpoint is None:
                raise RuntimeError("checkpoint support missing")
            checkpoint.checkpoint_bytes()


def _request_checkpoint():
    if _checkpoint is None:
        raise RuntimeError("checkpoint support missing")
    _checkpoint.checkpoint_bytes()


def llm(*args, **kwargs):
    return _RunnerAwaitable("llm", *args, **kwargs)


def http(*args, **kwargs):
    return _RunnerAwaitable("http", *args, **kwargs)
