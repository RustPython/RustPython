# perf_ci axis: import machinery cost for a few common stdlib modules.
# Fresh process per run, so these are real (uncached) imports.
import collections
import functools
import itertools
import json
import re

print(len((collections, functools, itertools, json, re)))
