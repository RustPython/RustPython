# NOT_RPYTHON
"""
A pure Python reimplementation of the _sre module from CPython 2.4
Copyright 2005 Nik Haldimann, licensed under the MIT license

This code is based on material licensed under CNRI's Python 1.6 license and
copyrighted by: Copyright (c) 1997-2001 by Secret Labs AB
"""

import array, operator, sys
from sre_constants import ATCODES, OPCODES, CHCODES, MAXREPEAT
from sre_constants import SRE_INFO_PREFIX, SRE_INFO_LITERAL
from sre_constants import SRE_FLAG_UNICODE, SRE_FLAG_LOCALE
import sre_constants


import sys

# Identifying as _sre from Python 2.3 or 2.4
MAGIC = 20140917

# In _sre.c this is bytesize of the code word type of the C implementation.
# There it's 2 for normal Python builds and more for wide unicode builds (large
# enough to hold a 32-bit UCS-4 encoded character). Since here in pure Python
# we only see re bytecodes as Python longs, we shouldn't have to care about the
# codesize. But sre_compile will compile some stuff differently depending on the
# codesize (e.g., charsets).
if sys.maxunicode == 65535:
    CODESIZE = 2
else:
    CODESIZE = 4

copyright = "_sre.py 2.4c Copyright 2005 by Nik Haldimann"


def getcodesize():
    return CODESIZE


def compile(pattern, flags, code, groups=0, groupindex={}, indexgroup=[None]):
    """Compiles (or rather just converts) a pattern descriptor to a SRE_Pattern
    object. Actual compilation to opcodes happens in sre_compile."""
    return SRE_Pattern(pattern, flags, code, groups, groupindex, indexgroup)

def getlower(char_ord, flags):
    if (char_ord < 128) or (flags & SRE_FLAG_UNICODE) \
                              or (flags & SRE_FLAG_LOCALE and char_ord < 256):
        return ord(chr(char_ord).lower())
    else:
        return char_ord


class SRE_Pattern(object):

    def __init__(self, pattern, flags, code, groups=0, groupindex={}, indexgroup=[None]):
        self.pattern = pattern
        self.flags = flags
        self.groups = groups
        self.groupindex = groupindex # Maps group names to group indices
        self._indexgroup = indexgroup # Maps indices to group names
        self._code = code

    def match(self, string, pos=0, endpos=sys.maxsize):
        """If zero or more characters at the beginning of string match this
        regular expression, return a corresponding MatchObject instance. Return
        None if the string does not match the pattern."""
        state = _State(string, pos, endpos, self.flags)
        if state.match(self._code):
            return SRE_Match(self, state)
        else:
            return None

    def fullmatch(self, string, pos=0, endpos=sys.maxsize):
        """If the whole string matches the regular expression pattern, return a
        corresponding match object. Return None if the string does not match the
        pattern; note that this is different from a zero-length match."""
        match = self.match(string, pos, endpos)
        if match and match.start() == pos and match.end() == min(endpos, len(string)):
            return match
        else:
            return None

    def search(self, string, pos=0, endpos=sys.maxsize):
        """Scan through string looking for a location where this regular
        expression produces a match, and return a corresponding MatchObject
        instance. Return None if no position in the string matches the
        pattern."""
        state = _State(string, pos, endpos, self.flags)
        if state.search(self._code):
            return SRE_Match(self, state)
        else:
            return None

    def findall(self, string, pos=0, endpos=sys.maxsize):
        """Return a list of all non-overlapping matches of pattern in string."""
        matchlist = []
        state = _State(string, pos, endpos, self.flags)
        while state.start <= state.end:
            state.reset()
            state.string_position = state.start
            if not state.search(self._code):
                break
            match = SRE_Match(self, state)
            if self.groups == 0 or self.groups == 1:
                item = match.group(self.groups)
            else:
                item = match.groups("")
            matchlist.append(item)
            if state.string_position == state.start:
                state.start += 1
            else:
                state.start = state.string_position
        return matchlist

    def _subx(self, template, string, count=0, subn=False):
        filter = template
        if not callable(template) and "\\" in template:
            # handle non-literal strings ; hand it over to the template compiler
            import re
            filter = re._subx(self, template)
        state = _State(string, 0, sys.maxsize, self.flags)
        sublist = []

        n = last_pos = 0
        while not count or n < count:
            state.reset()
            state.string_position = state.start
            if not state.search(self._code):
                break
            if last_pos < state.start:
                sublist.append(string[last_pos:state.start])
            if not (last_pos == state.start and
                                last_pos == state.string_position and n > 0):
                # the above ignores empty matches on latest position
                if callable(filter):
                    sublist.extend(filter(SRE_Match(self, state)))
                else:
                    sublist.append(filter)
                last_pos = state.string_position
                n += 1
            if state.string_position == state.start:
                state.start += 1
            else:
                state.start = state.string_position

        if last_pos < state.end:
            sublist.append(string[last_pos:state.end])
        item = "".join(sublist)
        if subn:
            return item, n
        else:
            return item

    def sub(self, repl, string, count=0):
        """Return the string obtained by replacing the leftmost non-overlapping
        occurrences of pattern in string by the replacement repl."""
        return self._subx(repl, string, count, False)

    def subn(self, repl, string, count=0):
        """Return the tuple (new_string, number_of_subs_made) found by replacing
        the leftmost non-overlapping occurrences of pattern with the replacement
        repl."""
        return self._subx(repl, string, count, True)

    def split(self, string, maxsplit=0):
        """Split string by the occurrences of pattern."""
        splitlist = []
        state = _State(string, 0, sys.maxsize, self.flags)
        n = 0
        last = state.start
        while not maxsplit or n < maxsplit:
            state.reset()
            state.string_position = state.start
            if not state.search(self._code):
                break
            if state.start == state.string_position: # zero-width match
                if last == state.end:                # or end of string
                    break
                state.start += 1
                continue
            splitlist.append(string[last:state.start])
            # add groups (if any)
            if self.groups:
                match = SRE_Match(self, state)
                splitlist.extend(list(match.groups(None)))
            n += 1
            last = state.start = state.string_position
        splitlist.append(string[last:state.end])
        return splitlist

    def finditer(self, string, pos=0, endpos=sys.maxsize):
        """Return a list of all non-overlapping matches of pattern in string."""
        scanner = self.scanner(string, pos, endpos)
        return iter(scanner.search, None)

    def scanner(self, string, pos=0, endpos=sys.maxsize):
        return SRE_Scanner(self, string, pos, endpos)

    def __copy__(self):
        raise TypeError("cannot copy this pattern object")

    def __deepcopy__(self):
        raise TypeError("cannot copy this pattern object")


