pub fn repr(b: &[u8]) -> String {
    repr_with(b, &[], "")
}

pub fn repr_with(b: &[u8], prefixes: &[&str], suffix: &str) -> String {
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
        // TODO: OverflowError
        out_len = out_len.checked_add(incr).unwrap();
    }

    let (quote, num_escaped_quotes) = crate::str::choose_quotes_for_repr(squote, dquote);
    // we'll be adding backslashes in front of the existing inner quotes
    out_len += num_escaped_quotes;

    // 3 is for b prefix + outer quotes
    out_len += 3 + prefixes.iter().map(|s| s.len()).sum::<usize>() + suffix.len();

    let mut res = String::with_capacity(out_len);
    res.extend(prefixes.iter().copied());
    res.push('b');
    res.push(quote);
    for &ch in b {
        match ch {
            b'\t' => res.push_str("\\t"),
            b'\n' => res.push_str("\\n"),
            b'\r' => res.push_str("\\r"),
            // printable ascii range
            0x20..=0x7e => {
                let ch = ch as char;
                if ch == quote || ch == '\\' {
                    res.push('\\');
                }
                res.push(ch);
            }
            _ => write!(res, "\\x{:02x}", ch).unwrap(),
        }
    }
    res.push(quote);
    res.push_str(suffix);

    res
}
