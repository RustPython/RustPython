#!/usr/bin/env python

# Modified from https://github.com/agramian/custom-text-test-runner

# The MIT License (MIT)
#
# Copyright (c) 2015 Abtin Gramian
#
# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to deal
# in the Software without restriction, including without limitation the rights
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:
#
# The above copyright notice and this permission notice shall be included in all
# copies or substantial portions of the Software.
#
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
# OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
# SOFTWARE.


import inspect
import json
import operator
import os
import re
import sys
import time
import unittest
from functools import reduce
from unittest.runner import registerResult, result


def _subtest_id(parent, subtest):
    try:
        return subtest.id()
    except Exception as exc:
        # Parameter formatting must not change the test's outcome.
        return f"{parent.id()} (<subtest id unavailable: {type(exc).__name__}>)"


def _get_method_dict(test):
    """Get the __dict__ of the underlying function for a test method.

    Works for both bound methods (__func__.__dict__) and plain functions.
    """
    method = getattr(test, getattr(test, "_testMethodName", ""), None)
    func = getattr(method, "__func__", method)
    return getattr(func, "__dict__", {})


class TablePrinter(object):
    # Modified from https://github.com/agramian/table-printer, same license as above
    "Print a list of dicts as a table"

    def __init__(self, fmt, sep="", ul=None, tl=None, bl=None):
        """
        @param fmt: list of tuple(heading, key, width)
                        heading: str, column label
                        key: dictionary key to value to print
                        width: int, column width in chars
        @param sep: string, separation between columns
        @param ul: string, character to underline column label, or None for no underlining
        @param tl: string, character to draw as top line over table, or None
        @param bl: string, character to draw as bottom line under table, or None
        """
        super(TablePrinter, self).__init__()
        fmt = [x + ("left",) if len(x) < 4 else x for x in fmt]
        self.fmt = str(sep).join(
            "{lb}{0}:{align}{1}{rb}".format(
                key, width, lb="{", rb="}", align="<" if alignment == "left" else ">"
            )
            for heading, key, width, alignment in fmt
        )
        self.head = {key: heading for heading, key, width, alignment in fmt}
        self.ul = (
            {key: str(ul) * width for heading, key, width, alignment in fmt}
            if ul
            else None
        )
        self.width = {key: width for heading, key, width, alignment in fmt}
        self.tl = (
            {key: str(tl) * width for heading, key, width, alignment in fmt}
            if tl
            else None
        )
        self.bl = (
            {key: str(bl) * width for heading, key, width, alignment in fmt}
            if bl
            else None
        )

    def row(self, data, separation_character=False):
        if separation_character:
            return self.fmt.format(
                **{k: str(data.get(k, ""))[:w] for k, w in self.width.items()}
            )
        else:
            data = {
                k: str(data.get(k, ""))
                if len(str(data.get(k, ""))) <= w
                else "%s..." % str(data.get(k, ""))[: (w - 3)]
                for k, w in self.width.items()
            }
            return self.fmt.format(**data)

    def __call__(self, data_list, totals=None):
        _r = self.row
        res = [_r(data) for data in data_list]
        res.insert(0, _r(self.head))
        if self.ul:
            res.insert(1, _r(self.ul, True))
        if self.tl:
            res.insert(0, _r(self.tl, True))
        if totals:
            if self.ul:
                res.insert(len(res), _r(self.ul, True))
            res.insert(len(res), _r(totals))
        if self.bl:
            res.insert(len(res), _r(self.bl, True))
        return "\n".join(res)


def get_function_args(func_ref):
    try:
        return [p for p in inspect.getfullargspec(func_ref).args if p != "self"]
    except:
        return None


def store_class_fields(class_ref, args_passed):
    """Store the passed in class fields in self"""
    params = get_function_args(class_ref.__init__)
    for p in params:
        setattr(class_ref, p, args_passed[p])


def sum_dict_key(d, key, cast_type=None):
    """Sum together all values matching a key given a passed dict"""
    return reduce(
        (lambda x, y: x + y),
        [eval("%s(x['%s'])" % (cast_type, key)) if cast_type else x[key] for x in d],
    )


