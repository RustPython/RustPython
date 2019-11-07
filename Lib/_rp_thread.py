import _thread
import _dummy_thread

for k in _dummy_thread.__all__ + ['_set_sentinel', 'stack_size']:
    if k not in _thread.__dict__:
        # print('Populating _thread.%s' % k)
        setattr(_thread, k, getattr(_dummy_thread, k))
