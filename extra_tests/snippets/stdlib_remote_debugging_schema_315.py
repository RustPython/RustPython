"""Run unchanged on both runtimes to compare the exported data schemas."""

import json

import _remote_debugging as remote

location = remote.LocationInfo((10, None, 3, None))
assert tuple(location) == (10, None, 3, None)
assert (
    location.lineno,
    location.end_lineno,
    location.col_offset,
    location.end_col_offset,
) == tuple(location)
frame = remote.FrameInfo(("example.py", location, "function", None))
assert frame.location is location
assert (frame.filename, frame.funcname, frame.opcode) == (
    "example.py",
    "function",
    None,
)
synthetic = remote.FrameInfo(("", None, "synthetic", None))
assert synthetic.location is None
thread = remote.ThreadInfo((123, 0, [frame]))
assert thread.thread_id == 123 and thread.status == 0
assert thread.frame_info == [frame]
print(
    json.dumps(
        {
            "location": list(location),
            "frame": [
                frame.filename,
                list(frame.location),
                frame.funcname,
                frame.opcode,
            ],
            "synthetic": list(synthetic),
            "thread": [thread.thread_id, thread.status],
        }
    )
)
