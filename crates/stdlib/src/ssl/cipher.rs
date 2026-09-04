//! OpenSSL cipher strings, as `man openssl-ciphers` defines them.
//!
//! `SSLContext.set_ciphers()` takes the same string OpenSSL does, so the whole
//! grammar has to be honoured and not just the parts that look like cipher
//! names: `!` deletes a cipher and bars it from coming back, `-` deletes one
//! that a later term may restore, `+` moves matching ciphers to the end,
//! `@STRENGTH` sorts by key length, and `@SECLEVEL=n` drops everything below
//! the level's key size.  A term is itself a `+`-joined conjunction, so
//! `AES128+aECDSA` selects the intersection.
//!
//! What a term can name is likewise OpenSSL's: the `DEFAULT` / `ALL` /
//! `COMPLEMENTOFALL` groups, a protocol version, `aXXX` / `kXXX` / `eXXX` for
//! authentication, key exchange and cipher part, a bare part, or a full name
//! in either OpenSSL or IANA spelling.

use core::str::FromStr;
use rustls::{CipherSuite, SupportedCipherSuite, crypto::SupportedKxGroup};
use rustpython_common::lock::LazyLock;

use super::providers::CryptoExt;

// See `man SSL_CTX_set_security_level` for details.
const SECURITY_LEVEL_TO_MIN_BITS: &[u16] = &[0, 80, 112, 128, 192, 256];

pub(super) struct CipherList<'a> {
    ops: Vec<CipherFilterOp<'a>>,
}