def case_name(name):
    """Test case name decorator to override function name."""

    def decorator(function):
        function.__dict__["test_case_name"] = name
        return function

    return decorator


def skip_device(name):
    """Decorator to mark a test to only run on certain devices
    Takes single device name or list of names as argument
    """

    def decorator(function):
        name_list = name if type(name) == list else [name]
        function.__dict__["skip_device"] = name_list
        return function

    return decorator


def _set_test_type(function, test_type):
    """Test type setter"""
    if "test_type" in function.__dict__:
        function.__dict__["test_type"].append(test_type)
    else:
        function.__dict__["test_type"] = [test_type]
    return function


def smoke(function):
    """Test decorator to mark test as smoke type"""
    return _set_test_type(function, "smoke")


def guide_discovery(function):
    """Test decorator to mark test as guide_discovery type"""
    return _set_test_type(function, "guide_discovery")


def focus(function):
    """Test decorator to mark test as focus type to all rspec style debugging of cases"""
    return _set_test_type(function, "focus")


class _WritelnDecorator(object):
    """Used to decorate file-like objects with a handy 'writeln' method"""

    def __init__(self, stream):
        self.stream = stream

    def __getattr__(self, attr):
        if attr in ("stream", "__getstate__"):
            raise AttributeError(attr)
        return getattr(self.stream, attr)

    def writeln(self, arg=None):
        if arg:
            self.write(arg)
        self.write("\n")  # text-mode streams translate to \r\n if needed


