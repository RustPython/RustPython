use alloc::{borrow::Cow, string::String};

/// Translate physical source line endings before tokenization, including those
/// inside literals, as CPython's `_PyTokenizer_translate_newlines` does.
pub fn normalize_newlines(source: &str) -> Cow<'_, str> {
    let Some(first_cr) = source.find('\r') else {
        return Cow::Borrowed(source);
    };
    let mut normalized = String::with_capacity(source.len());
    normalized.push_str(&source[..first_cr]);
    let mut rest = &source[first_cr..];
    while let Some(cr) = rest.find('\r') {
        normalized.push_str(&rest[..cr]);
        normalized.push('\n');
        rest = &rest[cr + 1..];
        if let Some(after_lf) = rest.strip_prefix('\n') {
            rest = after_lf;
        }
    }
    normalized.push_str(rest);
    Cow::Owned(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrow_lf_source() {
        let source = "print('한글')\n";
        assert!(matches!(normalize_newlines(source), Cow::Borrowed(s) if s == source));
    }

    #[test]
    fn physical_line_endings() {
        assert_eq!(
            normalize_newlines("a\r\nb\rc\n\rd\r\r\n"),
            "a\nb\nc\n\nd\n\n"
        );
        assert_eq!(normalize_newlines("'\\r'\r"), "'\\r'\n");
    }
}
