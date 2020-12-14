pub fn get_chars(s: &str, range: std::ops::Range<usize>) -> &str {
    let mut chars = s.chars();
    for _ in 0..range.start {
        let _ = chars.next();
    }
    let start = chars.as_str();
    for _ in range {
        let _ = chars.next();
    }
    let end = chars.as_str();
    &start[..start.len() - end.len()]
}

pub fn zfill(bytes: &[u8], width: usize) -> Vec<u8> {
    if width <= bytes.len() {
        bytes.to_vec()
    } else {
        let (sign, s) = match bytes.first() {
            Some(_sign @ b'+') | Some(_sign @ b'-') => {
                (unsafe { bytes.get_unchecked(..1) }, &bytes[1..])
            }
            _ => (&b""[..], bytes),
        };
        let mut filled = Vec::new();
        filled.extend_from_slice(sign);
        filled.extend(std::iter::repeat(b'0').take(width - bytes.len()));
        filled.extend_from_slice(s);
        filled
    }
}

/// Convert a string to ascii compatible, escaping unicodes into escape
/// sequences.
pub fn to_ascii(value: &str) -> String {
    let mut ascii = String::new();
    for c in value.chars() {
        if c.is_ascii() {
            ascii.push(c)
        } else {
            let c = c as i64;
            let hex = if c < 0x100 {
                format!("\\x{:02x}", c)
            } else if c < 0x10000 {
                format!("\\u{:04x}", c)
            } else {
                format!("\\U{:08x}", c)
            };
            ascii.push_str(&hex)
        }
    }
    ascii
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_chars() {
        let s = "0123456789";
        assert_eq!(get_chars(s, 3..7), "3456");
        assert_eq!(get_chars(s, 3..7), &s[3..7]);

        let s = "0유니코드 문자열9";
        assert_eq!(get_chars(s, 3..7), "코드 문");

        let s = "0😀😃😄😁😆😅😂🤣9";
        assert_eq!(get_chars(s, 3..7), "😄😁😆😅");
    }
}