enum CipherFilterOp<'a> {
    /// The cipher string @STRENGTH can be used at any point to sort the current cipher list in order of encryption
    /// algorithm key length.
    Strength,

    /// The cipher string @SECLEVEL=n can be used at any point to set the security level to n.
    SecLevel(usize),

    /// Just add matching ciphers to the end of the current list.
    Append(CipherFilterSubOpList<'a>),

    /// If ! is used then the ciphers are permanently deleted from the list. The ciphers deleted can never reappear
    /// in the list even if they are explicitly stated.
    DelAndBlock(CipherFilterSubOpList<'a>),

    /// If - is used then the ciphers are deleted from the list, but some or all of the ciphers can be added again
    /// by later options.
    Del(CipherFilterSubOpList<'a>),

    /// If + is used then the ciphers are moved to the end of the list. This option doesn't add any new ciphers it
    /// just moves matching existing ones.
    MoveToEnd(CipherFilterSubOpList<'a>),
}

struct CipherFilterSubOpList<'a> {
    sub_ops: Vec<CipherFilterSubOp<'a>>,
}

enum CipherFilterSubOp<'a> {
    /// Default cipher list. Valid only as a first operation.
    Default,

    /// The ciphers included in ALL, but not enabled by default.
    ComplementOfDefault,

    /// All cipher suites except the eNULL ciphers.
    All,

    /// The cipher suites not enabled by ALL, currently eNULL.
    ComplementOfAll,

    /// The list of enabled cipher suites will be loaded from the system crypto policy configuration file.
    ProfileSystem,

    /// "High" encryption cipher suites.
    High,

    /// "Medium" encryption cipher suites.
    Medium,

    /// "Low" encryption cipher suites.
    Low,

    /// Lists cipher suites which are only supported in at least TLS v1.0.
    TlsV10,

    /// Lists cipher suites which are only supported in at least TLS v1.2.
    TlsV12,

    /// Lists cipher suites which are only supported in at least SSL v3.
    SslV3,

    /// Enables suite B mode of operation.
    SuiteB(SuiteBType),

    /// All cipher suites using encryption algorithm in Cipher Block Chaining (CBC) mode.
    Cbc,

    /// AES in Galois Counter Mode (GCM): these cipher suites are only supported in TLS v1.2.
    AesGcm,

    /// Match by message authentication algorithm.
    Auth(&'a str),

    /// Match by key exchange algorithm.
    KeyEx(&'a str),

    /// Match by part of an OpenSSL name that usually contains key exchange algorithm and symmetric cipher
    /// and may contain other identifiers.
    Part(&'a str),

    /// Match by full OpenSSL or IANA cipher name.
    Full(&'a str),
}

enum SuiteBType {
    Use128Permit192,
    Use128Only,
    Use192Only,
}

impl SuiteBType {
    fn parameters(
        &self,
    ) -> (
        &'static [CipherSuite],
        &'static [(rustls::NamedGroup, &'static str)],
    ) {
        match self {
            Self::Use128Permit192 => (
                &[
                    CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
                    CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
                ],
                &[
                    (rustls::NamedGroup::secp256r1, "secp256r1"),
                    (rustls::NamedGroup::secp384r1, "secp384r1"),
                ],
            ),
            Self::Use128Only => (
                &[CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256],
                &[(rustls::NamedGroup::secp256r1, "secp256r1")],
            ),
            Self::Use192Only => (
                &[CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384],
                &[(rustls::NamedGroup::secp384r1, "secp384r1")],
            ),
        }
    }
}

impl<'a> CipherList<'a> {
    pub(super) fn parse_to_rustls(
        s: &'a str,
    ) -> Result<WithOptionSuiteB<Vec<SupportedCipherSuite>>, &'static str> {
        Self::parse(s)?.to_rustls()
    }

    fn parse(s: &'a str) -> Result<Self, &'static str> {
        let ops: Vec<_> = s
            .split(|c: char| c == ':' || c == ',' || c.is_ascii_whitespace())
            .filter(|s| !s.is_empty())
            .enumerate()
            .map(|(i, s)| {
                let suite_b = match s {
                    "SUITEB128" => Some(CipherFilterSubOp::SuiteB(SuiteBType::Use128Permit192)),
                    "SUITEB128ONLY" => Some(CipherFilterSubOp::SuiteB(SuiteBType::Use128Only)),
                    "SUITEB192" => Some(CipherFilterSubOp::SuiteB(SuiteBType::Use192Only)),
                    _ => None,
                };

                match (i, s, suite_b) {
                    (0, "DEFAULT", _) => Ok(CipherFilterOp::Append(
                        CipherFilterSubOpList::from_sub_op(CipherFilterSubOp::Default),
                    )),

                    (0, _, Some(suite_b)) => Ok(CipherFilterOp::Append(
                        CipherFilterSubOpList::from_sub_op(suite_b),
                    )),

                    (_, _, _) => CipherFilterOp::parse(s),
                }
            })
            .collect::<Result<_, _>>()?;
        if ops.is_empty() {
            Err("list of ciphers is empty")
        } else {
            Ok(Self { ops })
        }
    }

    fn to_rustls(&self) -> Result<WithOptionSuiteB<Vec<SupportedCipherSuite>>, &'static str> {
        let mut min_bits = SECURITY_LEVEL_TO_MIN_BITS[0];
        let mut block_list = Vec::new();
        let mut ids = Vec::new();

        let sanitize = |ids: &mut Vec<u16>, min_bits, block_list: &[u16]| {
            ids.retain(|id| CIPHER_MAPPINGS.entry(*id).bits >= min_bits);
            ids.retain(|id| !block_list.contains(id));
        };
        let extend = |ids: &mut Vec<u16>, source: &[u16]| {
            // Extend and deduplicate.
            for id in source {
                if !ids.contains(id) {
                    ids.push(*id);
                }
            }
        };
        let ids_to_suits = |ids: &[u16]| {
            ids.iter()
                .map(|id| *CIPHER_MAPPINGS.entry(*id).suite)
                .collect()
        };

        for op in &self.ops {
            match op {
                CipherFilterOp::Strength => {
                    ids.sort_by_key(|id| -i32::from(CIPHER_MAPPINGS.entry(*id).bits))
                }

                CipherFilterOp::SecLevel(level) => {
                    min_bits = *SECURITY_LEVEL_TO_MIN_BITS
                        .get(*level)
                        .ok_or("@SECLEVEL value too big")?;
                    sanitize(&mut ids, min_bits, &block_list);
                }

                CipherFilterOp::Append(sub_op_list) => {
                    let (mut new_ids, suite_b) = sub_op_list.to_rustls_ids()?;
                    if suite_b.is_some() {
                        // SUITEB* cipherstrings should appear first in the cipher list and anything
                        // after them is ignored.
                        return Ok((ids_to_suits(&new_ids), suite_b));
                    }
                    sanitize(&mut new_ids, min_bits, &block_list);
                    extend(&mut ids, &new_ids);
                }

                CipherFilterOp::DelAndBlock(sub_op_list) => {
                    extend(&mut block_list, &sub_op_list.to_rustls_ids()?.0);
                    sanitize(&mut ids, min_bits, &block_list);
                }

                CipherFilterOp::Del(sub_op_list) => {
                    let (del_ids, _) = sub_op_list.to_rustls_ids()?;
                    ids.retain(|id| !del_ids.contains(id));
                }

                CipherFilterOp::MoveToEnd(sub_op_list) => {
                    let (move_ids, _) = sub_op_list.to_rustls_ids()?;
                    ids.sort_by_key(|id| move_ids.contains(id))
                }
            }
        }

        Ok((ids_to_suits(&ids), None))
    }
}

impl<'a> CipherFilterOp<'a> {
    fn parse(mut s: &'a str) -> Result<Self, &'static str> {
        if s == "@STRENGTH" {
            return Ok(Self::Strength);
        }
        const SECLEVEL: &str = "@SECLEVEL=";
        if s.starts_with(SECLEVEL) {
            return Ok(Self::SecLevel(
                usize::from_str(s.get(SECLEVEL.len()..).unwrap_or(""))
                    .map_err(|_| "invalid @SECLEVEL value")?,
            ));
        }

        let prefix = s.get(..1).unwrap_or("");
        if ["!", "-", "+"].contains(&prefix) {
            s = s.get(1..).unwrap_or("");
        }
        Ok(match prefix {
            "!" => Self::DelAndBlock(CipherFilterSubOpList::parse(s)?),
            "-" => Self::Del(CipherFilterSubOpList::parse(s)?),
            "+" => Self::MoveToEnd(CipherFilterSubOpList::parse(s)?),
            _ => Self::Append(CipherFilterSubOpList::parse(s)?),
        })
    }
}

impl<'a> CipherFilterSubOpList<'a> {
    fn parse(s: &'a str) -> Result<Self, &'static str> {
        let sub_ops: Vec<_> = s
            .split('+')
            .map(CipherFilterSubOp::parse)
            .collect::<Result<_, _>>()?;
        if sub_ops.is_empty() {
            Err("list of cipher filtering operations is empty")
        } else {
            Ok(Self { sub_ops })
        }
    }

    fn from_sub_op(sub_op: CipherFilterSubOp<'a>) -> Self {
        Self {
            sub_ops: vec![sub_op],
        }
    }

    fn to_rustls_ids(&self) -> Result<WithOptionSuiteB<Vec<u16>>, &'static str> {
        let mut ids = Vec::new();
        let mut first = true;
        for sub_op in &self.sub_ops {
            match sub_op {
                CipherFilterSubOp::Default => {
                    Self::extend_or_intersect(&mut first, &mut ids, &CIPHER_MAPPINGS.default)
                }

                CipherFilterSubOp::ComplementOfDefault => Self::extend_or_intersect(
                    &mut first,
                    &mut ids,
                    &CIPHER_MAPPINGS.complement_of_default,
                ),

                CipherFilterSubOp::All => {
                    Self::extend_or_intersect(&mut first, &mut ids, &CIPHER_MAPPINGS.all)
                }

                CipherFilterSubOp::ComplementOfAll => Self::extend_or_intersect(
                    &mut first,
                    &mut ids,
                    &CIPHER_MAPPINGS.complement_of_all,
                ),

                CipherFilterSubOp::ProfileSystem => {
                    return Err(
                        "reading cipher suites from system crypto policy file is not supported with rustls",
                    );
                }

                // Every suite in the table encrypts with at least 128 bits,
                // which is what "high" asks for, and so nothing is left for
                // the two weaker grades to name.
                CipherFilterSubOp::High => {
                    Self::extend_or_intersect(&mut first, &mut ids, &CIPHER_MAPPINGS.all)
                }

                // Nor is there a suite older than TLS 1.2 to name.
                CipherFilterSubOp::Medium
                | CipherFilterSubOp::Low
                | CipherFilterSubOp::TlsV10
                | CipherFilterSubOp::SslV3 => Self::extend_or_intersect(&mut first, &mut ids, &[]),

                CipherFilterSubOp::TlsV12 => {
                    Self::extend_or_intersect(&mut first, &mut ids, &CIPHER_MAPPINGS.tls_1_2)
                }

                // RFC 6460
                CipherFilterSubOp::SuiteB(suite_b) => {
                    let (suites, groups) = suite_b.parameters();
                    let ids: Vec<u16> = suites.iter().map(|suite| u16::from(*suite)).collect();
                    CIPHER_MAPPINGS.validate_suite_b(&ids)?;
                    let groups = groups
                        .iter()
                        .map(|(group, name)| kx_group_by_name(*group, name))
                        .collect::<Result<_, _>>()?;
                    return Ok((ids, Some(groups)));
                }

                CipherFilterSubOp::Cbc => {
                    // OpenSSL names might contain either -CBC- or -CBC3-, IANA seems to only contain _CBC_.
                    let rhs = CIPHER_MAPPINGS
                        .select(|entry| entry.iana.split('_').any(|part| part == "CBC"));

                    Self::extend_or_intersect(&mut first, &mut ids, &rhs)
                }

                CipherFilterSubOp::AesGcm => {
                    let rhs = CIPHER_MAPPINGS.select(|entry| {
                        let parts = || entry.openssl.split(['-', '_']);
                        parts().any(|part| part.starts_with("AES"))
                            && parts().any(|part| part == "GCM")
                    });

                    Self::extend_or_intersect(&mut first, &mut ids, &rhs)
                }

                CipherFilterSubOp::Auth(auth) => {
                    let rhs = CIPHER_MAPPINGS.select(|entry| match entry.suite {
                        SupportedCipherSuite::Tls12(c) => c.sign.iter().any(|scheme| {
                            scheme
                                .as_str()
                                .is_some_and(|s| s.split('_').any(|s| s == *auth))
                        }),

                        // usable_for_signature_algorithm() always returns true for TLS 1.3.
                        SupportedCipherSuite::Tls13(_) => true,
                    });

                    Self::extend_or_intersect(&mut first, &mut ids, &rhs)
                }

                CipherFilterSubOp::KeyEx(key_ex) => {
                    let rhs = CIPHER_MAPPINGS.select(|entry| entry.key_ex == *key_ex);
                    Self::extend_or_intersect(&mut first, &mut ids, &rhs)
                }

                CipherFilterSubOp::Part(part) => {
                    let rhs = CIPHER_MAPPINGS.select(|entry| name_has_part(entry.openssl, part));
                    Self::extend_or_intersect(&mut first, &mut ids, &rhs)
                }

                CipherFilterSubOp::Full(full) => {
                    let rhs: Vec<_> = CIPHER_MAPPINGS.by_name(full).into_iter().collect();
                    Self::extend_or_intersect(&mut first, &mut ids, &rhs)
                }
            }
        }
        Ok((ids, None))
    }

    fn extend_or_intersect(first: &mut bool, lhs: &mut Vec<u16>, rhs: &[u16]) {
        if core::mem::take(first) {
            lhs.extend_from_slice(rhs)
        } else {
            lhs.retain(|id| rhs.contains(id))
        }
    }
}

pub(super) fn kx_group_by_name(
    name: rustls::NamedGroup,
    error_name: &'static str,
) -> Result<&'static dyn SupportedKxGroup, &'static str> {
    CryptoExt::get_ext()
        .all_kx_or_default()
        .iter()
        .find(|g| g.name() == name)
        .copied()
        .ok_or(error_name)
}

pub(super) fn kx_group_by_openssl_name(name: &str) -> Option<&'static dyn SupportedKxGroup> {
    CryptoExt::get_ext()
        .all_kx_or_default()
        .iter()
        .find(|group| kx_group_openssl_names(group.name()).contains(&name))
        .copied()
}

fn kx_group_openssl_names(name: rustls::NamedGroup) -> &'static [&'static str] {
    match name {
        rustls::NamedGroup::secp256r1 => &["prime256v1", "secp256r1"],
        rustls::NamedGroup::secp384r1 => &["secp384r1", "prime384v1"],
        rustls::NamedGroup::secp521r1 => &["secp521r1", "prime521v1"],
        rustls::NamedGroup::X25519 => &["X25519", "x25519"],
        rustls::NamedGroup::X448 => &["X448", "x448"],
        rustls::NamedGroup::MLKEM768 => &["MLKEM768"],
        rustls::NamedGroup::MLKEM1024 => &["MLKEM1024"],
        rustls::NamedGroup::secp256r1MLKEM768 => &["SecP256r1MLKEM768", "secp256r1MLKEM768"],
        rustls::NamedGroup::X25519MLKEM768 => &["X25519MLKEM768"],
        _ => &[],
    }
}

type WithOptionSuiteB<T> = (T, Option<Vec<&'static dyn SupportedKxGroup>>);

impl<'a> CipherFilterSubOp<'a> {
    fn parse(mut s: &'a str) -> Result<Self, &'static str> {
        let wrong_position = match s {
            "DEFAULT" => Some("DEFAULT specified at wrong position in the cipher string"),
            "SUITEB128" => Some("SUITEB128 specified at wrong position in the cipher string"),
            "SUITEB128ONLY" => {
                Some("SUITEB128ONLY specified at wrong position in the cipher string")
            }
            "SUITEB192" => Some("SUITEB192 specified at wrong position in the cipher string"),
            _ => None,
        };
        if let Some(error) = wrong_position {
            return Err(error);
        }

        Ok(match s {
            "COMPLEMENTOFDEFAULT" => Self::ComplementOfDefault,
            "ALL" => Self::All,
            "COMPLEMENTOFALL" => Self::ComplementOfAll,
            "PROFILE=SYSTEM" => Self::ProfileSystem,
            "HIGH" => Self::High,
            "MEDIUM" => Self::Medium,
            "LOW" => Self::Low,
            "TLSv1.0" => Self::TlsV10,
            "TLSv1.2" => Self::TlsV12,
            "SSLv3" => Self::SslV3,
            "CBC" => Self::Cbc,
            "AESGCM" => Self::AesGcm,

            // RSA is an alias for kRSA.
            "RSA" => Self::KeyEx("RSA"),

            // Every suite in the table agrees an ephemeral ECDH key, which
            // ECDH selects along with the fixed and anonymous forms it also
            // covers, and which ECDHE and EECDH select on their own.
            "ECDH" | "ECDHE" | "EECDH" => Self::KeyEx(EPHEMERAL_ECDH),

            _ => {
                let prefix = s.get(..1).unwrap_or("");
                if ["a", "k", "e"].contains(&prefix) {
                    s = s.get(1..).unwrap_or("");
                }

                if s.is_empty() {
                    return Err("item of cipher string is empty");
                }
                if !s
                    .chars()
                    .all(|c| char::is_ascii_alphanumeric(&c) || matches!(c, '-' | '_'))
                {
                    return Err("item of cipher string contains invalid characters");
                }

                match prefix {
                    "a" => Self::Auth(s),
                    "k" => Self::KeyEx(key_ex_alias(s)),
                    "e" => Self::Part(s),

                    _ => {
                        if s.contains(['_', '-']) {
                            Self::Full(s)
                        } else {
                            Self::Part(s)
                        }
                    }
                }
            }
        })
    }
}

/// What `SSLContext.get_ciphers()` reports about one suite, laid out the way
/// `SSL_CIPHER_description` lays it out, padding included.
pub(super) struct CipherDescription {
    pub id: u32,
    pub name: &'static str,
    pub protocol: &'static str,
    pub bits: u16,
    pub description: String,
}

pub(super) fn describe(suite: &SupportedCipherSuite) -> CipherDescription {
    let entry = CIPHER_MAPPINGS.entry(suite.suite().into());
    let tls13 = suite.tls13().is_some();
    let protocol = if tls13 { "TLSv1.3" } else { "TLSv1.2" };
    // A TLS 1.3 suite names neither the key agreement nor the authentication;
    // the handshake settles both apart from the suite.
    let key_ex = if tls13 { "any" } else { entry.key_ex };

    CipherDescription {
        // A cipher is numbered with the protocol it belongs to in the high
        // half, and every TLS suite belongs to 0x0300.
        id: 0x0300_0000 | u32::from(entry.id),
        name: entry.openssl,
        protocol,
        bits: entry.bits,
        description: format!(
            "{:<30} {protocol} Kx={key_ex:<8} Au={:<5} Enc={:<22} Mac=AEAD",
            entry.openssl, entry.auth, entry.encryption
        ),
    }
}

/// How the table spells the key agreement every suite in it uses.
const EPHEMERAL_ECDH: &str = "ECDH";

/// `kECDHE` and `kEECDH` name that same agreement.
fn key_ex_alias(key_ex: &str) -> &str {
    match key_ex {
        "ECDHE" | "EECDH" => EPHEMERAL_ECDH,
        _ => key_ex,
    }
}

/// Whether an OpenSSL name is one a cipher-string part selects.  A part
/// usually names one dash-separated piece of it, but `AES` stands for either
/// key length, and the digest names stand for a MAC that no suite in the
/// table has -- the SHA in an AEAD suite's name is its handshake hash.
fn name_has_part(openssl: &str, part: &str) -> bool {
    let mut parts = openssl.split(['-', '_']);
    match part {
        "AES" => parts.any(|p| p.starts_with("AES")),
        "SHA" | "SHA1" | "SHA256" | "SHA384" => false,
        _ => parts.any(|p| p == part),
    }
}

static CIPHER_MAPPINGS: LazyLock<CipherMappings> = LazyLock::new(CipherMappings::new);

/// What the table knows about one suite the provider offers.
struct CipherEntry {
    id: u16,
    openssl: &'static str,
    iana: &'static str,
    key_ex: &'static str,
    auth: &'static str,
    encryption: &'static str,
    bits: u16,
    suite: &'static SupportedCipherSuite,
}

struct CipherMappings {
    /// Every suite the provider offers, in the order it offers them, which is
    /// the preference order a selection out of it keeps.
    entries: Vec<CipherEntry>,
    complement_of_default: Vec<u16>,
    complement_of_all: Vec<u16>,
    default: Vec<u16>,
    all: Vec<u16>,
    tls_1_2: Vec<u16>,
}

impl CipherMappings {
    fn find(&self, id: u16) -> Option<&CipherEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    fn entry(&self, id: u16) -> &CipherEntry {
        self.find(id)
            .expect("BUG: cipher id is not one of the table's")
    }

    fn validate_suite_b(&self, ids: &[u16]) -> Result<(), &'static str> {
        if ids.iter().all(|id| self.find(*id).is_some()) {
            Ok(())
        } else {
            Err("Suite B cipher is not supported by crypto provider")
        }
    }

    fn select(&self, matches: impl Fn(&CipherEntry) -> bool) -> Vec<u16> {
        self.entries
            .iter()
            .filter(|entry| matches(entry))
            .map(|entry| entry.id)
            .collect()
    }

    fn by_name(&self, name: &str) -> Option<u16> {
        self.entries
            .iter()
            .find(|entry| entry.openssl == name || entry.iana == name)
            .map(|entry| entry.id)
    }

    fn default_cipher_string(&self) -> String {
        self.default
            .iter()
            .map(|id| self.entry(*id).openssl)
            .collect::<Vec<_>>()
            .join(":")
    }

    fn new() -> Self {
        let all_cipher_suites = CryptoExt::get_ext().all_ciphers_or_default();
        let default_cipher_suites = CryptoExt::get_ext().default_ciphers_or_provider();

        let mut entries = Vec::with_capacity(all_cipher_suites.len());
        let mut all = Vec::with_capacity(all_cipher_suites.len());
        let mut tls_1_2 = Vec::with_capacity(all_cipher_suites.len());

        for cipher in all_cipher_suites {
            // See https://www.ssl.org/cipher-suite-mapping
            let (openssl, iana, key_ex, auth, enc, bits, min_tls_ver) = match cipher.suite() {
                CipherSuite::TLS13_AES_256_GCM_SHA384 => (
                    "TLS_AES_256_GCM_SHA384",
                    "TLS_AES_256_GCM_SHA384",
                    "ECDH",
                    "any",
                    "AESGCM(256)",
                    256,
                    13,
                ),

                CipherSuite::TLS13_AES_128_GCM_SHA256 => (
                    "TLS_AES_128_GCM_SHA256",
                    "TLS_AES_128_GCM_SHA256",
                    "ECDH",
                    "any",
                    "AESGCM(128)",
                    128,
                    13,
                ),

                CipherSuite::TLS13_CHACHA20_POLY1305_SHA256 => (
                    "TLS_CHACHA20_POLY1305_SHA256",
                    "TLS_CHACHA20_POLY1305_SHA256",
                    "ECDH",
                    "any",
                    "CHACHA20/POLY1305(256)",
                    256,
                    13,
                ),

                CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384 => (
                    "ECDHE-ECDSA-AES256-GCM-SHA384",
                    "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384",
                    "ECDH",
                    "ECDSA",
                    "AESGCM(256)",
                    256,
                    12,
                ),

                CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 => (
                    "ECDHE-ECDSA-AES128-GCM-SHA256",
                    "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256",
                    "ECDH",
                    "ECDSA",
                    "AESGCM(128)",
                    128,
                    12,
                ),

                CipherSuite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256 => (
                    "ECDHE-ECDSA-CHACHA20-POLY1305",
                    "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256",
                    "ECDH",
                    "ECDSA",
                    "CHACHA20/POLY1305(256)",
                    256,
                    12,
                ),

                CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 => (
                    "ECDHE-RSA-AES256-GCM-SHA384",
                    "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384",
                    "ECDH",
                    "RSA",
                    "AESGCM(256)",
                    256,
                    12,
                ),

                CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 => (
                    "ECDHE-RSA-AES128-GCM-SHA256",
                    "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256",
                    "ECDH",
                    "RSA",
                    "AESGCM(128)",
                    128,
                    12,
                ),

                CipherSuite::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256 => (
                    "ECDHE-RSA-CHACHA20-POLY1305",
                    "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256",
                    "ECDH",
                    "RSA",
                    "CHACHA20/POLY1305(256)",
                    256,
                    12,
                ),

                // This is tested by that_all_rustls_ciphers_are_known().
                // This may happen after rustls update, just add more ciphers above is this case.
                _ => unreachable!("BUG: Unknown cipher suite {cipher:?}"),
            };

            let id = cipher.suite().into();

            if bits > 0 {
                all.push(id);
            }
            if min_tls_ver >= 12 {
                tls_1_2.push(id);
            }
            entries.push(CipherEntry {
                id,
                openssl,
                iana,
                key_ex,
                auth,
                encryption: enc,
                bits,
                suite: cipher,
            });
        }

        let default: Vec<_> = default_cipher_suites
            .iter()
            .map(|c| u16::from(c.suite()))
            .collect();

        Self {
            complement_of_default: all_cipher_suites
                .iter()
                .filter(|c| !default.contains(&c.suite().into()))
                .map(|c| u16::from(c.suite()))
                .collect(),
            complement_of_all: all_cipher_suites
                .iter()
                .filter(|c| !all.contains(&c.suite().into()))
                .map(|c| u16::from(c.suite()))
                .collect(),

            entries,
            default,
            all,
            tls_1_2,
        }
    }
}

pub(super) fn default_cipher_string() -> String {
    CIPHER_MAPPINGS.default_cipher_string()
}

pub(super) fn restore_default_tls13(
    mut selected: Vec<SupportedCipherSuite>,
    defaults: &[SupportedCipherSuite],
) -> Vec<SupportedCipherSuite> {
    selected.retain(|suite| suite.tls13().is_none());
    let tls13 = defaults
        .iter()
        .filter(|suite| suite.tls13().is_some())
        .copied();
    selected.splice(..0, tls13);
    selected
}

#[cfg(test)]
mod tests {
    use core::hint::black_box;
    use std::sync::Once;

    use rustls::crypto::aws_lc_rs;

    use super::*;

    fn install_test_crypto_provider() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let ext = CryptoExt {
                all_cipher_suites: Some(aws_lc_rs::ALL_CIPHER_SUITES),
                default_cipher_suites: Some(aws_lc_rs::DEFAULT_CIPHER_SUITES),
                all_kx_groups: Some(aws_lc_rs::ALL_KX_GROUPS),
                any_supported_key: Some(aws_lc_rs::sign::any_supported_type),
                ticketer: aws_lc_rs::Ticketer::new,
            };
            let _ = CryptoExt::set_provider(aws_lc_rs::default_provider(), ext);
        })
    }

    fn cipher_names(s: &str) -> Vec<&'static str> {
        install_test_crypto_provider();

        let (suites, suite_b) = CipherList::parse_to_rustls(s).unwrap();
        assert!(suite_b.is_none());
        suites
            .iter()
            .map(|suite| CIPHER_MAPPINGS.entry(suite.suite().into()).openssl)
            .collect()
    }

    /// The metadata table has to name every suite the provider offers, and it
    /// panics on one it does not, so building it is the assertion.
    #[test]
    fn every_rustls_cipher_is_known() {
        install_test_crypto_provider();
        for suite in CryptoExt::get_ext().all_ciphers_or_default() {
            let _ = black_box(describe(suite));
        }

        assert!(
            CryptoExt::get_ext()
                .all_kx_or_default()
                .iter()
                .all(|group| !kx_group_openssl_names(group.name()).is_empty())
        );
    }

    #[test]
    fn default_and_names() {
        install_test_crypto_provider();

        let default = CryptoExt::get_ext()
            .default_ciphers_or_provider()
            .iter()
            .map(|suite| suite.suite())
            .collect::<Vec<_>>();
        let (suites, suite_b) = CipherList::parse_to_rustls("DEFAULT").unwrap();

        assert!(suite_b.is_none());
        assert_eq!(
            suites.iter().map(|suite| suite.suite()).collect::<Vec<_>>(),
            default
        );
        assert_eq!(default_cipher_string(), cipher_names("DEFAULT").join(":"));
        // Either spelling names the same suite, and a term is a conjunction.
        assert_eq!(
            cipher_names("TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256, ECDHE-ECDSA-AES128-GCM-SHA256"),
            [
                "ECDHE-RSA-AES128-GCM-SHA256",
                "ECDHE-ECDSA-AES128-GCM-SHA256",
            ]
        );
        assert_eq!(
            cipher_names("AES128+aECDSA"),
            ["ECDHE-ECDSA-AES128-GCM-SHA256"]
        );

        let (selected, _) = CipherList::parse_to_rustls("ALL").unwrap();
        let defaults = &[
            rustls::crypto::aws_lc_rs::cipher_suite::TLS13_AES_256_GCM_SHA384,
            rustls::crypto::aws_lc_rs::cipher_suite::TLS13_AES_128_GCM_SHA256,
        ];
        let selected = restore_default_tls13(selected, defaults);
        assert_eq!(
            selected
                .iter()
                .filter(|suite| suite.tls13().is_some())
                .map(|suite| suite.suite())
                .collect::<Vec<_>>(),
            [
                CipherSuite::TLS13_AES_256_GCM_SHA384,
                CipherSuite::TLS13_AES_128_GCM_SHA256,
            ]
        );
    }

    #[test]
    fn keyword_families() {
        install_test_crypto_provider();

        // `AES` names either key length, while `ECDH` names the ephemeral
        // agreement every suite here uses, as `kECDHE` does.
        assert_eq!(
            cipher_names("AES"),
            [
                "TLS_AES_256_GCM_SHA384",
                "TLS_AES_128_GCM_SHA256",
                "ECDHE-ECDSA-AES256-GCM-SHA384",
                "ECDHE-ECDSA-AES128-GCM-SHA256",
                "ECDHE-RSA-AES256-GCM-SHA384",
                "ECDHE-RSA-AES128-GCM-SHA256",
            ]
        );
        assert_eq!(cipher_names("ECDH"), cipher_names("ALL"));
        assert_eq!(cipher_names("kECDHE"), cipher_names("ALL"));

        // A selection keeps the order the provider offers, whatever order the
        // maps it was collected out of iterate in.
        assert_eq!(
            cipher_names("aRSA"),
            [
                "TLS_AES_256_GCM_SHA384",
                "TLS_AES_128_GCM_SHA256",
                "TLS_CHACHA20_POLY1305_SHA256",
                "ECDHE-RSA-AES256-GCM-SHA384",
                "ECDHE-RSA-AES128-GCM-SHA256",
                "ECDHE-RSA-CHACHA20-POLY1305",
            ]
        );

        // Nothing here is of a weaker grade, older than TLS 1.2, or carries a
        // MAC of its own.
        for nothing in ["MEDIUM", "LOW", "TLSv1.0", "SSLv3", "SHA256", "CBC"] {
            assert_eq!(cipher_names(nothing), Vec::<&str>::new(), "{nothing}");
        }

        // An empty first half of a conjunction stays empty rather than making
        // its second half act like a fresh selection.
        for nothing in ["MEDIUM+AES", "TLSv1.0+AES", "NO-SUCH-CIPHER+AES"] {
            assert_eq!(cipher_names(nothing), Vec::<&str>::new(), "{nothing}");
        }
    }

    #[test]
    fn deletes_and_moves() {
        install_test_crypto_provider();

        // `+` moves an existing cipher to the end rather than adding one.
        assert_eq!(
            cipher_names(
                "ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:+ECDHE-ECDSA-AES128-GCM-SHA256"
            ),
            [
                "ECDHE-RSA-AES128-GCM-SHA256",
                "ECDHE-ECDSA-AES128-GCM-SHA256",
            ]
        );
        // `-` lets a later term restore the cipher; `!` does not.
        assert_eq!(
            cipher_names(
                "ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES128-GCM-SHA256:-ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES128-GCM-SHA256:!ECDHE-RSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256"
            ),
            ["ECDHE-ECDSA-AES128-GCM-SHA256"]
        );
    }

    #[test]
    fn strength_and_security_level() {
        install_test_crypto_provider();

        assert_eq!(
            cipher_names("ECDHE-RSA-AES128-GCM-SHA256:ECDHE-RSA-AES256-GCM-SHA384:@STRENGTH"),
            ["ECDHE-RSA-AES256-GCM-SHA384", "ECDHE-RSA-AES128-GCM-SHA256"]
        );
        // The level applies to what is already selected and to what follows.
        assert_eq!(
            cipher_names("ECDHE-RSA-AES128-GCM-SHA256:@SECLEVEL=4:ECDHE-RSA-AES256-GCM-SHA384"),
            ["ECDHE-RSA-AES256-GCM-SHA384"]
        );
    }

    #[test]
    fn suite_b() {
        install_test_crypto_provider();

        let (suites, suite_b) = CipherList::parse_to_rustls("SUITEB128:ALL").unwrap();

        // Anything after a SUITEB term is ignored, and the term carries the
        // key exchange groups RFC 6460 pins.
        assert_eq!(
            suites.iter().map(|suite| suite.suite()).collect::<Vec<_>>(),
            [
                CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
                CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
            ]
        );
        assert!(suite_b.is_some());
        assert!(CipherList::parse_to_rustls("ALL:SUITEB128").is_err());

        // A provider that omits one of the hardcoded Suite B suites must get
        // an error rather than reaching the invariant-enforcing lookup.
        assert!(CIPHER_MAPPINGS.validate_suite_b(&[u16::MAX]).is_err());
    }

    #[test]
    fn rejects_what_openssl_rejects() {
        install_test_crypto_provider();

        assert!(CipherList::parse_to_rustls("ALL:DEFAULT").is_err());
        assert!(CipherList::parse_to_rustls("ALL:@SECLEVEL=6").is_err());
        assert!(CipherList::parse_to_rustls("PROFILE=SYSTEM").is_err());
        assert!(CipherList::parse_to_rustls(";").is_err());
        assert!(CipherList::parse_to_rustls("").is_err());
        assert!(CipherList::parse_to_rustls("AES+").is_err());
        assert!(CipherList::parse_to_rustls("ALL++AES").is_err());
    }
}
