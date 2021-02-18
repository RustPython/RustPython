# taken from https://bitbucket.org/pypa/distlib/src/master/distlib/util.py
# flake8: noqa
# fmt: off

from types import SimpleNamespace as Container
import re

IDENTIFIER = re.compile(r'^([\w\.-]+)\s*')
VERSION_IDENTIFIER = re.compile(r'^([\w\.*+-]+)\s*')
COMPARE_OP = re.compile(r'^(<=?|>=?|={2,3}|[~!]=)\s*')
NON_SPACE = re.compile(r'(\S+)\s*')

def parse_requirement(req):
    """
    Parse a requirement passed in as a string. Return a Container
    whose attributes contain the various parts of the requirement.
    """
    remaining = req.strip()
    if not remaining or remaining.startswith('#'):
        return None
    m = IDENTIFIER.match(remaining)
    if not m:
        raise SyntaxError('name expected: %s' % remaining)
    distname = m.groups()[0]
    remaining = remaining[m.end():]
    extras = mark_expr = versions = uri = None
    if remaining and remaining[0] == '[':
        i = remaining.find(']', 1)
        if i < 0:
            raise SyntaxError('unterminated extra: %s' % remaining)
        s = remaining[1:i]
        remaining = remaining[i + 1:].lstrip()
        extras = []
        while s:
            m = IDENTIFIER.match(s)
            if not m:
                raise SyntaxError('malformed extra: %s' % s)
            extras.append(m.groups()[0])
            s = s[m.end():]
            if not s:
                break
            if s[0] != ',':
                raise SyntaxError('comma expected in extras: %s' % s)
            s = s[1:].lstrip()
        if not extras:
            extras = None
    if remaining:
        if remaining[0] == '@':
            # it's a URI
            remaining = remaining[1:].lstrip()
            m = NON_SPACE.match(remaining)
            if not m:
                raise SyntaxError('invalid URI: %s' % remaining)
            uri = m.groups()[0]
            t = urlparse(uri)
            # there are issues with Python and URL parsing, so this test
            # is a bit crude. See bpo-20271, bpo-23505. Python doesn't
            # always parse invalid URLs correctly - it should raise
            # exceptions for malformed URLs
            if not (t.scheme and t.netloc):
                raise SyntaxError('Invalid URL: %s' % uri)
            remaining = remaining[m.end():].lstrip()
        else:

            def get_versions(ver_remaining):
                """
                Return a list of operator, version tuples if any are
                specified, else None.
                """
                m = COMPARE_OP.match(ver_remaining)
                versions = None
                if m:
                    versions = []
                    while True:
                        op = m.groups()[0]
                        ver_remaining = ver_remaining[m.end():]
                        m = VERSION_IDENTIFIER.match(ver_remaining)
                        if not m:
                            raise SyntaxError('invalid version: %s' % ver_remaining)
                        v = m.groups()[0]
                        versions.append((op, v))
                        ver_remaining = ver_remaining[m.end():]
                        if not ver_remaining or ver_remaining[0] != ',':
                            break
                        ver_remaining = ver_remaining[1:].lstrip()
                        m = COMPARE_OP.match(ver_remaining)
                        if not m:
                            raise SyntaxError('invalid constraint: %s' % ver_remaining)
                    if not versions:
                        versions = None
                return versions, ver_remaining

            if remaining[0] != '(':
                versions, remaining = get_versions(remaining)
            else:
                i = remaining.find(')', 1)
                if i < 0:
                    raise SyntaxError('unterminated parenthesis: %s' % remaining)
                s = remaining[1:i]
                remaining = remaining[i + 1:].lstrip()
                # As a special diversion from PEP 508, allow a version number
                # a.b.c in parentheses as a synonym for ~= a.b.c (because this
                # is allowed in earlier PEPs)
                if COMPARE_OP.match(s):
                    versions, _ = get_versions(s)
                else:
                    m = VERSION_IDENTIFIER.match(s)
                    if not m:
                        raise SyntaxError('invalid constraint: %s' % s)
                    v = m.groups()[0]
                    s = s[m.end():].lstrip()
                    if s:
                        raise SyntaxError('invalid constraint: %s' % s)
                    versions = [('~=', v)]

    if remaining:
        if remaining[0] != ';':
            raise SyntaxError('invalid requirement: %s' % remaining)
        remaining = remaining[1:].lstrip()

        mark_expr, remaining = parse_marker(remaining)

    if remaining and remaining[0] != '#':
        raise SyntaxError('unexpected trailing data: %s' % remaining)

    if not versions:
        rs = distname
    else:
        rs = '%s %s' % (distname, ', '.join(['%s %s' % con for con in versions]))
    return Container(name=distname, extras=extras, constraints=versions,
                     marker=mark_expr, url=uri, requirement=rs)
