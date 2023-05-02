use crate::escape::Quote;
use crate::str::ReprOverflowError;

pub fn repr(b: &[u8]) -> Result<String, ReprOverflowError> {
    repr_with(b, &[], "", Quote::Single)
}

pub fn repr_with_quote(b: &[u8], quote: Quote) -> Result<String, ReprOverflowError> {
    repr_with(b, &[], "", quote)
}

pub fn repr_with(
    b: &[u8],
    prefixes: &[&str],
    suffix: &str,
    quote: Quote,
) -> Result<String, ReprOverflowError> {
    use std::fmt::Write;

    let mut out_len = 0usize;
    let mut squote = 0;
    let mut dquote = 0;

    for &ch in b {
        let incr = match ch {
            b'\'' => {
                squote += 1;
                1
            }
            b'"' => {
                dquote += 1;
                1
            }
            b'\\' | b'\t' | b'\r' | b'\n' => 2,
            0x20..=0x7e => 1,
            _ => 4, // \xHH
        };
        out_len = out_len.checked_add(incr).ok_or(ReprOverflowError)?;
    }

    let (quote, num_escaped_quotes) = crate::escape::choose_quote(squote, dquote, quote);
    // we'll be adding backslashes in front of the existing inner quotes
    out_len += num_escaped_quotes;

    // 3 is for b prefix + outer quotes
    out_len += 3 + prefixes.iter().map(|s| s.len()).sum::<usize>() + suffix.len();

    let mut res = String::with_capacity(out_len);
    res.extend(prefixes.iter().copied());
    res.push('b');
    res.push(quote.to_char());
    for &ch in b {
        match ch {
            b'\t' => res.push_str("\\t"),
            b'\n' => res.push_str("\\n"),
            b'\r' => res.push_str("\\r"),
            // printable ascii range
            0x20..=0x7e => {
                let ch = ch as char;
                if ch == quote.to_char() || ch == '\\' {
                    res.push('\\');
                }
                res.push(ch);
            }
            _ => write!(res, "\\x{ch:02x}").unwrap(),
        }
    }
    res.push(quote.to_char());
    res.push_str(suffix);

    Ok(res)
}