class SRE_Scanner(object):
    """Undocumented scanner interface of sre."""

    def __init__(self, pattern, string, start, end):
        self.pattern = pattern
        self._state = _State(string, start, end, self.pattern.flags)

    def _match_search(self, matcher):
        state = self._state
        state.reset()
        state.string_position = state.start
        match = None
        if matcher(self.pattern._code):
            match = SRE_Match(self.pattern, state)
        if match is None or state.string_position == state.start:
            state.start += 1
        else:
            state.start = state.string_position
        return match

    def match(self):
        return self._match_search(self._state.match)

    def search(self):
        return self._match_search(self._state.search)


class SRE_Match(object):

    def __init__(self, pattern, state):
        self.re = pattern
        self.string = state.string
        self.pos = state.pos
        self.endpos = state.end
        self.lastindex = state.lastindex
        self.regs = self._create_regs(state)
        if pattern._indexgroup and 0 <= self.lastindex < len(pattern._indexgroup):
            # The above upper-bound check should not be necessary, as the re
            # compiler is supposed to always provide an _indexgroup list long
            # enough. But the re.Scanner class seems to screw up something
            # there, test_scanner in test_re won't work without upper-bound
            # checking. XXX investigate this and report bug to CPython.
            self.lastgroup = pattern._indexgroup[self.lastindex]
        else:
            self.lastgroup = None

    def __getitem__(self, rank):
        return self.group(rank)

    def _create_regs(self, state):
        """Creates a tuple of index pairs representing matched groups."""
        regs = [(state.start, state.string_position)]
        for group in range(self.re.groups):
            mark_index = 2 * group
            if mark_index + 1 < len(state.marks) \
                                    and state.marks[mark_index] is not None \
                                    and state.marks[mark_index + 1] is not None:
                regs.append((state.marks[mark_index], state.marks[mark_index + 1]))
            else:
                regs.append((-1, -1))
        return tuple(regs)

    def _get_index(self, group):
        if isinstance(group, int):
            if group >= 0 and group <= self.re.groups:
                return group
        else:
            if group in self.re.groupindex:
                return self.re.groupindex[group]
        raise IndexError("no such group")

    def _get_slice(self, group, default):
        group_indices = self.regs[group]
        if group_indices[0] >= 0:
            return self.string[group_indices[0]:group_indices[1]]
        else:
            return default

    def start(self, group=0):
        """Returns the indices of the start of the substring matched by group;
        group defaults to zero (meaning the whole matched substring). Returns -1
        if group exists but did not contribute to the match."""
        return self.regs[self._get_index(group)][0]

    def end(self, group=0):
        """Returns the indices of the end of the substring matched by group;
        group defaults to zero (meaning the whole matched substring). Returns -1
        if group exists but did not contribute to the match."""
        return self.regs[self._get_index(group)][1]

    def span(self, group=0):
        """Returns the 2-tuple (m.start(group), m.end(group))."""
        return self.start(group), self.end(group)

    def expand(self, template):
        """Return the string obtained by doing backslash substitution and
        resolving group references on template."""
        import sre
        return sre._expand(self.re, self, template)

    def groups(self, default=None):
        """Returns a tuple containing all the subgroups of the match. The
        default argument is used for groups that did not participate in the
        match (defaults to None)."""
        groups = []
        for indices in self.regs[1:]:
            if indices[0] >= 0:
                groups.append(self.string[indices[0]:indices[1]])
            else:
                groups.append(default)
        return tuple(groups)

    def groupdict(self, default=None):
        """Return a dictionary containing all the named subgroups of the match.
        The default argument is used for groups that did not participate in the
        match (defaults to None)."""
        groupdict = {}
        for key, value in self.re.groupindex.items():
            groupdict[key] = self._get_slice(value, default)
        return groupdict

    def group(self, *args):
        """Returns one or more subgroups of the match. Each argument is either a
        group index or a group name."""
        if len(args) == 0:
            args = (0,)
        grouplist = []
        for group in args:
            grouplist.append(self._get_slice(self._get_index(group), None))
        if len(grouplist) == 1:
            return grouplist[0]
        else:
            return tuple(grouplist)

    def __copy__():
        raise TypeError("cannot copy this pattern object")

    def __deepcopy__():
        raise TypeError("cannot copy this pattern object")


