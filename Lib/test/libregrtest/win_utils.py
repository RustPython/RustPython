import _overlapped
import _thread
import _winapi
import math
import struct
import winreg


# Seconds per measurement
SAMPLING_INTERVAL = 1
# Exponential damping factor to compute exponentially weighted moving average
# on 1 minute (60 seconds)
LOAD_FACTOR_1 = 1 / math.exp(SAMPLING_INTERVAL / 60)
# Initialize the load using the arithmetic mean of the first NVALUE values
# of the Processor Queue Length
NVALUE = 5


class WindowsLoadTracker():
    """
    This class asynchronously reads the performance counters to calculate
    the system load on Windows.  A "raw" thread is used here to prevent
    interference with the test suite's cases for the threading module.
    """

    def __init__(self):
        # make __del__ not fail if pre-flight test fails
        self._running = None
        self._stopped = None

        # Pre-flight test for access to the performance data;
        # `PermissionError` will be raised if not allowed
        winreg.QueryInfoKey(winreg.HKEY_PERFORMANCE_DATA)

        self._values = []

        _thread.start_new_thread(self._update_load, (), {})

    def _update_load(self,
                    # localize module access to prevent shutdown errors
                     _wait, _signal):
        pass

    def _calculate_load(self,
                        # localize module access to prevent shutdown errors
                        _query=winreg.QueryValueEx,
                        _hkey=winreg.HKEY_PERFORMANCE_DATA,
                        _unpack=struct.unpack_from):
        pass

    def close(self, kill=True):
        self.__del__()
        return

    def __del__(self, _wait, _close, _signal):
        pass

    def getloadavg(self):
        return None
