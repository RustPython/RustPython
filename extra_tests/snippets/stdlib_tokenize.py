import io
import tokenize

# Pending f-string parts must use the correct source line after CRLF lines.
source = 'pass\r\npass\r\nf"a}}b"\r\n'
tokens = list(tokenize.generate_tokens(io.StringIO(source).readline))
parts = [tok for tok in tokens if tok.type == tokenize.FSTRING_MIDDLE]
assert parts
assert all(tok.line == 'f"a}}b"\r\n' for tok in parts)
