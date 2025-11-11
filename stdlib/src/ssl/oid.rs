// spell-checker: disable

//! OID (Object Identifier) management for SSL/TLS
//!
//! This module provides OID lookup functionality compatible with CPython's ssl module.
//! It uses oid-registry crate for well-known OIDs while maintaining NID (Numerical Identifier)
//! mappings for CPython compatibility.

use oid_registry::asn1_rs::Oid;
use std::collections::HashMap;

/// OID entry with openssl-compatible metadata
#[derive(Debug, Clone)]
pub struct OidEntry {
    /// NID (OpenSSL Numerical Identifier) - must match CPython/OpenSSL values
    pub nid: i32,
    /// Short name (e.g., "CN", "serverAuth")
    pub short_name: &'static str,
    /// Long name/description (e.g., "commonName", "TLS Web Server Authentication")
    pub long_name: &'static str,
    /// OID reference (static or dynamic)
    pub oid: OidRef,
}

/// OID reference - either from oid-registry or runtime-created
#[derive(Debug, Clone)]
pub enum OidRef {
    /// Static OID from oid-registry crate (stored as value)
    Static(Oid<'static>),
    /// OID string (for OIDs not in oid-registry) - parsed on demand
    String(&'static str),
}

impl OidEntry {
    /// Create entry from oid-registry static constant
    pub fn from_static(
        nid: i32,
        short_name: &'static str,
        long_name: &'static str,
        oid: &Oid<'static>,
    ) -> Self {
        Self {
            nid,
            short_name,
            long_name,
            oid: OidRef::Static(oid.clone()),
        }
    }

    /// Create entry from OID string (for OIDs not in oid-registry)
    pub const fn from_string(
        nid: i32,
        short_name: &'static str,
        long_name: &'static str,
        oid_str: &'static str,
    ) -> Self {
        Self {
            nid,
            short_name,
            long_name,
            oid: OidRef::String(oid_str),
        }
    }

    /// Get OID as string (e.g., "2.5.4.3")
    pub fn oid_string(&self) -> String {
        match &self.oid {
            OidRef::Static(oid) => oid.to_id_string(),
            OidRef::String(s) => s.to_string(),
        }
    }
}

/// OID table with multiple indices for fast lookup
pub struct OidTable {
    /// All entries
    entries: Vec<OidEntry>,
    /// NID -> index mapping
    nid_to_idx: HashMap<i32, usize>,
    /// Short name -> index mapping
    short_name_to_idx: HashMap<&'static str, usize>,
    /// Long name -> index mapping (case-insensitive)
    long_name_to_idx: HashMap<String, usize>,
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

        for (idx, entry) in entries.iter().enumerate() {
            nid_to_idx.insert(entry.nid, idx);
            short_name_to_idx.insert(entry.short_name, idx);
            long_name_to_idx.insert(entry.long_name.to_lowercase(), idx);
            oid_str_to_idx.insert(entry.oid_string(), idx);
        }

        Self {
            entries,
            nid_to_idx,
            short_name_to_idx,
            long_name_to_idx,
            oid_str_to_idx,
        }
    }

    pub fn find_by_nid(&self, nid: i32) -> Option<&OidEntry> {
        self.nid_to_idx.get(&nid).map(|&idx| &self.entries[idx])
    }

    pub fn find_by_oid_string(&self, oid_str: &str) -> Option<&OidEntry> {
        self.oid_str_to_idx
            .get(oid_str)
            .map(|&idx| &self.entries[idx])
    }

    pub fn find_by_name(&self, name: &str) -> Option<&OidEntry> {
        // Try short name first (exact match)
        self.short_name_to_idx
            .get(name)
            .or_else(|| {
                // Try long name (case-insensitive)
                self.long_name_to_idx.get(&name.to_lowercase())
            })
            .map(|&idx| &self.entries[idx])
    }
}

/// Global OID table
static OID_TABLE: std::sync::LazyLock<OidTable> = std::sync::LazyLock::new(OidTable::build);

/// Macro to define OID entry using oid-registry constant
macro_rules! oid_static {
    ($nid:expr, $short:expr, $long:expr, $oid_const:path) => {
        OidEntry::from_static($nid, $short, $long, &$oid_const)
    };
}

/// Macro to define OID entry from string
macro_rules! oid_string {
    ($nid:expr, $short:expr, $long:expr, $oid_str:expr) => {
        OidEntry::from_string($nid, $short, $long, $oid_str)
    };
}

/// Build the complete OID table
fn build_oid_entries() -> Vec<OidEntry> {
    vec![
        // Priority 1: X.509 DN Attributes (OpenSSL NID values)
        // These NIDs MUST match OpenSSL for CPython compatibility
        oid_static!(13, "CN", "commonName", oid_registry::OID_X509_COMMON_NAME),
        oid_static!(14, "C", "countryName", oid_registry::OID_X509_COUNTRY_NAME),
        oid_static!(
            15,
            "L",
            "localityName",
            oid_registry::OID_X509_LOCALITY_NAME
        ),
        oid_static!(
            16,
            "ST",
            "stateOrProvinceName",
            oid_registry::OID_X509_STATE_OR_PROVINCE_NAME
        ),
        oid_static!(
            17,
            "O",
            "organizationName",
            oid_registry::OID_X509_ORGANIZATION_NAME
        ),
        oid_static!(
            18,
            "OU",
            "organizationalUnitName",
            oid_registry::OID_X509_ORGANIZATIONAL_UNIT
        ),
        oid_static!(41, "name", "name", oid_registry::OID_X509_NAME),
        oid_static!(42, "GN", "givenName", oid_registry::OID_X509_GIVEN_NAME),
        oid_static!(43, "initials", "initials", oid_registry::OID_X509_INITIALS),
        oid_static!(
            4,
            "serialNumber",
            "serialNumber",
            oid_registry::OID_X509_SERIALNUMBER
        ),
        oid_static!(100, "surname", "surname", oid_registry::OID_X509_SURNAME),
        // emailAddress is special - it's in PKCS#9, not X.509
        oid_static!(
            48,
            "emailAddress",
            "emailAddress",
            oid_registry::OID_PKCS9_EMAIL_ADDRESS
        ),
        // Priority 2: X.509 Extensions (Critical ones)
        oid_static!(
            82,
            "subjectKeyIdentifier",
            "X509v3 Subject Key Identifier",
            oid_registry::OID_X509_EXT_SUBJECT_KEY_IDENTIFIER
        ),
        oid_static!(
            83,
            "keyUsage",
            "X509v3 Key Usage",
            oid_registry::OID_X509_EXT_KEY_USAGE
        ),
        oid_static!(
            85,
            "subjectAltName",
            "X509v3 Subject Alternative Name",
            oid_registry::OID_X509_EXT_SUBJECT_ALT_NAME
        ),
        oid_static!(
            86,
            "issuerAltName",
            "X509v3 Issuer Alternative Name",
            oid_registry::OID_X509_EXT_ISSUER_ALT_NAME
        ),
        oid_static!(
            87,
            "basicConstraints",
            "X509v3 Basic Constraints",
            oid_registry::OID_X509_EXT_BASIC_CONSTRAINTS
        ),
        oid_static!(
            88,
            "crlNumber",
            "X509v3 CRL Number",
            oid_registry::OID_X509_EXT_CRL_NUMBER
        ),
        oid_static!(
            90,
            "authorityKeyIdentifier",
            "X509v3 Authority Key Identifier",
            oid_registry::OID_X509_EXT_AUTHORITY_KEY_IDENTIFIER
        ),
        oid_static!(
            126,
            "extendedKeyUsage",
            "X509v3 Extended Key Usage",
            oid_registry::OID_X509_EXT_EXTENDED_KEY_USAGE
        ),
        oid_static!(
            103,
            "crlDistributionPoints",
            "X509v3 CRL Distribution Points",
            oid_registry::OID_X509_EXT_CRL_DISTRIBUTION_POINTS
        ),
        oid_static!(
            89,
            "certificatePolicies",
            "X509v3 Certificate Policies",
            oid_registry::OID_X509_EXT_CERTIFICATE_POLICIES
        ),
        oid_static!(
            177,
            "authorityInfoAccess",
            "Authority Information Access",
            oid_registry::OID_PKIX_AUTHORITY_INFO_ACCESS
        ),
        oid_static!(
            105,
            "nameConstraints",
            "X509v3 Name Constraints",
            oid_registry::OID_X509_EXT_NAME_CONSTRAINTS
        ),
        // Priority 3: Extended Key Usage OIDs (not in oid-registry)
        // These are defined in RFC 5280 but not in oid-registry, so we use strings
        oid_string!(
            129,
            "serverAuth",
            "TLS Web Server Authentication",
            "1.3.6.1.5.5.7.3.1"
        ),
        oid_string!(
            130,
            "clientAuth",
            "TLS Web Client Authentication",
            "1.3.6.1.5.5.7.3.2"
        ),
        oid_string!(131, "codeSigning", "Code Signing", "1.3.6.1.5.5.7.3.3"),
        oid_string!(
            132,
            "emailProtection",
            "E-mail Protection",
            "1.3.6.1.5.5.7.3.4"
        ),
        oid_string!(133, "timeStamping", "Time Stamping", "1.3.6.1.5.5.7.3.8"),
        oid_string!(180, "OCSPSigning", "OCSP Signing", "1.3.6.1.5.5.7.3.9"),
        // Priority 4: Signature Algorithms
        oid_static!(
            6,
            "rsaEncryption",
            "rsaEncryption",
            oid_registry::OID_PKCS1_RSAENCRYPTION
        ),
        oid_static!(
            65,
            "sha1WithRSAEncryption",
            "sha1WithRSAEncryption",
            oid_registry::OID_PKCS1_SHA1WITHRSA
        ),
        oid_static!(
            668,
            "sha256WithRSAEncryption",
            "sha256WithRSAEncryption",
            oid_registry::OID_PKCS1_SHA256WITHRSA
        ),
        oid_static!(
            669,
            "sha384WithRSAEncryption",
            "sha384WithRSAEncryption",
            oid_registry::OID_PKCS1_SHA384WITHRSA
        ),
        oid_static!(
            670,
            "sha512WithRSAEncryption",
            "sha512WithRSAEncryption",
            oid_registry::OID_PKCS1_SHA512WITHRSA
        ),
        oid_static!(
            408,
            "id-ecPublicKey",
            "id-ecPublicKey",
            oid_registry::OID_KEY_TYPE_EC_PUBLIC_KEY
        ),
        oid_static!(
            794,
            "ecdsa-with-SHA256",
            "ecdsa-with-SHA256",
            oid_registry::OID_SIG_ECDSA_WITH_SHA256
        ),
        oid_static!(
            795,
            "ecdsa-with-SHA384",
            "ecdsa-with-SHA384",
            oid_registry::OID_SIG_ECDSA_WITH_SHA384
        ),
        oid_static!(
            796,
            "ecdsa-with-SHA512",
            "ecdsa-with-SHA512",
            oid_registry::OID_SIG_ECDSA_WITH_SHA512
        ),
        // Priority 5: Hash Algorithms
        oid_string!(64, "sha1", "sha1", "1.3.14.3.2.26"),
        oid_static!(672, "sha256", "sha256", oid_registry::OID_NIST_HASH_SHA256),
        oid_static!(673, "sha384", "sha384", oid_registry::OID_NIST_HASH_SHA384),
        oid_static!(674, "sha512", "sha512", oid_registry::OID_NIST_HASH_SHA512),
        oid_string!(675, "sha224", "sha224", "2.16.840.1.101.3.4.2.4"),
        // Priority 6: Elliptic Curve OIDs
        oid_static!(714, "secp256r1", "secp256r1", oid_registry::OID_EC_P256),
        oid_string!(715, "secp384r1", "secp384r1", "1.3.132.0.34"),
        oid_string!(716, "secp521r1", "secp521r1", "1.3.132.0.35"),
        oid_string!(1172, "X25519", "X25519", "1.3.101.110"),
        oid_string!(1173, "Ed25519", "Ed25519", "1.3.101.112"),
        // Additional useful OIDs
        oid_string!(
            183,
            "subjectInfoAccess",
            "Subject Information Access",
            "1.3.6.1.5.5.7.1.11"
        ),
        oid_string!(920, "OCSP", "OCSP", "1.3.6.1.5.5.7.48.1"),
        oid_string!(921, "caIssuers", "CA Issuers", "1.3.6.1.5.5.7.48.2"),
    ]
}

// Public API Functions

/// Find OID entry by NID
pub fn find_by_nid(nid: i32) -> Option<&'static OidEntry> {
    OID_TABLE.find_by_nid(nid)
}

