#![no_std]

include!(concat!(env!("OUT_DIR"), "/index.rs"));

pub static BLOB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/docs.xz"));

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
) -> Option<DocRef> {
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

const fn nonempty(doc: Option<DocRef>) -> Option<DocRef> {
    match doc {
        Some(doc) if doc.len != 0 || !doc.listed => Some(doc),
        _ => None,
    }
}

const fn find_named(
    prefix: &[u8],
    module: &str,
    class: &str,
    attr: Option<&str>,
) -> Option<DocRef> {
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

/// Location of an exact `module.class.attr` or `module.name` key in the blob.
/// `attr == None` looks up `module.class`.
#[must_use]
pub const fn get(key: &str) -> Option<DocRef> {
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

/// Location of `module.class.attr` without building the key string.
#[must_use]
pub const fn get_attr(module: &str, class: &str, attr: &str) -> Option<DocRef> {
    find_composed(
        b"",
        module.as_bytes(),
        class.as_bytes(),
        attr.as_bytes(),
        true,
    )
}

/// `module.class` or `module.class.attr`, with the same module aliases the
/// builtin types are stored under. Empty entries are skipped.
#[must_use]
pub const fn get_qualified(
    module: &str,
    class: &str,
    attr: Option<&str>,
    allow_builtins: bool,
) -> Option<DocRef> {
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

/// Location of a class attribute doc. `module == None` means `builtins`.
#[must_use]
pub const fn class_attr_doc(module: Option<&str>, class: &str, attr: &str) -> Option<DocRef> {
    let module = match module {
        Some(module) => module,
        None => "builtins",
    };
    get_qualified(module, class, Some(attr), true)
}

#[cfg(test)]
extern crate alloc;

#[cfg(test)]
pub mod plain {
    include!("data.inc.rs");
}

#[cfg(test)]
mod test {
    use super::{BLOB, DB, class_attr_doc, get, get_attr};
    use crate::plain;

    fn text() -> alloc::string::String {
        let bytes = xz::decode_all(BLOB).unwrap();
        alloc::string::String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn db_sorted_unique_and_roundtrip() {
        assert!(!DB.is_empty());
        assert_eq!(DB.len(), plain::DB.len());
        let text = text();
        let mut i = 0;
        while i < DB.len() {
            if i > 0 {
                assert!(DB[i - 1].0 < DB[i].0, "{}", DB[i].0);
            }
            assert_eq!(DB[i].0, plain::DB[i].0);
            let doc = &DB[i].1;
            assert_eq!(doc.key, DB[i].0);
            if doc.listed {
                let start = doc.offset as usize;
                let end = start + doc.len as usize;
                assert_eq!(&text[start..end], plain::DB[i].1);
            }
            i += 1;
        }
    }

    #[test]
    fn get_hits_ends_and_middle() {
        assert_eq!(get(DB[0].0).unwrap().offset, DB[0].1.offset);
        assert_eq!(get(DB[DB.len() / 2].0).unwrap().len, DB[DB.len() / 2].1.len);
        assert_eq!(
            get(DB[DB.len() - 1].0).unwrap().offset,
            DB[DB.len() - 1].1.offset
        );
        assert!(get("no.such.key").is_none());
    }

    #[test]
    fn get_attr_and_alias() {
        let doc = get_attr("builtins", "int", "__add__").unwrap();
        assert_eq!(doc.offset, get("builtins.int.__add__").unwrap().offset);
        assert_eq!(
            super::get_qualified("os", "stat_result", None, true)
                .unwrap()
                .offset,
            get("posix.stat_result").unwrap().offset
        );
        assert_eq!(
            class_attr_doc(None, "int", "__add__").unwrap().offset,
            get_attr("builtins", "int", "__add__").unwrap().offset
        );
    }
}