class _State(object):

    def __init__(self, string, start, end, flags):
        if isinstance(string, bytearray):
            string = str(bytes(string), "latin1")
        if isinstance(string, bytes):
            string = str(string, "latin1")
        self.string = string
        if start < 0:
            start = 0
        if end > len(string):
            end = len(string)
        self.start = start
        self.string_position = self.start
        self.end = end
        self.pos = start
        self.flags = flags
        self.reset()

    def reset(self):
        self.marks = []
        self.lastindex = -1
        self.marks_stack = []
        self.context_stack = []
        self.repeat = None

    def match(self, pattern_codes):
        # Optimization: Check string length. pattern_codes[3] contains the
        # minimum length for a string to possibly match.
        if pattern_codes[0] == sre_constants.INFO and pattern_codes[3]:
            if self.end - self.string_position < pattern_codes[3]:
                #_log("reject (got %d chars, need %d)"
                #         % (self.end - self.string_position, pattern_codes[3]))
                return False

        dispatcher = _OpcodeDispatcher()
        self.context_stack.append(_MatchContext(self, pattern_codes))
        has_matched = None
        while len(self.context_stack) > 0:
            context = self.context_stack[-1]
            has_matched = dispatcher.match(context)
            if has_matched is not None: # don't pop if context isn't done
                self.context_stack.pop()
        return has_matched

    def search(self, pattern_codes):
        flags = 0
        if pattern_codes[0] == sre_constants.INFO:
            # optimization info block
            # <INFO> <1=skip> <2=flags> <3=min> <4=max> <5=prefix info>
            if pattern_codes[2] & SRE_INFO_PREFIX and pattern_codes[5] > 1:
                return self.fast_search(pattern_codes)
            flags = pattern_codes[2]
            pattern_codes = pattern_codes[pattern_codes[1] + 1:]

        string_position = self.start
        if pattern_codes[0] == sre_constants.LITERAL:
            # Special case: Pattern starts with a literal character. This is
            # used for short prefixes
            character = pattern_codes[1]
            while True:
                while string_position < self.end \
                        and ord(self.string[string_position]) != character:
                    string_position += 1
                if string_position >= self.end:
                    return False
                self.start = string_position
                string_position += 1
                self.string_position = string_position
                if flags & SRE_INFO_LITERAL:
                    return True
                if self.match(pattern_codes[2:]):
                    return True
            return False

        # General case
        while string_position <= self.end:
            self.reset()
            self.start = self.string_position = string_position
            if self.match(pattern_codes):
                return True
            string_position += 1
        return False

    def fast_search(self, pattern_codes):
        """Skips forward in a string as fast as possible using information from
        an optimization info block."""
        # pattern starts with a known prefix
        # <5=length> <6=skip> <7=prefix data> <overlap data>
        flags = pattern_codes[2]
        prefix_len = pattern_codes[5]
        prefix_skip = pattern_codes[6] # don't really know what this is good for
        prefix = pattern_codes[7:7 + prefix_len]
        overlap = pattern_codes[7 + prefix_len - 1:pattern_codes[1] + 1]
        pattern_codes = pattern_codes[pattern_codes[1] + 1:]
        i = 0
        string_position = self.string_position
        while string_position < self.end:
            while True:
                if ord(self.string[string_position]) != prefix[i]:
                    if i == 0:
                        break
                    else:
                        i = overlap[i]
                else:
                    i += 1
                    if i == prefix_len:
                        # found a potential match
                        self.start = string_position + 1 - prefix_len
                        self.string_position = string_position + 1 \
                                                     - prefix_len + prefix_skip
                        if flags & SRE_INFO_LITERAL:
                            return True # matched all of pure literal pattern
                        if self.match(pattern_codes[2 * prefix_skip:]):
                            return True
                        i = overlap[i]
                    break
            string_position += 1
        return False

    def set_mark(self, mark_nr, position):
        if mark_nr & 1:
            # This id marks the end of a group.
            self.lastindex = mark_nr // 2 + 1
        if mark_nr >= len(self.marks):
            self.marks.extend([None] * (mark_nr - len(self.marks) + 1))
        self.marks[mark_nr] = position

    def get_marks(self, group_index):
        marks_index = 2 * group_index
        if len(self.marks) > marks_index + 1:
            return self.marks[marks_index], self.marks[marks_index + 1]
        else:
            return None, None

    def marks_push(self):
        self.marks_stack.append((self.marks[:], self.lastindex))

    def marks_pop(self):
        self.marks, self.lastindex = self.marks_stack.pop()

    def marks_pop_keep(self):
        self.marks, self.lastindex = self.marks_stack[-1]

    def marks_pop_discard(self):
        self.marks_stack.pop()

    def lower(self, char_ord):
        return getlower(char_ord, self.flags)


class _MatchContext(object):

    def __init__(self, state, pattern_codes):
        self.state = state
        self.pattern_codes = pattern_codes
        self.string_position = state.string_position
        self.code_position = 0
        self.has_matched = None

    def push_new_context(self, pattern_offset):
        """Creates a new child context of this context and pushes it on the
        stack. pattern_offset is the offset off the current code position to
        start interpreting from."""
        child_context = _MatchContext(self.state,
            self.pattern_codes[self.code_position + pattern_offset:])
        self.state.context_stack.append(child_context)
        return child_context

    def peek_char(self, peek=0):
        return self.state.string[self.string_position + peek]

    def skip_char(self, skip_count):
        self.string_position += skip_count

    def remaining_chars(self):
        return self.state.end - self.string_position

    def peek_code(self, peek=0):
        return self.pattern_codes[self.code_position + peek]

    def skip_code(self, skip_count):
        self.code_position += skip_count

    def remaining_codes(self):
        return len(self.pattern_codes) - self.code_position

    def at_beginning(self):
        return self.string_position == 0

    def at_end(self):
        return self.string_position == self.state.end

    def at_linebreak(self):
        return not self.at_end() and _is_linebreak(self.peek_char())

    def at_boundary(self, word_checker):
        if self.at_beginning() and self.at_end():
            return False
        that = not self.at_beginning() and word_checker(self.peek_char(-1))
        this = not self.at_end() and word_checker(self.peek_char())
        return this != that


