import traceback

try:
	1/0
except ZeroDivisionError as ex:
	tb = traceback.format_list(traceback.extract_tb(ex.__traceback__))
	assert len(tb) == 1
