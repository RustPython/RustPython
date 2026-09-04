// spell-checker: disable

//! OID (Object Identifier) management for SSL/TLS
//!
//! `txt2obj` and `nid2obj` have to answer with OpenSSL's own numbers, so the
//! table behind them is read out of OpenSSL's own object database rather than
//! transcribed by hand: `obj_mac.num` carries every object's NID, and
//! `objects.txt` carries the numeric OID, short name and description for the
//! same objects.  `objects.pl` is what OpenSSL generates its C tables from,
//! and the identifier it derives per object is what joins the two files here:
//! the `!Cname` override if one precedes the object, else its description,
//! else its short name -- each qualified by the enclosing `!module` and with
//! `-` spelled `_`.

use alloc::rc::Rc;
use std::collections::HashMap;

/// OID entry with openssl-compatible metadata
#[derive(Debug, Clone)]
pub(super) struct OidEntry {
    /// NID (OpenSSL Numerical Identifier) - must match CPython/OpenSSL values
    pub nid: i32,
    /// Short name (e.g., "CN", "serverAuth")
    pub short_name: &'static str,
    /// Long name/description (e.g., "commonName", "TLS Web Server Authentication")
    pub long_name: &'static str,
    /// Dotted-decimal form, rendered once while the table is built.  `None`
    /// for the objects OpenSSL names but assigns no OID -- cipher and digest
    /// algorithm names like `DES-EDE3` -- whose `nid2obj` reports it as such.
    oid: Option<Box<str>>,
}

impl OidEntry {
    /// Get OID as string (e.g., "2.5.4.3"), if this object has one
    pub(super) fn oid_string(&self) -> Option<&str> {
        self.oid.as_deref()
    }
}

/// OID table with multiple indices for fast lookup
pub(super) struct OidTable {
    /// All entries
    entries: Vec<OidEntry>,
    /// NID -> index mapping
    nid_to_idx: HashMap<i32, usize>,
    /// Short name -> index mapping
    short_name_to_idx: HashMap<&'static str, usize>,
    /// Long name -> index mapping
    long_name_to_idx: HashMap<&'static str, usize>,
    /// OID string -> index mapping
    oid_str_to_idx: HashMap<String, usize>,
}

impl OidTable {
    fn build() -> Self {
        let entries = build_oid_entries();
        let mut nid_to_idx = HashMap::with_capacity(entries.len());
        let mut short_name_to_idx = HashMap::with_capacity(entries.len());
        let mut long_name_to_idx = HashMap::with_capacity(entries.len());
        let mut oid_str_to_idx = HashMap::with_capacity(entries.len());

        // OpenSSL's own database repeats a name, and an OID, across objects;
        // its lookups answer with the first such object, so keep the first.
        for (idx, entry) in entries.iter().enumerate() {
            nid_to_idx.entry(entry.nid).or_insert(idx);
            short_name_to_idx.entry(entry.short_name).or_insert(idx);
            long_name_to_idx.entry(entry.long_name).or_insert(idx);
            if let Some(oid) = entry.oid_string() {
                oid_str_to_idx.entry(oid.to_owned()).or_insert(idx);
            }
        }

        Self {
            entries,
            nid_to_idx,
            short_name_to_idx,
            long_name_to_idx,
            oid_str_to_idx,
        }
    }

    pub(super) fn find_by_nid(&self, nid: i32) -> Option<&OidEntry> {
        self.nid_to_idx.get(&nid).map(|&idx| &self.entries[idx])
    }

    pub(super) fn find_by_oid_string(&self, oid_str: &str) -> Option<&OidEntry> {
        self.oid_str_to_idx
            .get(canonical_oid(oid_str)?.as_str())
            .map(|&idx| &self.entries[idx])
    }

    pub(super) fn find_by_name(&self, name: &str) -> Option<&OidEntry> {
        // OpenSSL object names are case-sensitive. Try the short name first,
        // as OBJ_txt2obj does when a short and long name collide.
        self.short_name_to_idx
            .get(name)
            .or_else(|| self.long_name_to_idx.get(name))
            .map(|&idx| &self.entries[idx])
    }
}

/// Parse the numeric form accepted by OpenSSL and render it canonically for
/// lookup. The first arc may not have leading zeroes, while later arcs may.
fn canonical_oid(oid: &str) -> Option<String> {
    let oid = oid.trim_end_matches(' ');
    let oid = oid.strip_suffix('.').unwrap_or(oid);
    let mut parts = oid.split('.');
    let first = parts.next()?;
    if !matches!(first, "0" | "1" | "2") {
        return None;
    }

    let mut canonical = first.to_owned();
    let mut count = 1;
    for part in parts {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let part = part.trim_start_matches('0');
        canonical.push('.');
        canonical.push_str(if part.is_empty() { "0" } else { part });
        count += 1;
    }
    (count >= 2).then_some(canonical)
}