class CustomTextTestResult(result.TestResult):
    _num_formatting_chars = 150
    _execution_time_significant_digits = 4
    _pass_percentage_significant_digits = 2

    def __init__(
        self,
        stream,
        descriptions,
        verbosity,
        results_file_path,
        result_screenshots_dir,
        show_previous_results,
        config,
        test_types,
    ):
        super(CustomTextTestResult, self).__init__(stream, descriptions, verbosity)
        store_class_fields(self, locals())
        self.show_overall_results = verbosity > 0
        self.show_test_info = verbosity > 1
        self.show_individual_suite_results = verbosity > 2
        self.show_errors = verbosity > 3
        self.show_errors_detail = verbosity > 1
        self.show_all = verbosity > 4
        self.suite = None
        self.current_test = None
        self.total_execution_time = 0
        self.separator1 = "=" * CustomTextTestResult._num_formatting_chars
        self.separator2 = "-" * CustomTextTestResult._num_formatting_chars
        self.separator3 = "_" * CustomTextTestResult._num_formatting_chars
        self.separator4 = "*" * CustomTextTestResult._num_formatting_chars
        self.separator_failure = "!" * CustomTextTestResult._num_formatting_chars
        self.separator_pre_result = "." * CustomTextTestResult._num_formatting_chars

    def getDescription(self, test):
        doc_first_line = test.shortDescription()
        if self.descriptions and doc_first_line:
            return "\n".join((str(test), doc_first_line))
        else:
            return str(test)

    def getSuiteDescription(self, test):
        doc = test.__class__.__doc__
        return doc and doc.split("\n")[0].strip() or None

    def startTestRun(self):
        self.results = None
        self.previous_suite_runs = []
        if os.path.isfile(self.results_file_path):
            with open(self.results_file_path, "rb") as f:
                try:
                    self.results = json.load(f)
                    # recreated results dict with int keys
                    self.results["suites"] = {
                        int(k): v for (k, v) in list(self.results["suites"].items())
                    }
                    self.suite_map = {
                        v["name"]: int(k)
                        for (k, v) in list(self.results["suites"].items())
                    }
                    self.previous_suite_runs = list(self.results["suites"].keys())
                except:
                    pass
        if not self.results:
            self.results = {
                "suites": {},
                "name": "",
                "num_passed": 0,
                "num_failed": 0,
                "num_skipped": 0,
                "num_expected_failures": 0,
                "execution_time": None,
            }
        self.suite_number = (
            int(sorted(self.results["suites"].keys())[-1]) + 1
            if len(self.results["suites"])
            else 0
        )
        self.case_number = 0
        self.suite_map = {}

    def stopTestRun(self):
        # if no tests or some failure occurred execution time may not have been set
        try:
            self.results["suites"][self.suite_map[self.suite]]["execution_time"] = (
                format(
                    self.suite_execution_time,
                    ".%sf" % CustomTextTestResult._execution_time_significant_digits,
                )
            )
        except:
            pass
        self.results["execution_time"] = format(
            self.total_execution_time,
            ".%sf" % CustomTextTestResult._execution_time_significant_digits,
        )
        self.stream.writeln(self.separator3)
        with open(self.results_file_path, "w") as f:
            json.dump(self.results, f)

    def startTest(self, test):
        super().startTest(test)
        self._start_case(test)

    def _start_case(self, test):
        suite_base_category = (
            test.__class__.base_test_category
            if hasattr(test.__class__, "base_test_category")
            else ""
        )
        self.next_suite = os.path.join(
            suite_base_category,
            test.__class__.name
            if hasattr(test.__class__, "name")
            else test.__class__.__name__,
        )
        self.current_test = test
        self.case = getattr(test, "_testMethodName", test.id())
        if not self.suite or self.suite != self.next_suite:
            if self.suite:
                self.results["suites"][self.suite_map[self.suite]]["execution_time"] = (
                    format(
                        self.suite_execution_time,
                        ".%sf"
                        % CustomTextTestResult._execution_time_significant_digits,
                    )
                )
            self.suite_execution_time = 0
            self.suite = self.next_suite
            if self.show_test_info:
                self.stream.writeln(self.separator1)
                self.stream.writeln("TEST SUITE: %s" % self.suite)
                self.stream.writeln("Description: %s" % self.getSuiteDescription(test))
        try:
            name_override = _get_method_dict(test)["test_case_name"]
        except:
            name_override = None
        self.case = name_override if name_override else self.case
        if self.show_test_info:
            # self.stream.writeln(self.separator2)
            self.stream.write("CASE: %s" % test.id())
            if desc := test.shortDescription():
                self.stream.write(" (Description: %s)" % desc)
            self.stream.write("... ")
            # self.stream.writeln(self.separator2)
            self.stream.flush()
        self.current_case_number = self.case_number
        if self.suite not in self.suite_map:
            self.suite_map[self.suite] = self.suite_number
            self.results["suites"][self.suite_number] = {
                "name": self.suite,
                "class": test.__class__.__name__,
                "module": (
                    m.group(1)
                    if (m := re.compile(r".* \((.*)\)").match(str(test)))
                    else str(test)
                ),
                "description": self.getSuiteDescription(test),
                "cases": {},
                "used_case_names": {},
                "num_passed": 0,
                "num_failed": 0,
                "num_skipped": 0,
                "num_expected_failures": 0,
                "execution_time": None,
            }
            self.suite_number += 1
        self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.case_number
        ] = {
            "name": self.case,
            "method": getattr(test, "_testMethodName", test.id()),
            "id": test.id(),
            "subtests": [],
            "result": None,
            "description": test.shortDescription(),
            "note": None,
            "errors": None,
            "failures": None,
            "screenshots": [],
            "new_version": "No",
            "execution_time": None,
        }
        self.start_time = time.time()
        if self.test_types:
            if "test_type" in _get_method_dict(test) and set(
                [s.lower() for s in self.test_types]
            ) == set([s.lower() for s in _get_method_dict(test)["test_type"]]):
                pass
            else:
                _get_method_dict(test)["__unittest_skip_why__"] = (
                    'Test run specified to only run tests of type "%s"'
                    % ",".join(self.test_types)
                )
                _get_method_dict(test)["__unittest_skip__"] = True
        if "skip_device" in _get_method_dict(test):
            for device in _get_method_dict(test)["skip_device"]:
                if self.config and device.lower() in self.config["device_name"].lower():
                    _get_method_dict(test)["__unittest_skip_why__"] = (
                        "Test is marked to be skipped on %s" % device
                    )
                    _get_method_dict(test)["__unittest_skip__"] = True
                    break

    def stopTest(self, test):
        super().stopTest(test)
        self._finish_case()

    def _finish_case(self):
        case = self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]
        if case["result"] is None and any(
            subtest["result"] == "skipped" for subtest in case["subtests"]
        ):
            # A skipped subtest prevents unittest from calling addSuccess.
            case["result"] = "skipped"
            if self.show_test_info:
                self.stream.writeln("SKIPPED (subtest)")
        self.end_time = time.time()
        self.execution_time = self.end_time - self.start_time
        self.suite_execution_time += self.execution_time
        self.total_execution_time += self.execution_time
        # JSON/table totals count parent cases once. The standard unittest
        # summary still counts individual failure/error/skip events, including
        # subtests; keep those two metrics distinct.
        suite = self.results["suites"][self.suite_map[self.suite]]
        status = case["result"]
        outcome = {
            "passed": "passed",
            "failed": "failed",
            "error": "failed",
            "unexpected_success": "failed",
            "skipped": "skipped",
            "expected_failure": "expected_failures",
        }.get(status)
        if outcome is not None:
            key = "num_" + outcome
            suite[key] += 1
            self.results[key] += 1
        self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]["execution_time"] = format(
            self.execution_time,
            ".%sf" % CustomTextTestResult._execution_time_significant_digits,
        )
        self.case_number += 1
        self.current_test = None

    def addScreenshots(self, test):
        for root, dirs, files in os.walk(self.result_screenshots_dir):
            for file in files:
                self.results["suites"][self.suite_map[self.suite]]["cases"][
                    self.current_case_number
                ]["screenshots"].append(os.path.join(root, file))

    def addSuccess(self, test):
        super(CustomTextTestResult, self).addSuccess(test)
        if self.show_test_info:
            # self.stream.writeln(self.separator_pre_result)
            self.stream.writeln("PASS")
        self.stream.flush()
        self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]["result"] = "passed"
        self.addScreenshots(test)

    def addSubTest(self, test, subtest, err):
        super().addSubTest(test, subtest, err)
        case = self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]
        subtest_id = _subtest_id(test, subtest)
        status = "passed"
        traceback_text = None
        if err is not None:
            status = "failed" if issubclass(err[0], test.failureException) else "error"
            traceback_text = self._exc_info_to_string(err, subtest)
            if case["result"] != "error":
                case["result"] = status
            key = "failures" if status == "failed" else "errors"
            case[key] = (case[key] or "") + subtest_id + "\n" + traceback_text
            if self.show_test_info:
                self.stream.writeln("%s: %s" % (status.upper(), subtest_id))
        case["subtests"].append(
            {
                "id": subtest_id,
                "result": status,
                "traceback": traceback_text,
            }
        )

    def addError(self, test, err):
        fixture = self.current_test is None
        if fixture:
            self._start_case(test)
        error_string = self._exc_info_to_string(err, test)
        super(CustomTextTestResult, self).addError(test, err)
        if self.show_test_info:
            # self.stream.writeln(self.separator_pre_result)
            self.stream.writeln("ERROR")
        self.stream.flush()
        self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]["result"] = "error"
        self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]["errors"] = (
            self.results["suites"][self.suite_map[self.suite]]["cases"][
                self.current_case_number
            ]["errors"]
            or ""
        ) + error_string
        self.addScreenshots(test)
        if fixture:
            self._finish_case()

    def addFailure(self, test, err):
        error_string = self._exc_info_to_string(err, test)
        super(CustomTextTestResult, self).addFailure(test, err)
        if self.show_test_info:
            # self.stream.writeln(self.separator_pre_result)
            self.stream.writeln("FAIL")
        self.stream.flush()
        self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]["result"] = (
            "error"
            if self.results["suites"][self.suite_map[self.suite]]["cases"][
                self.current_case_number
            ]["errors"]
            else "failed"
        )
        self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]["failures"] = (
            self.results["suites"][self.suite_map[self.suite]]["cases"][
                self.current_case_number
            ]["failures"]
            or ""
        ) + error_string
        self.addScreenshots(test)

    def addSkip(self, test, reason):
        fixture = self.current_test is None
        if fixture:
            self._start_case(test)
        super(CustomTextTestResult, self).addSkip(test, reason)
        if test is not self.current_test:
            self.results["suites"][self.suite_map[self.suite]]["cases"][
                self.current_case_number
            ]["subtests"].append(
                {
                    "id": _subtest_id(self.current_test, test),
                    "result": "skipped",
                    "reason": reason,
                }
            )
            return
        case = self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]
        if case["result"] in ("failed", "error"):
            case["note"] = reason
            return
        if self.show_test_info:
            # self.stream.writeln(self.separator_pre_result)
            self.stream.writeln("SKIPPED {0!r}".format(reason))
        self.stream.flush()
        self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]["result"] = "skipped"
        self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]["note"] = reason
        if fixture:
            self._finish_case()

    def addExpectedFailure(self, test, err):
        super(CustomTextTestResult, self).addExpectedFailure(test, err)
        if self.show_test_info:
            # self.stream.writeln(self.separator_pre_result)
            self.stream.writeln("EXPECTED FAILURE")
        self.stream.flush()
        self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]["result"] = "expected_failure"
        self.addScreenshots(test)

    def addUnexpectedSuccess(self, test):
        super(CustomTextTestResult, self).addUnexpectedSuccess(test)
        if self.show_test_info:
            # self.stream.writeln(self.separator_pre_result)
            self.stream.writeln("UNEXPECTED SUCCESS")
        self.stream.flush()
        self.results["suites"][self.suite_map[self.suite]]["cases"][
            self.current_case_number
        ]["result"] = "unexpected_success"
        self.addScreenshots(test)

    def printOverallSuiteResults(self, r):
        self.stream.writeln()
        self.stream.writeln(self.separator4)
        self.stream.writeln("OVERALL SUITE RESULTS")
        fmt = [
            ("SUITE", "suite", 50, "left"),
            ("CASES", "cases", 15, "right"),
            ("PASSED", "passed", 15, "right"),
            ("FAILED", "failed", 15, "right"),
            ("SKIPPED", "skipped", 15, "right"),
            ("%", "percentage", 20, "right"),
            ("TIME (s)", "time", 20, "right"),
        ]
        data = []
        for x in r:
            data.append(
                {
                    "suite": r[x]["name"],
                    "cases": r[x]["num_passed"] + r[x]["num_failed"],
                    "passed": r[x]["num_passed"],
                    "failed": r[x]["num_failed"],
                    "skipped": r[x]["num_skipped"],
                    "expected_failures": r[x]["num_expected_failures"],
                    "percentage": float(r[x]["num_passed"])
                    / (r[x]["num_passed"] + r[x]["num_failed"])
                    * 100
                    if (r[x]["num_passed"] + r[x]["num_failed"]) > 0
                    else 0,
                    "time": r[x]["execution_time"],
                }
            )
        total_suites_passed = len([x for x in data if not x["failed"]])
        total_suites_passed_percentage = format(
            float(total_suites_passed) / len(data) * 100,
            ".%sf" % CustomTextTestResult._pass_percentage_significant_digits,
        )
        totals = {
            "suite": "TOTALS %s/%s (%s%%) suites passed"
            % (total_suites_passed, len(data), total_suites_passed_percentage),
            "cases": sum_dict_key(data, "cases"),
            "passed": sum_dict_key(data, "passed"),
            "failed": sum_dict_key(data, "failed"),
            "skipped": sum_dict_key(data, "skipped"),
            "percentage": sum_dict_key(data, "percentage") / len(data),
            "time": sum_dict_key(data, "time", "float"),
        }
        for x in data:
            operator.setitem(
                x,
                "percentage",
                format(
                    x["percentage"],
                    ".%sf" % CustomTextTestResult._pass_percentage_significant_digits,
                ),
            )
        totals["percentage"] = format(
            totals["percentage"],
            ".%sf" % CustomTextTestResult._pass_percentage_significant_digits,
        )
        self.stream.writeln(
            TablePrinter(
                fmt, tl=self.separator1, ul=self.separator2, bl=self.separator3
            )(data, totals)
        )
        self.stream.writeln()

    def printIndividualSuiteResults(self, r):
        self.stream.writeln()
        self.stream.writeln(self.separator4)
        self.stream.writeln("INDIVIDUAL SUITE RESULTS")
        fmt = [
            ("CASE", "case", 50, "left"),
            ("DESCRIPTION", "description", 50, "right"),
            ("RESULT", "result", 25, "right"),
            ("TIME (s)", "time", 25, "right"),
        ]
        for suite in r:
            self.stream.writeln(self.separator1)
            self.stream.write("{0: <50}".format("SUITE: %s" % r[suite]["name"]))
            self.stream.writeln(
                "{0: <100}".format(
                    "DESCRIPTION: %s"
                    % (
                        r[suite]["description"]
                        if not r[suite]["description"]
                        or len(r[suite]["description"]) <= (100 - len("DESCRIPTION: "))
                        else "%s..."
                        % r[suite]["description"][: (97 - len("DESCRIPTION: "))]
                    )
                )
            )
            data = []
            cases = r[suite]["cases"]
            for x in cases:
                data.append(
                    {
                        "case": cases[x]["name"],
                        "description": cases[x]["description"],
                        "result": cases[x]["result"].upper()
                        if cases[x]["result"]
                        else cases[x]["result"],
                        "time": cases[x]["execution_time"],
                    }
                )
            self.stream.writeln(
                TablePrinter(fmt, tl=self.separator1, ul=self.separator2)(data)
            )
        self.stream.writeln(self.separator3)
        self.stream.writeln()

    def printErrorsOverview(self, r):
        self.stream.writeln()
        self.stream.writeln(self.separator4)
        self.stream.writeln("FAILURES AND ERRORS OVERVIEW")
        fmt = [
            ("SUITE", "suite", 50, "left"),
            ("CASE", "case", 50, "left"),
            ("RESULT", "result", 50, "right"),
        ]
        data = []
        for suite in r:
            cases = {
                k: v
                for (k, v) in list(r[suite]["cases"].items())
                if v["failures"] or v["errors"]
            }
            for x in cases:
                data.append(
                    {
                        "suite": "%s%s"
                        % (
                            r[suite]["name"],
                            " (%s)" % r[suite]["module"]
                            if r[suite]["class"] != r[suite]["name"]
                            else "",
                        ),
                        "case": "%s%s"
                        % (
                            cases[x]["name"],
                            " (%s)" % cases[x]["method"]
                            if cases[x]["name"] != cases[x]["method"]
                            else "",
                        ),
                        "result": cases[x]["result"].upper(),
                    }
                )
        self.stream.writeln(
            TablePrinter(fmt, tl=self.separator1, ul=self.separator2)(data)
        )
        self.stream.writeln(self.separator3)
        self.stream.writeln()

    def printErrorsDetail(self, r):
        self.stream.writeln()
        self.stream.writeln(self.separator4)
        self.stream.writeln("FAILURES AND ERRORS DETAIL")
        for suite in r:
            failures_and_errors = [
                k
                for (k, v) in list(r[suite]["cases"].items())
                if v["failures"] or v["errors"]
            ]
            # print failures_and_errors
            suite_str = "%s%s" % (
                r[suite]["name"],
                " (%s)" % r[suite]["module"]
                if r[suite]["class"] != r[suite]["name"]
                else "",
            )
            for case in failures_and_errors:
                case_ref = r[suite]["cases"][case]
                case_str = case_ref.get("id", case_ref["method"])
                errors = case_ref["errors"]
                failures = case_ref["failures"]
                self.stream.writeln(self.separator1)
                if errors:
                    self.stream.writeln("ERROR: %s [%s]" % (case_str, suite_str))
                    self.stream.writeln(self.separator2)
                    self.stream.writeln(errors)
                if failures:
                    self.stream.writeln("FAILURE: %s [%s]" % (case_str, suite_str))
                    self.stream.writeln(self.separator2)
                    self.stream.writeln(failures)
        self.stream.writeln(self.separator3)
        self.stream.writeln()

    def printSkippedDetail(self, r):
        self.stream.writeln()
        self.stream.writeln(self.separator4)
        self.stream.writeln("SKIPPED DETAIL")
        fmt = [
            ("SUITE", "suite", 50, "left"),
            ("CASE", "case", 50, "left"),
            ("REASON", "reason", 50, "right"),
        ]
        data = []
        for suite in r:
            cases = {
                k: v
                for (k, v) in list(r[suite]["cases"].items())
                if v["result"] == "skipped"
            }
            for x in cases:
                data.append(
                    {
                        "suite": "%s%s"
                        % (
                            r[suite]["name"],
                            " (%s)" % r[suite]["module"]
                            if r[suite]["class"] != r[suite]["name"]
                            else "",
                        ),
                        "case": "%s%s"
                        % (
                            cases[x]["name"],
                            " (%s)" % cases[x]["method"]
                            if cases[x]["name"] != cases[x]["method"]
                            else "",
                        ),
                        "reason": cases[x]["note"],
                    }
                )
        self.stream.writeln(
            TablePrinter(fmt, tl=self.separator1, ul=self.separator2)(data)
        )
        self.stream.writeln(self.separator3)
        self.stream.writeln()

    def returnCode(self):
        return not self.wasSuccessful()