class _RepeatContext(_MatchContext):

    def __init__(self, context):
        _MatchContext.__init__(self, context.state,
                            context.pattern_codes[context.code_position:])
        self.count = -1
        self.previous = context.state.repeat
        self.last_position = None


class _Dispatcher(object):

    DISPATCH_TABLE = None

    def dispatch(self, code, context):
        method = self.DISPATCH_TABLE.get(code, self.__class__.unknown)
        return method(self, context)

    def unknown(self, code, ctx):
        raise NotImplementedError()

    @classmethod
    def build_dispatch_table(cls, codes, method_prefix):
        if cls.DISPATCH_TABLE is not None:
            return
        table = {}
        for code in codes:
            code_name = code.name.lower()
            if hasattr(cls, "%s%s" % (method_prefix, code_name)):
                table[code] = getattr(cls, "%s%s" % (method_prefix, code_name))
        cls.DISPATCH_TABLE = table


class _OpcodeDispatcher(_Dispatcher):

    def __init__(self):
        self.executing_contexts = {}
        self.at_dispatcher = _AtcodeDispatcher()
        self.ch_dispatcher = _ChcodeDispatcher()
        self.set_dispatcher = _CharsetDispatcher()

    def match(self, context):
        """Returns True if the current context matches, False if it doesn't and
        None if matching is not finished, ie must be resumed after child
        contexts have been matched."""
        while context.remaining_codes() > 0 and context.has_matched is None:
            opcode = context.peek_code()
            if not self.dispatch(opcode, context):
                return None
        if context.has_matched is None:
            context.has_matched = False
        return context.has_matched

    def dispatch(self, opcode, context):
        """Dispatches a context on a given opcode. Returns True if the context
        is done matching, False if it must be resumed when next encountered."""
        if id(context) in self.executing_contexts:
            generator = self.executing_contexts[id(context)]
            del self.executing_contexts[id(context)]
            has_finished = next(generator)
        else:
            method = self.DISPATCH_TABLE.get(opcode, _OpcodeDispatcher.unknown)
            has_finished = method(self, context)
            if hasattr(has_finished, "__next__"): # avoid using the types module
                generator = has_finished
                has_finished = next(generator)
        if not has_finished:
            self.executing_contexts[id(context)] = generator
        return has_finished

    def op_success(self, ctx):
        # end of pattern
        #self._log(ctx, "SUCCESS")
        ctx.state.string_position = ctx.string_position
        ctx.has_matched = True
        return True

    def op_failure(self, ctx):
        # immediate failure
        #self._log(ctx, "FAILURE")
        ctx.has_matched = False
        return True

    def general_op_literal(self, ctx, compare, decorate=lambda x: x):
        if ctx.at_end() or not compare(decorate(ord(ctx.peek_char())),
                                            decorate(ctx.peek_code(1))):
            ctx.has_matched = False
        ctx.skip_code(2)
        ctx.skip_char(1)

    def op_literal(self, ctx):
        # match literal string
        # <LITERAL> <code>
        #self._log(ctx, "LITERAL", ctx.peek_code(1))
        self.general_op_literal(ctx, operator.eq)
        return True

    def op_not_literal(self, ctx):
        # match anything that is not the given literal character
        # <NOT_LITERAL> <code>
        #self._log(ctx, "NOT_LITERAL", ctx.peek_code(1))
        self.general_op_literal(ctx, operator.ne)
        return True

    def op_literal_ignore(self, ctx):
        # match literal regardless of case
        # <LITERAL_IGNORE> <code>
        #self._log(ctx, "LITERAL_IGNORE", ctx.peek_code(1))
        self.general_op_literal(ctx, operator.eq, ctx.state.lower)
        return True

    def op_literal_uni_ignore(self, ctx):
        self.general_op_literal(ctx, operator.eq, ctx.state.lower)
        return True

    def op_not_literal_ignore(self, ctx):
        # match literal regardless of case
        # <LITERAL_IGNORE> <code>
        #self._log(ctx, "LITERAL_IGNORE", ctx.peek_code(1))
        self.general_op_literal(ctx, operator.ne, ctx.state.lower)
        return True

    def op_at(self, ctx):
        # match at given position
        # <AT> <code>
        #self._log(ctx, "AT", ctx.peek_code(1))
        if not self.at_dispatcher.dispatch(ctx.peek_code(1), ctx):
            ctx.has_matched = False
            return True
        ctx.skip_code(2)
        return True

    def op_category(self, ctx):
        # match at given category
        # <CATEGORY> <code>
        #self._log(ctx, "CATEGORY", ctx.peek_code(1))
        if ctx.at_end() or not self.ch_dispatcher.dispatch(ctx.peek_code(1), ctx):
            ctx.has_matched = False
            return True
        ctx.skip_code(2)
        ctx.skip_char(1)
        return True

    def op_any(self, ctx):
        # match anything (except a newline)
        # <ANY>
        #self._log(ctx, "ANY")
        if ctx.at_end() or ctx.at_linebreak():
            ctx.has_matched = False
            return True
        ctx.skip_code(1)
        ctx.skip_char(1)
        return True

    def op_any_all(self, ctx):
        # match anything
        # <ANY_ALL>
        #self._log(ctx, "ANY_ALL")
        if ctx.at_end():
            ctx.has_matched = False
            return True
        ctx.skip_code(1)
        ctx.skip_char(1)
        return True

    def general_op_in(self, ctx, decorate=lambda x: x):
        #self._log(ctx, "OP_IN")
        if ctx.at_end():
            ctx.has_matched = False
            return
        skip = ctx.peek_code(1)
        ctx.skip_code(2) # set op pointer to the set code
        if not self.check_charset(ctx, decorate(ord(ctx.peek_char()))):
            ctx.has_matched = False
            return
        ctx.skip_code(skip - 1)
        ctx.skip_char(1)

    def op_in(self, ctx):
        # match set member (or non_member)
        # <IN> <skip> <set>
        #self._log(ctx, "OP_IN")
        self.general_op_in(ctx)
        return True

    def op_in_ignore(self, ctx):
        # match set member (or non_member), disregarding case of current char
        # <IN_IGNORE> <skip> <set>
        #self._log(ctx, "OP_IN_IGNORE")
        self.general_op_in(ctx, ctx.state.lower)
        return True

    def op_in_uni_ignore(self, ctx):
        self.general_op_in(ctx, ctx.state.lower)
        return True

    def op_jump(self, ctx):
        # jump forward
        # <JUMP> <offset>
        #self._log(ctx, "JUMP", ctx.peek_code(1))
        ctx.skip_code(ctx.peek_code(1) + 1)
        return True

    # skip info
    # <INFO> <skip>
    op_info = op_jump

    def op_mark(self, ctx):
        # set mark
        # <MARK> <gid>
        #self._log(ctx, "OP_MARK", ctx.peek_code(1))
        ctx.state.set_mark(ctx.peek_code(1), ctx.string_position)
        ctx.skip_code(2)
        return True

    def op_branch(self, ctx):
        # alternation
        # <BRANCH> <0=skip> code <JUMP> ... <NULL>
        #self._log(ctx, "BRANCH")
        ctx.state.marks_push()
        ctx.skip_code(1)
        current_branch_length = ctx.peek_code(0)
        while current_branch_length:
            # The following tries to shortcut branches starting with a
            # (unmatched) literal. _sre.c also shortcuts charsets here.
            if not (ctx.peek_code(1) == sre_constants.LITERAL and \
                    (ctx.at_end() or ctx.peek_code(2) != ord(ctx.peek_char()))):
                ctx.state.string_position = ctx.string_position
                child_context = ctx.push_new_context(1)
                yield False
                if child_context.has_matched:
                    ctx.has_matched = True
                    yield True
                ctx.state.marks_pop_keep()
            ctx.skip_code(current_branch_length)
            current_branch_length = ctx.peek_code(0)
        ctx.state.marks_pop_discard()
        ctx.has_matched = False
        yield True

    def op_repeat_one(self, ctx):
        # match repeated sequence (maximizing).
        # this operator only works if the repeated item is exactly one character
        # wide, and we're not already collecting backtracking points.
        # <REPEAT_ONE> <skip> <1=min> <2=max> item <SUCCESS> tail
        mincount = ctx.peek_code(2)
        maxcount = ctx.peek_code(3)
        #self._log(ctx, "REPEAT_ONE", mincount, maxcount)

        if ctx.remaining_chars() < mincount:
            ctx.has_matched = False
            yield True
        ctx.state.string_position = ctx.string_position
        count = self.count_repetitions(ctx, maxcount)
        ctx.skip_char(count)
        if count < mincount:
            ctx.has_matched = False
            yield True
        if ctx.peek_code(ctx.peek_code(1) + 1) == sre_constants.SUCCESS:
            # tail is empty.  we're finished
            ctx.state.string_position = ctx.string_position
            ctx.has_matched = True
            yield True

        ctx.state.marks_push()
        if ctx.peek_code(ctx.peek_code(1) + 1) == sre_constants.LITERAL:
            # Special case: Tail starts with a literal. Skip positions where
            # the rest of the pattern cannot possibly match.
            char = ctx.peek_code(ctx.peek_code(1) + 2)
            while True:
                while count >= mincount and \
                                (ctx.at_end() or ord(ctx.peek_char()) != char):
                    ctx.skip_char(-1)
                    count -= 1
                if count < mincount:
                    break
                ctx.state.string_position = ctx.string_position
                child_context = ctx.push_new_context(ctx.peek_code(1) + 1)
                yield False
                if child_context.has_matched:
                    ctx.has_matched = True
                    yield True
                ctx.skip_char(-1)
                count -= 1
                ctx.state.marks_pop_keep()

        else:
            # General case: backtracking
            while count >= mincount:
                ctx.state.string_position = ctx.string_position
                child_context = ctx.push_new_context(ctx.peek_code(1) + 1)
                yield False
                if child_context.has_matched:
                    ctx.has_matched = True
                    yield True
                ctx.skip_char(-1)
                count -= 1
                ctx.state.marks_pop_keep()

        ctx.state.marks_pop_discard()
        ctx.has_matched = False
        yield True

    def op_min_repeat_one(self, ctx):
        # match repeated sequence (minimizing)
        # <MIN_REPEAT_ONE> <skip> <1=min> <2=max> item <SUCCESS> tail
        mincount = ctx.peek_code(2)
        maxcount = ctx.peek_code(3)
        #self._log(ctx, "MIN_REPEAT_ONE", mincount, maxcount)

        if ctx.remaining_chars() < mincount:
            ctx.has_matched = False
            yield True
        ctx.state.string_position = ctx.string_position
        if mincount == 0:
            count = 0
        else:
            count = self.count_repetitions(ctx, mincount)
            if count < mincount:
                ctx.has_matched = False
                yield True
            ctx.skip_char(count)
        if ctx.peek_code(ctx.peek_code(1) + 1) == sre_constants.SUCCESS:
            # tail is empty.  we're finished
            ctx.state.string_position = ctx.string_position
            ctx.has_matched = True
            yield True

        ctx.state.marks_push()
        while maxcount == MAXREPEAT or count <= maxcount:
            ctx.state.string_position = ctx.string_position
            child_context = ctx.push_new_context(ctx.peek_code(1) + 1)
            yield False
            if child_context.has_matched:
                ctx.has_matched = True
                yield True
            ctx.state.string_position = ctx.string_position
            if self.count_repetitions(ctx, 1) == 0:
                break
            ctx.skip_char(1)
            count += 1
            ctx.state.marks_pop_keep()

        ctx.state.marks_pop_discard()
        ctx.has_matched = False
        yield True

    def op_repeat(self, ctx):
        # create repeat context.  all the hard work is done by the UNTIL
        # operator (MAX_UNTIL, MIN_UNTIL)
        # <REPEAT> <skip> <1=min> <2=max> item <UNTIL> tail
        #self._log(ctx, "REPEAT", ctx.peek_code(2), ctx.peek_code(3))
        repeat = _RepeatContext(ctx)
        ctx.state.repeat = repeat
        ctx.state.string_position = ctx.string_position
        child_context = ctx.push_new_context(ctx.peek_code(1) + 1)
        yield False
        ctx.state.repeat = repeat.previous
        ctx.has_matched = child_context.has_matched
        yield True

    def op_max_until(self, ctx):
        # maximizing repeat
        # <REPEAT> <skip> <1=min> <2=max> item <MAX_UNTIL> tail
        repeat = ctx.state.repeat
        if repeat is None:
            raise RuntimeError("Internal re error: MAX_UNTIL without REPEAT.")
        mincount = repeat.peek_code(2)
        maxcount = repeat.peek_code(3)
        ctx.state.string_position = ctx.string_position
        count = repeat.count + 1
        #self._log(ctx, "MAX_UNTIL", count)

        if count < mincount:
            # not enough matches
            repeat.count = count
            child_context = repeat.push_new_context(4)
            yield False
            ctx.has_matched = child_context.has_matched
            if not ctx.has_matched:
                repeat.count = count - 1
                ctx.state.string_position = ctx.string_position
            yield True

        if (count < maxcount or maxcount == MAXREPEAT) \
                      and ctx.state.string_position != repeat.last_position:
            # we may have enough matches, if we can match another item, do so
            repeat.count = count
            ctx.state.marks_push()
            save_last_position = repeat.last_position # zero-width match protection
            repeat.last_position = ctx.state.string_position
            child_context = repeat.push_new_context(4)
            yield False
            repeat.last_position = save_last_position
            if child_context.has_matched:
                ctx.state.marks_pop_discard()
                ctx.has_matched = True
                yield True
            ctx.state.marks_pop()
            repeat.count = count - 1
            ctx.state.string_position = ctx.string_position

        # cannot match more repeated items here.  make sure the tail matches
        ctx.state.repeat = repeat.previous
        child_context = ctx.push_new_context(1)
        yield False
        ctx.has_matched = child_context.has_matched
        if not ctx.has_matched:
            ctx.state.repeat = repeat
            ctx.state.string_position = ctx.string_position
        yield True

    def op_min_until(self, ctx):
        # minimizing repeat
        # <REPEAT> <skip> <1=min> <2=max> item <MIN_UNTIL> tail
        repeat = ctx.state.repeat
        if repeat is None:
            raise RuntimeError("Internal re error: MIN_UNTIL without REPEAT.")
        mincount = repeat.peek_code(2)
        maxcount = repeat.peek_code(3)
        ctx.state.string_position = ctx.string_position
        count = repeat.count + 1
        #self._log(ctx, "MIN_UNTIL", count)

        if count < mincount:
            # not enough matches
            repeat.count = count
            child_context = repeat.push_new_context(4)
            yield False
            ctx.has_matched = child_context.has_matched
            if not ctx.has_matched:
                repeat.count = count - 1
                ctx.state.string_position = ctx.string_position
            yield True

        # see if the tail matches
        ctx.state.marks_push()
        ctx.state.repeat = repeat.previous
        child_context = ctx.push_new_context(1)
        yield False
        if child_context.has_matched:
            ctx.has_matched = True
            yield True
        ctx.state.repeat = repeat
        ctx.state.string_position = ctx.string_position
        ctx.state.marks_pop()

        # match more until tail matches
        if count >= maxcount and maxcount != MAXREPEAT:
            ctx.has_matched = False
            yield True
        repeat.count = count
        child_context = repeat.push_new_context(4)
        yield False
        ctx.has_matched = child_context.has_matched
        if not ctx.has_matched:
            repeat.count = count - 1
            ctx.state.string_position = ctx.string_position
        yield True

    def general_op_groupref(self, ctx, decorate=lambda x: x):
        group_start, group_end = ctx.state.get_marks(ctx.peek_code(1))
        if group_start is None or group_end is None or group_end < group_start:
            ctx.has_matched = False
            return True
        while group_start < group_end:
            if ctx.at_end() or decorate(ord(ctx.peek_char())) \
                                != decorate(ord(ctx.state.string[group_start])):
                ctx.has_matched = False
                return True
            group_start += 1
            ctx.skip_char(1)
        ctx.skip_code(2)
        return True

    def op_groupref(self, ctx):
        # match backreference
        # <GROUPREF> <zero-based group index>
        #self._log(ctx, "GROUPREF", ctx.peek_code(1))
        return self.general_op_groupref(ctx)

    def op_groupref_ignore(self, ctx):
        # match backreference case-insensitive
        # <GROUPREF_IGNORE> <zero-based group index>
        #self._log(ctx, "GROUPREF_IGNORE", ctx.peek_code(1))
        return self.general_op_groupref(ctx, ctx.state.lower)

    def op_groupref_exists(self, ctx):
        # <GROUPREF_EXISTS> <group> <skip> codeyes <JUMP> codeno ...
        #self._log(ctx, "GROUPREF_EXISTS", ctx.peek_code(1))
        group_start, group_end = ctx.state.get_marks(ctx.peek_code(1))
        if group_start is None or group_end is None or group_end < group_start:
            ctx.skip_code(ctx.peek_code(2) + 1)
        else:
            ctx.skip_code(3)
        return True

    def op_assert(self, ctx):
        # assert subpattern
        # <ASSERT> <skip> <back> <pattern>
        #self._log(ctx, "ASSERT", ctx.peek_code(2))
        ctx.state.string_position = ctx.string_position - ctx.peek_code(2)
        if ctx.state.string_position < 0:
            ctx.has_matched = False
            yield True
        child_context = ctx.push_new_context(3)
        yield False
        if child_context.has_matched:
            ctx.skip_code(ctx.peek_code(1) + 1)
        else:
            ctx.has_matched = False
        yield True

    def op_assert_not(self, ctx):
        # assert not subpattern
        # <ASSERT_NOT> <skip> <back> <pattern>
        #self._log(ctx, "ASSERT_NOT", ctx.peek_code(2))
        ctx.state.string_position = ctx.string_position - ctx.peek_code(2)
        if ctx.state.string_position >= 0:
            child_context = ctx.push_new_context(3)
            yield False
            if child_context.has_matched:
                ctx.has_matched = False
                yield True
        ctx.skip_code(ctx.peek_code(1) + 1)
        yield True

    def unknown(self, ctx):
        #self._log(ctx, "UNKNOWN", ctx.peek_code())
        raise RuntimeError("Internal re error. Unknown opcode: %s" % ctx.peek_code())

    def check_charset(self, ctx, char):
        """Checks whether a character matches set of arbitrary length. Assumes
        the code pointer is at the first member of the set."""
        self.set_dispatcher.reset(char)
        save_position = ctx.code_position
        result = None
        while result is None:
            result = self.set_dispatcher.dispatch(ctx.peek_code(), ctx)
        ctx.code_position = save_position
        return result

    def count_repetitions(self, ctx, maxcount):
        """Returns the number of repetitions of a single item, starting from the
        current string position. The code pointer is expected to point to a
        REPEAT_ONE operation (with the repeated 4 ahead)."""
        count = 0
        real_maxcount = ctx.state.end - ctx.string_position
        if maxcount < real_maxcount and maxcount != MAXREPEAT:
            real_maxcount = maxcount
        # XXX could special case every single character pattern here, as in C.
        # This is a general solution, a bit hackisch, but works and should be
        # efficient.
        code_position = ctx.code_position
        string_position = ctx.string_position
        ctx.skip_code(4)
        reset_position = ctx.code_position
        while count < real_maxcount:
            # this works because the single character pattern is followed by
            # a success opcode
            ctx.code_position = reset_position
            self.dispatch(ctx.peek_code(), ctx)
            if ctx.has_matched is False: # could be None as well
                break
            count += 1
        ctx.has_matched = None
        ctx.code_position = code_position
        ctx.string_position = string_position
        return count

    def _log(self, context, opname, *args):
        arg_string = ("%s " * len(args)) % args
        _log("|%s|%s|%s %s" % (context.pattern_codes,
            context.string_position, opname, arg_string))