/// Global OID table
static OID_TABLE: rustpython_common::lock::LazyLock<OidTable> =
    rustpython_common::lock::LazyLock::new(OidTable::build);

/// OpenSSL's NID assignments: one `identifier<whitespace>nid` line per object.
static OBJ_MAC_NUM: &str = include_str!("../../rustls-data/obj_mac.num");

/// OpenSSL's object database: the numeric OID, short name and description of
/// each object, with `!Alias` / `!Cname` / `!module` / `!global` directives
/// between them.
static OBJECTS_TXT: &str = include_str!("../../rustls-data/objects.txt");

/// The name `objects.pl` derives for an object or alias: qualified by the
/// enclosing `!module`, with `-` spelled `_`.
fn qualify(name: &str, module: Option<&str>) -> Rc<str> {
    let name = match module {
        Some(module) => format!("{module}_{name}"),
        None => name.to_owned(),
    };
    name.replace('-', "_").into()
}

/// Expand an OID written as a mix of arc numbers and earlier names into the
/// numbers alone.  Resolving against `aliases` as we go is what keeps the
/// expansion non-recursive: a name can only refer to something already read.
fn resolve_oid<'a>(
    parts: impl Iterator<Item = &'a str>,
    aliases: &HashMap<Rc<str>, Rc<[u64]>>,
) -> Rc<[u64]> {
    let mut oid = Vec::with_capacity(16);
    for part in parts {
        match aliases.get(part.replace('-', "_").as_str()) {
            Some(prefix) => oid.extend_from_slice(prefix),
            None => oid.push(part.parse().expect("OID arc is not a number")),
        }
    }
    oid.into()
}

fn dotted(oid: &[u64]) -> Box<str> {
    oid.iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(".")
        .into_boxed_str()
}

/// Build the complete OID table out of OpenSSL's object database
fn build_oid_entries() -> Vec<OidEntry> {
    let nids: HashMap<&str, i32> = OBJ_MAC_NUM
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut field = line.split_whitespace();
            let name = field.next()?;
            let nid = field.next()?.parse().ok()?;
            Some((name, nid))
        })
        .collect();

    let mut entries = Vec::with_capacity(nids.len());
    let mut aliases: HashMap<Rc<str>, Rc<[u64]>> = HashMap::new();
    let mut cname: Option<&str> = None;
    let mut module: Option<&str> = None;

    for line in OBJECTS_TXT
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        if let Some(directive) = line.strip_prefix('!') {
            let mut field = directive.split_whitespace();
            match field.next() {
                Some("Alias") => {
                    let alias = field.next().expect("!Alias without a name");
                    let oid = resolve_oid(field, &aliases);
                    aliases.insert(qualify(alias, module), oid);
                }
                Some("Cname") => cname = Some(field.next().expect("!Cname without a name")),
                Some("module") => module = Some(field.next().expect("!module without a name")),
                Some("global") => module = None,
                other => panic!("unknown objects.txt directive {other:?}"),
            }
            continue;
        }

        let mut field = line.split(':').map(str::trim);
        let oid = resolve_oid(
            field
                .next()
                .expect("object without an OID")
                .split_whitespace(),
            &aliases,
        );
        // An object written with only one name carries it as both: OpenSSL's
        // `OBJ_nid2sn(NID_pkcs9_emailAddress)` answers "emailAddress" for a
        // line whose short-name column is blank.
        let (short_name, long_name) = match (field.next().unwrap_or(""), field.next().unwrap_or(""))
        {
            ("", "") => continue,
            ("", name) | (name, "") => (name, name),
            (short_name, long_name) => (short_name, long_name),
        };

        // Every name this object can be reached by, most specific first.  They
        // become aliases for later objects to build on, and the first of them
        // that `obj_mac.num` knows carries this object's NID.
        let mut identifiers: Vec<Rc<str>> = Vec::with_capacity(3);
        for name in [cname.take(), Some(long_name), Some(short_name)]
            .into_iter()
            .flatten()
            .filter(|name| !name.is_empty())
        {
            let name = qualify(name, module);
            if !identifiers.contains(&name) {
                aliases.entry(name.clone()).or_insert_with(|| oid.clone());
                identifiers.push(name);
            }
        }

        let Some(nid) = identifiers
            .iter()
            .find_map(|name| nids.get(&**name).copied())
        else {
            continue;
        };
        entries.push(OidEntry {
            nid,
            short_name,
            long_name,
            // A single arc does not encode: DER packs the first two into one
            // byte, so OpenSSL gives the bare roots -- `iso`, `itu-t` -- a NID
            // and no OID, exactly as it does for the algorithm names above.
            oid: (oid.len() >= 2).then(|| dotted(&oid)),
        });
    }

    assert!(cname.is_none(), "!Cname with no object after it");
    entries
}

// Public API Functions

