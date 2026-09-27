#![no_std]

include!("./data.inc.rs");

const fn cmp_bytes(left: &[u8], right: &[u8]) -> i8 {
    let n = if left.len() < right.len() {
        left.len()
    } else {
        right.len()
    };
    let mut i = 0;
    while i < n {
        if left[i] != right[i] {
            return if left[i] < right[i] { -1 } else { 1 };
        }
        i += 1;
    }
    if left.len() == right.len() {
        0
    } else if left.len() < right.len() {
        -1
    } else {
        1
    }
}

const fn composed_len(
    prefix: &[u8],
    module: &[u8],
    class: &[u8],
    attr: &[u8],
    has_attr: bool,
) -> usize {
    let mut n = prefix.len() + module.len() + 1 + class.len();
    if has_attr {
        n += 1 + attr.len();
    }
    n
}

const fn composed_byte(
    index: usize,
    prefix: &[u8],
    module: &[u8],
    class: &[u8],
    attr: &[u8],
) -> u8 {
    let mut index = index;
    if index < prefix.len() {
        return prefix[index];
    }
    index -= prefix.len();
    if index < module.len() {
        return module[index];
    }
    index -= module.len();
    if index == 0 {
        return b'.';
    }
    index -= 1;
    if index < class.len() {
        return class[index];
    }
    index -= class.len();
    if index == 0 {
        return b'.';
    }
    attr[index - 1]
}

const fn cmp_composed(
    key: &[u8],
    prefix: &[u8],
    module: &[u8],
    class: &[u8],
    attr: &[u8],
    has_attr: bool,
) -> i8 {
    let query_len = composed_len(prefix, module, class, attr, has_attr);
    let n = if key.len() < query_len {
        key.len()
    } else {
        query_len
    };
    let mut i = 0;
    while i < n {
        let query = composed_byte(i, prefix, module, class, attr);
        if key[i] != query {
            return if key[i] < query { -1 } else { 1 };
        }
        i += 1;
    }
    if key.len() == query_len {
        0
    } else if key.len() < query_len {
        -1
    } else {
        1
    }
}

const fn find_composed(
    prefix: &[u8],
    module: &[u8],
    class: &[u8],
    attr: &[u8],
    has_attr: bool,
) -> Option<&'static str> {
    let mut lo = 0;
    let mut hi = DB.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let ord = cmp_composed(DB[mid].0.as_bytes(), prefix, module, class, attr, has_attr);
        if ord == 0 {
            return Some(DB[mid].1);
        } else if ord < 0 {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    None
}

const fn eq(left: &str, right: &str) -> bool {
    cmp_bytes(left.as_bytes(), right.as_bytes()) == 0
}

const fn starts_with_underscore(module: &str) -> bool {
    let bytes = module.as_bytes();
    !bytes.is_empty() && bytes[0] == b'_'
}

const fn strip_underscore(module: &str) -> Option<&str> {
    let bytes = module.as_bytes();
    if bytes.len() <= 1 || bytes[0] != b'_' {
        return None;
    }
    match core::str::from_utf8(bytes.split_at(1).1) {
        Ok(rest) => Some(rest),
        Err(_) => None,
    }
}

const fn nonempty(doc: Option<&str>) -> Option<&str> {
    match doc {
        Some(doc) if !doc.is_empty() => Some(doc),
        _ => None,
    }
}

const fn find_named(
    prefix: &[u8],
    module: &str,
    class: &str,
    attr: Option<&str>,
) -> Option<&'static str> {
    match attr {
        Some(attr) => find_composed(
            prefix,
            module.as_bytes(),
            class.as_bytes(),
            attr.as_bytes(),
            true,
        ),
        None => find_composed(prefix, module.as_bytes(), class.as_bytes(), b"", false),
    }
}

/// Docstring for an exact `module.class.attr` or `module.name` key.
/// `attr == None` looks up `module.class`.
#[must_use]
pub const fn get(key: &str) -> Option<&'static str> {
    let key = key.as_bytes();
    let mut lo = 0;
    let mut hi = DB.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let ord = cmp_bytes(DB[mid].0.as_bytes(), key);
        if ord == 0 {
            return Some(DB[mid].1);
        } else if ord < 0 {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    None
}

/// Docstring for `module.class.attr` without building the key string.
#[must_use]
pub const fn get_attr(module: &str, class: &str, attr: &str) -> Option<&'static str> {
    find_composed(
        b"",
        module.as_bytes(),
        class.as_bytes(),
        attr.as_bytes(),
        true,
    )
}

/// `module.class` or `module.class.attr`, with the same module aliases the
/// builtin types are stored under.
#[must_use]
pub const fn get_qualified(
    module: &str,
    class: &str,
    attr: Option<&str>,
    allow_builtins: bool,
) -> Option<&'static str> {
    if let Some(doc) = nonempty(find_named(b"", module, class, attr)) {
        return Some(doc);
    }
    if eq(module, "os") || eq(module, "_os") {
        if let Some(doc) = nonempty(find_named(b"", "posix", class, attr)) {
            return Some(doc);
        }
        if let Some(doc) = nonempty(find_named(b"", "nt", class, attr)) {
            return Some(doc);
        }
    }
    if let Some(stripped) = strip_underscore(module)
        && let Some(doc) = nonempty(find_named(b"", stripped, class, attr))
    {
        return Some(doc);
    }
    if !module.is_empty()
        && !starts_with_underscore(module)
        && let Some(doc) = nonempty(find_named(b"_", module, class, attr))
    {
        return Some(doc);
    }
    if allow_builtins && !eq(module, "builtins") && !starts_with_underscore(module) {
        return nonempty(find_named(b"", "builtins", class, attr));
    }
    None
}

/// Docstring for a class attribute. `module == None` means `builtins`.
#[must_use]
pub const fn class_attr_doc(module: Option<&str>, class: &str, attr: &str) -> Option<&'static str> {
    let module = match module {
        Some(module) => module,
        None => "builtins",
    };
    get_qualified(module, class, Some(attr), true)
}

#[cfg(test)]
mod test {
    use super::{DB, class_attr_doc, get, get_attr};

    #[test]
    fn db_sorted_unique() {
        assert!(!DB.is_empty());
        let mut i = 1;
        while i < DB.len() {
            assert!(DB[i - 1].0 < DB[i].0, "{}", DB[i].0);
            i += 1;
        }
    }

    #[test]
    fn get_hits_ends_and_middle() {
        assert_eq!(get(DB[0].0), Some(DB[0].1));
        assert_eq!(get(DB[DB.len() / 2].0), Some(DB[DB.len() / 2].1));
        assert_eq!(get(DB[DB.len() - 1].0), Some(DB[DB.len() - 1].1));
        assert_eq!(get("no.such.key"), None);
    }

    #[test]
    fn get_attr_and_alias() {
        let doc = get_attr("builtins", "int", "__add__").unwrap();
        assert_eq!(doc, get("builtins.int.__add__").unwrap());
        assert_eq!(
            super::get_qualified("os", "stat_result", None, true),
            get("posix.stat_result")
        );
        assert_eq!(
            class_attr_doc(None, "int", "__add__"),
            get_attr("builtins", "int", "__add__")
        );
    }
}
