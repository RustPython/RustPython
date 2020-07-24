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