class CustomTextTestRunner(unittest.TextTestRunner):
    """A test runner class that displays results in textual form.
    It prints out the names of tests as they are run, errors as they
    occur, and a summary of the results at the end of the test run.
    """

    def __init__(
        self,
        stream=sys.stderr,
        descriptions=True,
        verbosity=1,
        failfast=False,
        buffer=False,
        resultclass=CustomTextTestResult,
        results_file_path="results.json",
        result_screenshots_dir="",
        show_previous_results=False,
        test_name=None,
        test_description=None,
        config=None,
        test_types=None,
    ):
        store_class_fields(self, locals())
        self.stream = _WritelnDecorator(stream)

    def _makeResult(self):
        return self.resultclass(
            self.stream,
            self.descriptions,
            self.verbosity,
            self.results_file_path,
            self.result_screenshots_dir,
            self.show_previous_results,
            self.config,
            self.test_types,
        )

    def run(self, test):
        output = ""
        "Run the given test case or test suite."
        result = self._makeResult()
        registerResult(result)
        result.failfast = self.failfast
        result.buffer = self.buffer
        startTime = time.time()
        startTestRun = getattr(result, "startTestRun", None)
        if startTestRun is not None:
            startTestRun()
        try:
            test(result)
        finally:
            stopTestRun = getattr(result, "stopTestRun", None)
            if stopTestRun is not None:
                stopTestRun()
        stopTime = time.time()
        timeTaken = stopTime - startTime
        # filter results to output
        if result.show_previous_results:
            r = result.results["suites"]
        else:
            r = {
                k: v
                for (k, v) in list(result.results["suites"].items())
                if k not in result.previous_suite_runs
            }
        # print results based on verbosity
        if result.show_all:
            result.printSkippedDetail(r)
        if result.show_errors_detail:
            result.printErrorsDetail(r)
        if result.show_individual_suite_results:
            result.printIndividualSuiteResults(r)
        if result.show_errors:
            result.printErrorsOverview(r)
        if result.show_overall_results:
            result.printOverallSuiteResults(r)
        run = result.testsRun
        self.stream.writeln(
            "Ran %d test case%s in %.4fs" % (run, run != 1 and "s" or "", timeTaken)
        )
        self.stream.writeln()

        expectedFails = unexpectedSuccesses = skipped = 0
        try:
            results = map(
                len,
                (result.expectedFailures, result.unexpectedSuccesses, result.skipped),
            )
        except AttributeError:
            pass
        else:
            expectedFails, unexpectedSuccesses, skipped = results

        infos = []
        if not result.wasSuccessful():
            self.stream.write("FAILED")
            failed, errored = map(len, (result.failures, result.errors))
            if failed:
                infos.append("failures=%d" % failed)
            if errored:
                infos.append("errors=%d" % errored)
        else:
            self.stream.write("OK")
        if skipped:
            infos.append("skipped=%d" % skipped)
        if expectedFails:
            infos.append("expected failures=%d" % expectedFails)
        if unexpectedSuccesses:
            infos.append("unexpected successes=%d" % unexpectedSuccesses)
        if infos:
            self.stream.writeln(" (%s)" % (", ".join(infos),))
        else:
            self.stream.write("\n")
        return result