/// Find OID entry by NID
pub(super) fn find_by_nid(nid: i32) -> Option<&'static OidEntry> {
    OID_TABLE.find_by_nid(nid)
}

/// Find OID entry by OID string (e.g., "2.5.4.3")
pub(super) fn find_by_oid_string(oid_str: &str) -> Option<&'static OidEntry> {
    OID_TABLE.find_by_oid_string(oid_str)
}

/// Find OID entry by name (short or long name)
pub(super) fn find_by_name(name: &str) -> Option<&'static OidEntry> {
    OID_TABLE.find_by_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_by_nid_ok() {
        let entry = find_by_nid(13).unwrap();
        assert_eq!(entry.short_name, "CN");
        assert_eq!(entry.long_name, "commonName");
        assert_eq!(entry.oid_string(), Some("2.5.4.3"));
    }

    #[test]
    fn find_by_oid_string_ok() {
        let entry = find_by_oid_string("2.5.4.3").unwrap();
        assert_eq!(entry.nid, 13);
        assert_eq!(entry.short_name, "CN");
    }

    #[test]
    fn find_by_name_short() {
        let entry = find_by_name("CN").unwrap();
        assert_eq!(entry.nid, 13);
        assert_eq!(entry.oid_string(), Some("2.5.4.3"));
    }

    #[test]
    fn find_by_name_long() {
        let entry = find_by_name("commonName").unwrap();
        assert_eq!(entry.nid, 13);
        assert_eq!(entry.short_name, "CN");
    }

    #[test]
    fn find_by_name_is_case_sensitive() {
        assert!(find_by_name("commonName").is_some());
        assert!(find_by_name("COMMONNAME").is_none());
    }

    #[test]
    fn find_by_oid_string_normalizes_arcs() {
        let entry = find_by_oid_string("2.005.004.003.").unwrap();
        assert_eq!(entry.nid, 13);
        assert_eq!(entry.oid_string(), Some("2.5.4.3"));
    }

    #[test]
    fn subject_alt_name() {
        let entry = find_by_nid(85).unwrap();
        assert_eq!(entry.short_name, "subjectAltName");
        assert_eq!(entry.oid_string(), Some("2.5.29.17"));
    }

    #[test]
    fn server_auth_eku() {
        let entry = find_by_nid(129).unwrap();
        assert_eq!(entry.short_name, "serverAuth");
        assert_eq!(entry.oid_string(), Some("1.3.6.1.5.5.7.3.1"));
    }

    #[test]
    fn no_duplicate_nids() {
        let table = &*OID_TABLE;
        assert_eq!(
            table.entries.len(),
            table.nid_to_idx.len(),
            "Duplicate NIDs detected!"
        );
    }

    #[test]
    fn oid_count() {
        let table = &*OID_TABLE;
        // The table is OpenSSL's whole object database, not a curated subset.
        assert!(
            table.entries.len() >= 1000,
            "Expected at least 1000 OIDs, got {}",
            table.entries.len()
        );
    }

    #[test]
    fn nids_agree_with_openssl() {
        // Spot-check objects whose NIDs Python code and test suites depend on.
        // Every row is what `_ssl.nid2obj(nid)` answers on CPython built
        // against OpenSSL, covering both name shapes: a distinct short and
        // long name, and a single name standing for both.
        for (nid, short_name, long_name, oid) in [
            (13, "CN", "commonName", Some("2.5.4.3")),
            (
                48,
                "emailAddress",
                "emailAddress",
                Some("1.2.840.113549.1.9.1"),
            ),
            (
                85,
                "subjectAltName",
                "X509v3 Subject Alternative Name",
                Some("2.5.29.17"),
            ),
            (105, "serialNumber", "serialNumber", Some("2.5.4.5")),
            (
                129,
                "serverAuth",
                "TLS Web Server Authentication",
                Some("1.3.6.1.5.5.7.3.1"),
            ),
            (660, "street", "streetAddress", Some("2.5.4.9")),
            // Two arcs is still an object, and OpenSSL names some objects
            // with no OID at all.
            (11, "X500", "directory services (X.500)", Some("2.5")),
            (181, "ISO", "iso", None),
            (33, "DES-EDE3", "des-ede3", None),
            (114, "MD5-SHA1", "md5-sha1", None),
            (405, "ansi-X9-62", "ANSI X9.62", Some("1.2.840.10045")),
            (
                983,
                "md_gost12_512",
                "GOST R 34.11-2012 with 512 bit hash",
                Some("1.2.643.7.1.1.2.3"),
            ),
        ] {
            let entry = find_by_nid(nid).unwrap_or_else(|| panic!("no entry for NID {nid}"));
            assert_eq!(entry.short_name, short_name, "NID {nid}");
            assert_eq!(entry.long_name, long_name, "NID {nid}");
            assert_eq!(entry.oid_string(), oid, "NID {nid}");
        }
    }
}
