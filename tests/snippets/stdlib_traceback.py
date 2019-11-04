import traceback

try:
	1/0
except ZeroDivisionError as ex:
	tb = traceback.extract_tb(ex.__traceback__)
	assert len(tb) == 1


try:
	try:
		1/0
	except ZeroDivisionError as ex:
		 raise KeyError().with_traceback(ex.__traceback__)
except KeyError as ex2:
	tb = traceback.extract_tb(ex2.__traceback__)
	assert tb[1].line == "1/0"


try:
	try:
		1/0
	except ZeroDivisionError as ex:
		 raise ex.with_traceback(None)
except ZeroDivisionError as ex2:
	tb = traceback.extract_tb(ex2.__traceback__)
	assert len(tb) == 1