/// Find OID entry by OID string (e.g., "2.5.4.3")
pub fn find_by_oid_string(oid_str: &str) -> Option<&'static OidEntry> {
    OID_TABLE.find_by_oid_string(oid_str)
}

/// Find OID entry by name (short or long name)
pub fn find_by_name(name: &str) -> Option<&'static OidEntry> {
    OID_TABLE.find_by_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_by_nid() {
        let entry = find_by_nid(13).unwrap();
        assert_eq!(entry.short_name, "CN");
        assert_eq!(entry.long_name, "commonName");
        assert_eq!(entry.oid_string(), "2.5.4.3");
    }

    #[test]
    fn test_find_by_oid_string() {
        let entry = find_by_oid_string("2.5.4.3").unwrap();
        assert_eq!(entry.nid, 13);
        assert_eq!(entry.short_name, "CN");
    }

    #[test]
    fn test_find_by_name_short() {
        let entry = find_by_name("CN").unwrap();
        assert_eq!(entry.nid, 13);
        assert_eq!(entry.oid_string(), "2.5.4.3");
    }

    #[test]
    fn test_find_by_name_long() {
        let entry = find_by_name("commonName").unwrap();
        assert_eq!(entry.nid, 13);
        assert_eq!(entry.short_name, "CN");
    }

    #[test]
    fn test_find_by_name_case_insensitive() {
        let entry = find_by_name("COMMONNAME").unwrap();
        assert_eq!(entry.nid, 13);
    }

    #[test]
    fn test_subject_alt_name() {
        let entry = find_by_nid(85).unwrap();
        assert_eq!(entry.short_name, "subjectAltName");
        assert_eq!(entry.oid_string(), "2.5.29.17");
    }

    #[test]
    fn test_server_auth_eku() {
        let entry = find_by_nid(129).unwrap();
        assert_eq!(entry.short_name, "serverAuth");
        assert_eq!(entry.oid_string(), "1.3.6.1.5.5.7.3.1");
    }

    #[test]
    fn test_no_duplicate_nids() {
        let table = &*OID_TABLE;
        assert_eq!(
            table.entries.len(),
            table.nid_to_idx.len(),
            "Duplicate NIDs detected!"
        );
    }

    #[test]
    fn test_oid_count() {
        let table = &*OID_TABLE;
        // We should have 50+ OIDs defined
        assert!(
            table.entries.len() >= 50,
            "Expected at least 50 OIDs, got {}",
            table.entries.len()
        );
    }
}