_OpcodeDispatcher.build_dispatch_table(OPCODES, "op_")


class _CharsetDispatcher(_Dispatcher):

    def __init__(self):
        self.ch_dispatcher = _ChcodeDispatcher()

    def reset(self, char):
        self.char = char
        self.ok = True

    def set_failure(self, ctx):
        return not self.ok
    def set_literal(self, ctx):
        # <LITERAL> <code>
        if ctx.peek_code(1) == self.char:
            return self.ok
        else:
            ctx.skip_code(2)
    def set_category(self, ctx):
        # <CATEGORY> <code>
        if self.ch_dispatcher.dispatch(ctx.peek_code(1), ctx):
            return self.ok
        else:
            ctx.skip_code(2)
    def set_charset(self, ctx):
        # <CHARSET> <bitmap> (16 bits per code word)
        char_code = self.char
        ctx.skip_code(1) # point to beginning of bitmap
        if CODESIZE == 2:
            if char_code < 256 and ctx.peek_code(char_code >> 4) \
                                            & (1 << (char_code & 15)):
                return self.ok
            ctx.skip_code(16) # skip bitmap
        else:
            if char_code < 256 and ctx.peek_code(char_code >> 5) \
                                            & (1 << (char_code & 31)):
                return self.ok
            ctx.skip_code(8) # skip bitmap
    def set_range(self, ctx):
        # <RANGE> <lower> <upper>
        if ctx.peek_code(1) <= self.char <= ctx.peek_code(2):
            return self.ok
        ctx.skip_code(3)
    def set_negate(self, ctx):
        self.ok = not self.ok
        ctx.skip_code(1)
    def set_bigcharset(self, ctx):
        # <BIGCHARSET> <blockcount> <256 blockindices> <blocks>
        char_code = self.char
        count = ctx.peek_code(1)
        ctx.skip_code(2)
        if char_code < 65536:
            block_index = char_code >> 8
            # NB: there are CODESIZE block indices per bytecode
            a = array.array("B")
            a.frombytes(array.array(CODESIZE == 2 and "H" or "I",
                    [ctx.peek_code(block_index // CODESIZE)]).tobytes())
            block = a[block_index % CODESIZE]
            ctx.skip_code(256 // CODESIZE) # skip block indices
            block_value = ctx.peek_code(block * (32 // CODESIZE)
                    + ((char_code & 255) >> (CODESIZE == 2 and 4 or 5)))
            if block_value & (1 << (char_code & ((8 * CODESIZE) - 1))):
                return self.ok
        else:
            ctx.skip_code(256 // CODESIZE) # skip block indices
        ctx.skip_code(count * (32 // CODESIZE)) # skip blocks
    def unknown(self, ctx):
        return False

_CharsetDispatcher.build_dispatch_table(OPCODES, "set_")


class _AtcodeDispatcher(_Dispatcher):

    def at_beginning(self, ctx):
        return ctx.at_beginning()
    at_beginning_string = at_beginning
    def at_beginning_line(self, ctx):
        return ctx.at_beginning() or _is_linebreak(ctx.peek_char(-1))
    def at_end(self, ctx):
        return (ctx.remaining_chars() == 1 and ctx.at_linebreak()) or ctx.at_end()
    def at_end_line(self, ctx):
        return ctx.at_linebreak() or ctx.at_end()
    def at_end_string(self, ctx):
        return ctx.at_end()
    def at_boundary(self, ctx):
        return ctx.at_boundary(_is_word)
    def at_non_boundary(self, ctx):
        return not ctx.at_boundary(_is_word)
    def at_loc_boundary(self, ctx):
        return ctx.at_boundary(_is_loc_word)
    def at_loc_non_boundary(self, ctx):
        return not ctx.at_boundary(_is_loc_word)
    def at_uni_boundary(self, ctx):
        return ctx.at_boundary(_is_uni_word)
    def at_uni_non_boundary(self, ctx):
        return not ctx.at_boundary(_is_uni_word)
    def unknown(self, ctx):
        return False

_AtcodeDispatcher.build_dispatch_table(ATCODES, "")


class _ChcodeDispatcher(_Dispatcher):

    def category_digit(self, ctx):
        return _is_digit(ctx.peek_char())
    def category_not_digit(self, ctx):
        return not _is_digit(ctx.peek_char())
    def category_space(self, ctx):
        return _is_space(ctx.peek_char())
    def category_not_space(self, ctx):
        return not _is_space(ctx.peek_char())
    def category_word(self, ctx):
        return _is_word(ctx.peek_char())
    def category_not_word(self, ctx):
        return not _is_word(ctx.peek_char())
    def category_linebreak(self, ctx):
        return _is_linebreak(ctx.peek_char())
    def category_not_linebreak(self, ctx):
        return not _is_linebreak(ctx.peek_char())
    def category_loc_word(self, ctx):
        return _is_loc_word(ctx.peek_char())
    def category_loc_not_word(self, ctx):
        return not _is_loc_word(ctx.peek_char())
    def category_uni_digit(self, ctx):
        return ctx.peek_char().isdigit()
    def category_uni_not_digit(self, ctx):
        return not ctx.peek_char().isdigit()
    def category_uni_space(self, ctx):
        return ctx.peek_char().isspace()
    def category_uni_not_space(self, ctx):
        return not ctx.peek_char().isspace()
    def category_uni_word(self, ctx):
        return _is_uni_word(ctx.peek_char())
    def category_uni_not_word(self, ctx):
        return not _is_uni_word(ctx.peek_char())
    def category_uni_linebreak(self, ctx):
        return ord(ctx.peek_char()) in _uni_linebreaks
    def category_uni_not_linebreak(self, ctx):
        return ord(ctx.peek_char()) not in _uni_linebreaks
    def unknown(self, ctx):
        return False

_ChcodeDispatcher.build_dispatch_table(CHCODES, "")


_ascii_char_info = [ 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 6, 2,
2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0,
0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 25, 25, 25, 25, 25, 25, 25, 25,
25, 25, 0, 0, 0, 0, 0, 0, 0, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24,
24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 0, 0,
0, 0, 16, 0, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24,
24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 0, 0, 0, 0, 0 ]

def _is_digit(char):
    code = ord(char)
    return code < 128 and _ascii_char_info[code] & 1

def _is_space(char):
    code = ord(char)
    return code < 128 and _ascii_char_info[code] & 2

def _is_word(char):
    # NB: non-ASCII chars aren't words according to _sre.c
    code = ord(char)
    return code < 128 and _ascii_char_info[code] & 16

def _is_loc_word(char):
    return (not (ord(char) & ~255) and char.isalnum()) or char == '_'

def _is_uni_word(char):
    return chr(ord(char)).isalnum() or char == '_'

def _is_linebreak(char):
    return char == "\n"

# Static list of all unicode codepoints reported by Py_UNICODE_ISLINEBREAK.
_uni_linebreaks = [10, 13, 28, 29, 30, 133, 8232, 8233]

def _log(message):
    if 0:
        print(message)
