import traceback

try:
	1/0
except ZeroDivisionError as ex:
	traceback.print_tb(ex.__traceback__)