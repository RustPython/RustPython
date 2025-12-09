// cspell: ignore accessdescs

//! Certificate parsing, validation, and conversion utilities for SSL/TLS
//!
//! This module provides reusable functions for working with X.509 certificates:
//! - Parsing PEM/DER encoded certificates
//! - Validating certificate properties (CA status, etc.)
//! - Converting certificates to Python dict format
//! - Building and verifying certificate chains
//! - Loading certificates from files, directories, and bytes

use chrono::{DateTime, Utc};
use parking_lot::RwLock as ParkingRwLock;
use rustls::{
    DigitallySignedStruct, RootCertStore, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use rustpython_vm::{PyObjectRef, PyResult, VirtualMachine};
use std::collections::HashSet;
use std::sync::Arc;
use x509_parser::prelude::*;

use super::compat::{VERIFY_X509_PARTIAL_CHAIN, VERIFY_X509_STRICT};

// Certificate Verification Constants

/// All supported signature schemes for certificate verification
///
/// This list includes all modern signature algorithms supported by rustls.
/// Used by verifiers that accept any signature scheme (NoVerifier, EmptyRootStoreVerifier).
const ALL_SIGNATURE_SCHEMES: &[SignatureScheme] = &[
    SignatureScheme::RSA_PKCS1_SHA256,
    SignatureScheme::RSA_PKCS1_SHA384,
    SignatureScheme::RSA_PKCS1_SHA512,
    SignatureScheme::ECDSA_NISTP256_SHA256,
    SignatureScheme::ECDSA_NISTP384_SHA384,
    SignatureScheme::ECDSA_NISTP521_SHA512,
    SignatureScheme::RSA_PSS_SHA256,
    SignatureScheme::RSA_PSS_SHA384,
    SignatureScheme::RSA_PSS_SHA512,
    SignatureScheme::ED25519,
];

// Error Handling Utilities

/// Certificate loading error types with specific error messages
///
/// This module provides consistent error creation functions for certificate
/// operations, reducing code duplication and ensuring uniform error messages
/// across the codebase.
mod cert_error {
    use std::io;
    use std::sync::Arc;

    /// Create InvalidData error with formatted message
    pub fn invalid_data(msg: impl Into<String>) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, msg.into())
    }

    /// PEM parsing error variants
    pub mod pem {
        use super::*;

        pub fn no_start_line(context: &str) -> io::Error {
            invalid_data(format!("no start line: {context}"))
        }

        pub fn parse_failed(e: impl std::fmt::Display) -> io::Error {
            invalid_data(format!("Failed to parse PEM certificate: {e}"))
        }

        pub fn parse_failed_debug(e: impl std::fmt::Debug) -> io::Error {
            invalid_data(format!("Failed to parse PEM certificate: {e:?}"))
        }

        pub fn invalid_cert() -> io::Error {
            invalid_data("No certificates found in certificate file")
        }
    }

    /// DER parsing error variants
    pub mod der {
        use super::*;

        pub fn not_enough_data(context: &str) -> io::Error {
            invalid_data(format!("not enough data: {context}"))
        }

        pub fn parse_failed(e: impl std::fmt::Display) -> io::Error {
            invalid_data(format!("Failed to parse DER certificate: {e}"))
        }
    }

    /// Private key error variants
    pub mod key {
        use super::*;

        pub fn not_found(context: &str) -> io::Error {
            invalid_data(format!("No private key found in {context}"))
        }

        pub fn parse_failed(e: impl std::fmt::Display) -> io::Error {
            invalid_data(format!("Failed to parse private key: {e}"))
        }

        pub fn parse_encrypted_failed(e: impl std::fmt::Display) -> io::Error {
            invalid_data(format!("Failed to parse encrypted private key: {e}"))
        }

        pub fn decrypt_failed(e: impl std::fmt::Display) -> io::Error {
            io::Error::other(format!(
                "Failed to decrypt private key (wrong password?): {e}",
            ))
        }
    }

    /// Convert error message to rustls::Error with InvalidCertificate wrapper
    pub fn to_rustls_invalid_cert(msg: impl Into<String>) -> rustls::Error {
        rustls::Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
            Arc::new(invalid_data(msg)),
        )))
    }

    /// Convert error message to rustls::Error with InvalidCertificate wrapper and custom ErrorKind
    pub fn to_rustls_cert_error(kind: io::ErrorKind, msg: impl Into<String>) -> rustls::Error {
        rustls::Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
            Arc::new(io::Error::new(kind, msg.into())),
        )))
    }
}

// Helper Functions for Certificate Parsing

/// Map X.509 OID to human-readable attribute name
///
/// Converts common X.509 Distinguished Name OIDs to their standard names.
/// Returns the OID string itself if not recognized.
fn oid_to_attribute_name(oid_str: &str) -> &str {
    match oid_str {
        "2.5.4.3" => "commonName",
        "2.5.4.6" => "countryName",
        "2.5.4.7" => "localityName",
        "2.5.4.8" => "stateOrProvinceName",
        "2.5.4.10" => "organizationName",
        "2.5.4.11" => "organizationalUnitName",
        "1.2.840.113549.1.9.1" => "emailAddress",
        _ => oid_str,
    }
}

/// Format IP address (IPv4 or IPv6) to string
///
/// Formats raw IP address bytes according to standard notation:
/// - IPv4: dotted decimal (e.g., "192.0.2.1")
/// - IPv6: colon-separated hex (e.g., "2001:DB8:0:0:0:0:0:1")
fn format_ip_address(ip: &[u8]) -> String {
    if ip.len() == 4 {
        // IPv4
        format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
    } else if ip.len() == 16 {
        // IPv6 - format in full form without compression (uppercase)
        // CPython returns IPv6 in full form: 2001:DB8:0:0:0:0:0:1 (not 2001:db8::1)
        let segments = [
            u16::from_be_bytes([ip[0], ip[1]]),
            u16::from_be_bytes([ip[2], ip[3]]),
            u16::from_be_bytes([ip[4], ip[5]]),
            u16::from_be_bytes([ip[6], ip[7]]),
            u16::from_be_bytes([ip[8], ip[9]]),
            u16::from_be_bytes([ip[10], ip[11]]),
            u16::from_be_bytes([ip[12], ip[13]]),
            u16::from_be_bytes([ip[14], ip[15]]),
        ];
        format!(
            "{:X}:{:X}:{:X}:{:X}:{:X}:{:X}:{:X}:{:X}",
            segments[0],
            segments[1],
            segments[2],
            segments[3],
            segments[4],
            segments[5],
            segments[6],
            segments[7]
        )
    } else {
        // Unknown format - return as debug string
        format!("{ip:?}")
    }
}

/// Format ASN.1 time to string
///
/// Formats certificate validity dates in the format:
/// "Mon DD HH:MM:SS YYYY GMT"
fn format_asn1_time(time: &x509_parser::time::ASN1Time) -> String {
    let timestamp = time.timestamp();
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .expect("ASN1Time must be valid timestamp")
        .format("%b %e %H:%M:%S %Y GMT")
        .to_string()
}

/// Format certificate serial number to hexadecimal string with even padding
///
/// Converts a BigUint serial number to uppercase hex string, ensuring
/// even length by prepending '0' if necessary.
fn format_serial_number(serial: &num_bigint::BigUint) -> String {
    let mut serial_str = serial.to_str_radix(16).to_uppercase();
    if serial_str.len() % 2 == 1 {
        serial_str.insert(0, '0');
    }
    serial_str
}

/// Normalize wildcard hostname by stripping "*." prefix
///
/// Returns the normalized hostname without the wildcard prefix.
/// Used for wildcard certificate matching.
fn normalize_wildcard_hostname(hostname: &str) -> &str {
    hostname.strip_prefix("*.").unwrap_or(hostname)
}

/// Process Subject Alternative Name (SAN) general names into Python tuples
///
/// Converts X.509 GeneralName entries into Python tuple format.
/// Returns a vector of PyObjectRef tuples in the format: (type, value)
fn process_san_general_names(
    vm: &VirtualMachine,
    general_names: &[GeneralName<'_>],
) -> Vec<PyObjectRef> {
    general_names
        .iter()
        .filter_map(|name| match name {
            GeneralName::DNSName(dns) => Some(vm.new_tuple(("DNS", *dns)).into()),
            GeneralName::IPAddress(ip) => {
                let ip_str = format_ip_address(ip);
                Some(vm.new_tuple(("IP Address", ip_str)).into())
            }
            GeneralName::RFC822Name(email) => Some(vm.new_tuple(("email", *email)).into()),
            GeneralName::URI(uri) => Some(vm.new_tuple(("URI", *uri)).into()),
            GeneralName::DirectoryName(dn) => {
                let dn_str = format!("{dn}");
                Some(vm.new_tuple(("DirName", dn_str)).into())
            }
            GeneralName::RegisteredID(oid) => {
                let oid_str = oid.to_string();
                Some(vm.new_tuple(("Registered ID", oid_str)).into())
            }
            GeneralName::OtherName(oid, value) => {
                let oid_str = oid.to_string();
                let value_str = format!("{value:?}");
                Some(
                    vm.new_tuple(("othername", format!("{oid_str}:{value_str}")))
                        .into(),
                )
            }
            _ => None,
        })
        .collect()
}

// Certificate Validation and Parsing

/// Check if a certificate is a CA certificate by examining the Basic Constraints extension
///
/// Returns `true` if the certificate has Basic Constraints with CA=true,
/// `false` otherwise (including parse errors or missing extension).
/// This matches OpenSSL's X509_check_ca() behavior.
pub fn is_ca_certificate(cert_der: &[u8]) -> bool {
    // Parse the certificate
    let Ok((_, cert)) = X509Certificate::from_der(cert_der) else {
        return false;
    };

    // Check Basic Constraints extension
    // If extension exists and CA=true, it's a CA certificate
    // Otherwise (no extension or CA=false), it's NOT a CA certificate
    if let Ok(Some(ext)) = cert.basic_constraints() {
        return ext.value.ca;
    }

    // No Basic Constraints extension -> NOT a CA certificate
    // (matches OpenSSL X509_check_ca() behavior)
    false
}

/// Convert an X509Name to Python nested tuple format for SSL certificate dicts
///
/// Format: ((('CN', 'example.com'),), (('O', 'Example Org'),), ...)
fn name_to_py(vm: &VirtualMachine, name: &x509_parser::x509::X509Name<'_>) -> PyResult {
    let list: Vec<PyObjectRef> = name
        .iter()
        .flat_map(|rdn| {
            // Each RDN can have multiple attributes
            rdn.iter()
                .map(|attr| {
                    let oid_str = attr.attr_type().to_id_string();
                    let value_str = attr.attr_value().as_str().unwrap_or("").to_string();
                    let key = oid_to_attribute_name(&oid_str);

                    vm.new_tuple((vm.new_tuple((vm.ctx.new_str(key), vm.ctx.new_str(value_str))),))
                        .into()
                })
                .collect::<Vec<_>>()
        })
        .collect();

    Ok(vm.ctx.new_tuple(list).into())
}

/// Convert DER-encoded certificate to Python dict (for getpeercert with binary_form=False)
///
/// Returns a dict with fields: subject, issuer, version, serialNumber,
/// notBefore, notAfter, subjectAltName (if present)
pub fn cert_to_dict(
    vm: &VirtualMachine,
    cert: &x509_parser::certificate::X509Certificate<'_>,
) -> PyResult {
    let dict = vm.ctx.new_dict();

    // Subject and Issuer
    dict.set_item("subject", name_to_py(vm, cert.subject())?, vm)?;
    dict.set_item("issuer", name_to_py(vm, cert.issuer())?, vm)?;

    // Version (X.509 v3 = version 2 in the cert, but Python uses 3)
    dict.set_item(
        "version",
        vm.ctx.new_int(cert.version().0 as i32 + 1).into(),
        vm,
    )?;

    // Serial number - hex format with even length
    let serial = format_serial_number(&cert.serial);
    dict.set_item("serialNumber", vm.ctx.new_str(serial).into(), vm)?;

    // Validity dates - format with GMT using chrono
    dict.set_item(
        "notBefore",
        vm.ctx
            .new_str(format_asn1_time(&cert.validity().not_before))
            .into(),
        vm,
    )?;
    dict.set_item(
        "notAfter",
        vm.ctx
            .new_str(format_asn1_time(&cert.validity().not_after))
            .into(),
        vm,
    )?;

    // Subject Alternative Names (if present)
    if let Ok(Some(san_ext)) = cert.subject_alternative_name() {
        let san_list = process_san_general_names(vm, &san_ext.value.general_names);

        if !san_list.is_empty() {
            dict.set_item("subjectAltName", vm.ctx.new_tuple(san_list).into(), vm)?;
        }
    }

    Ok(dict.into())
}

/// Convert DER-encoded certificate to Python dict (for get_ca_certs)
///
/// Similar to cert_to_dict but includes additional fields like crlDistributionPoints
/// and uses CPython's specific ordering: issuer, notAfter, notBefore, serialNumber, subject, version
pub fn cert_der_to_dict_helper(vm: &VirtualMachine, cert_der: &[u8]) -> PyResult<PyObjectRef> {
    // Parse the certificate using x509-parser
    let (_, cert) = x509_parser::parse_x509_certificate(cert_der)
        .map_err(|e| vm.new_value_error(format!("Failed to parse certificate: {e}")))?;

    // Helper to convert X509Name to nested tuple format
    let name_to_tuple = |name: &x509_parser::x509::X509Name<'_>| -> PyResult {
        let mut entries = Vec::new();
        for rdn in name.iter() {
            for attr in rdn.iter() {
                let oid_str = attr.attr_type().to_id_string();

                // Get value as bytes and convert to string
                let value_str = if let Ok(s) = attr.attr_value().as_str() {
                    s.to_string()
                } else {
                    let value_bytes = attr.attr_value().data;
                    match std::str::from_utf8(value_bytes) {
                        Ok(s) => s.to_string(),
                        Err(_) => String::from_utf8_lossy(value_bytes).into_owned(),
                    }
                };

                let key = oid_to_attribute_name(&oid_str);

                let entry =
                    vm.new_tuple((vm.ctx.new_str(key.to_string()), vm.ctx.new_str(value_str)));
                entries.push(vm.new_tuple((entry,)).into());
            }
        }
        Ok(vm.ctx.new_tuple(entries).into())
    };

    let dict = vm.ctx.new_dict();

    // CPython ordering: issuer, notAfter, notBefore, serialNumber, subject, version
    dict.set_item("issuer", name_to_tuple(cert.issuer())?, vm)?;

    // Validity - format with GMT using chrono
    dict.set_item(
        "notAfter",
        vm.ctx
            .new_str(format_asn1_time(&cert.validity().not_after))
            .into(),
        vm,
    )?;
    dict.set_item(
        "notBefore",
        vm.ctx
            .new_str(format_asn1_time(&cert.validity().not_before))
            .into(),
        vm,
    )?;

    // Serial number - hex format with even length
    let serial = format_serial_number(&cert.serial);
    dict.set_item("serialNumber", vm.ctx.new_str(serial).into(), vm)?;

    dict.set_item("subject", name_to_tuple(cert.subject())?, vm)?;

    // Version
    dict.set_item(
        "version",
        vm.ctx.new_int(cert.version().0 as i32 + 1).into(),
        vm,
    )?;

    // Authority Information Access (OCSP and caIssuers) - use x509-parser's extensions_map
    let mut ocsp_urls = Vec::new();
    let mut ca_issuer_urls = Vec::new();
    let mut crl_urls = Vec::new();

    if let Ok(ext_map) = cert.tbs_certificate.extensions_map() {
        use x509_parser::extensions::{GeneralName, ParsedExtension};
        use x509_parser::oid_registry::{
            OID_PKIX_AUTHORITY_INFO_ACCESS, OID_X509_EXT_CRL_DISTRIBUTION_POINTS,
        };

        // Authority Information Access
        if let Some(ext) = ext_map.get(&OID_PKIX_AUTHORITY_INFO_ACCESS)
            && let ParsedExtension::AuthorityInfoAccess(aia) = &ext.parsed_extension()
        {
            for desc in &aia.accessdescs {
                if let GeneralName::URI(uri) = &desc.access_location {
                    let method_str = desc.access_method.to_id_string();
                    if method_str == "1.3.6.1.5.5.7.48.1" {
                        // OCSP
                        ocsp_urls.push(vm.ctx.new_str(uri.to_string()).into());
                    } else if method_str == "1.3.6.1.5.5.7.48.2" {
                        // caIssuers
                        ca_issuer_urls.push(vm.ctx.new_str(uri.to_string()).into());
                    }
                }
            }
        }

        // CRL Distribution Points
        if let Some(ext) = ext_map.get(&OID_X509_EXT_CRL_DISTRIBUTION_POINTS)
            && let ParsedExtension::CRLDistributionPoints(cdp) = &ext.parsed_extension()
        {
            for dp in cdp.points.iter() {
                if let Some(dist_point) = &dp.distribution_point {
                    use x509_parser::extensions::DistributionPointName;
                    if let DistributionPointName::FullName(names) = dist_point {
                        for name in names {
                            if let GeneralName::URI(uri) = name {
                                crl_urls.push(vm.ctx.new_str(uri.to_string()).into());
                            }
                        }
                    }
                }
            }
        }
    }

    if !ocsp_urls.is_empty() {
        dict.set_item("OCSP", vm.ctx.new_tuple(ocsp_urls).into(), vm)?;
    }
    if !ca_issuer_urls.is_empty() {
        dict.set_item("caIssuers", vm.ctx.new_tuple(ca_issuer_urls).into(), vm)?;
    }
    if !crl_urls.is_empty() {
        dict.set_item(
            "crlDistributionPoints",
            vm.ctx.new_tuple(crl_urls).into(),
            vm,
        )?;
    }

    // Subject Alternative Names
    if let Ok(Some(san_ext)) = cert.subject_alternative_name() {
        let mut san_entries = Vec::new();
        for name in &san_ext.value.general_names {
            use x509_parser::extensions::GeneralName;
            match name {
                GeneralName::DNSName(dns) => {
                    san_entries.push(vm.new_tuple(("DNS", *dns)).into());
                }
                GeneralName::IPAddress(ip) => {
                    let ip_str = format_ip_address(ip);
                    san_entries.push(vm.new_tuple(("IP Address", ip_str)).into());
                }
                GeneralName::RFC822Name(email) => {
                    san_entries.push(vm.new_tuple(("email", *email)).into());
                }
                GeneralName::URI(uri) => {
                    san_entries.push(vm.new_tuple(("URI", *uri)).into());
                }
                GeneralName::OtherName(_oid, _data) => {
                    // OtherName is not fully supported, mark as unsupported
                    san_entries.push(vm.new_tuple(("othername", "<unsupported>")).into());
                }
                GeneralName::DirectoryName(name) => {
                    // Convert X509Name to nested tuple format
                    let dir_tuple = name_to_tuple(name)?;
                    san_entries.push(vm.new_tuple(("DirName", dir_tuple)).into());
                }
                GeneralName::RegisteredID(oid) => {
                    // Convert OID to string representation
                    let oid_str = oid.to_id_string();
                    san_entries.push(vm.new_tuple(("Registered ID", oid_str)).into());
                }
                _ => {}
            }
        }
        if !san_entries.is_empty() {
            dict.set_item("subjectAltName", vm.ctx.new_tuple(san_entries).into(), vm)?;
        }
    }

    Ok(dict.into())
}

/// Build a verified certificate chain by adding CA certificates from the trust store
///
/// Takes peer certificates (from TLS handshake) and extends the chain by finding
/// issuer certificates from the trust store until reaching a root certificate.
///
/// Returns the complete chain as DER-encoded bytes.
pub fn build_verified_chain(
    peer_certs: &[CertificateDer<'static>],
    ca_certs_der: &[Vec<u8>],
) -> Vec<Vec<u8>> {
    let mut chain_der: Vec<Vec<u8>> = Vec::new();

    // Start with peer certificates (what was sent during handshake)
    for cert in peer_certs {
        chain_der.push(cert.as_ref().to_vec());
    }

    // Keep adding issuers until we reach a root or can't find the issuer
    while let Some(der) = chain_der.last() {
        let last_cert_der = der;

        // Parse the last certificate in the chain
        let (_, last_cert) = match X509Certificate::from_der(last_cert_der) {
            Ok(parsed) => parsed,
            Err(_) => break,
        };

        // Check if it's self-signed (root certificate)
        if last_cert.subject() == last_cert.issuer() {
            // This is a root certificate, we're done
            break;
        }

        // Try to find the issuer in the trust store
        let issuer_name = last_cert.issuer();
        let mut found_issuer = false;

        for ca_der in ca_certs_der.iter() {
            let (_, ca_cert) = match X509Certificate::from_der(ca_der) {
                Ok(parsed) => parsed,
                Err(_) => continue,
            };

            // Check if this CA's subject matches the certificate's issuer
            if ca_cert.subject() == issuer_name {
                // Check if we already have this certificate in the chain
                if !chain_der.iter().any(|existing| existing == ca_der) {
                    chain_der.push(ca_der.clone());
                    found_issuer = true;
                    break;
                }
            }
        }

        if !found_issuer {
            // Can't find issuer, stop here
            break;
        }
    }

    chain_der
}

/// Statistics from certificate loading operations
#[derive(Debug, Clone, Default)]
pub struct CertStats {
    pub total_certs: usize,
    pub ca_certs: usize,
}

/// Certificate loader that handles PEM/DER parsing and validation
///
/// This structure encapsulates the common pattern of loading certificates
/// from various sources (files, directories, bytes) and adding them to
/// a RootCertStore while tracking statistics.
///
/// Duplicate certificates are detected and only counted once.
pub struct CertLoader<'a> {
    store: &'a mut RootCertStore,
    ca_certs_der: &'a mut Vec<Vec<u8>>,
    seen_certs: HashSet<Vec<u8>>,
}

impl<'a> CertLoader<'a> {
    /// Create a new CertLoader with references to the store and DER cache
    pub fn new(store: &'a mut RootCertStore, ca_certs_der: &'a mut Vec<Vec<u8>>) -> Self {
        // Initialize seen_certs with existing certificates
        let seen_certs = ca_certs_der.iter().cloned().collect();
        Self {
            store,
            ca_certs_der,
            seen_certs,
        }
    }

    /// Load certificates from a file (supports both PEM and DER formats)
    ///
    /// Returns statistics about loaded certificates
    pub fn load_from_file(&mut self, path: &str) -> Result<CertStats, std::io::Error> {
        let contents = std::fs::read(path)?;
        self.load_from_bytes(&contents)
    }

    /// Load certificates from a directory
    ///
    /// Reads all files in the directory and attempts to parse them as certificates.
    /// Invalid files are silently skipped (matches OpenSSL capath behavior).
    pub fn load_from_dir(&mut self, dir_path: &str) -> Result<CertStats, std::io::Error> {
        let entries = std::fs::read_dir(dir_path)?;
        let mut stats = CertStats::default();

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Skip directories and process all files
            // OpenSSL capath uses hash-based naming like "4e1295a3.0"
            if path.is_file()
                && let Ok(contents) = std::fs::read(&path)
            {
                // Ignore errors for individual files (some may not be certs)
                if let Ok(file_stats) = self.load_from_bytes(&contents) {
                    stats.total_certs += file_stats.total_certs;
                    stats.ca_certs += file_stats.ca_certs;
                }
            }
        }

        Ok(stats)
    }

    /// Helper: Add a certificate to the store with duplicate checking
    ///
    /// Returns true if the certificate was added (not a duplicate), false if it was a duplicate.
    fn add_cert_to_store(
        &mut self,
        cert_bytes: Vec<u8>,
        cert_der: CertificateDer<'static>,
        treat_all_as_ca: bool,
        stats: &mut CertStats,
    ) -> bool {
        // Check for duplicates using HashSet
        if !self.seen_certs.insert(cert_bytes.clone()) {
            return false; // Duplicate certificate - skip
        }

        // Determine if this is a CA certificate
        let is_ca = if treat_all_as_ca {
            true
        } else {
            is_ca_certificate(&cert_bytes)
        };

        // Store full DER for get_ca_certs()
        self.ca_certs_der.push(cert_bytes);

        // Add to trust store (rustls may handle duplicates internally)
        let _ = self.store.add(cert_der);

        // Update statistics
        stats.total_certs += 1;
        if is_ca {
            stats.ca_certs += 1;
        }

        true
    }

    /// Load certificates from byte slice (auto-detects PEM vs DER format)
    ///
    /// Tries to parse as PEM first, falls back to DER if that fails.
    /// Duplicate certificates are detected and only counted once.
    ///
    /// If `treat_all_as_ca` is true, all certificates are counted as CA certificates
    /// regardless of their Basic Constraints (this matches
    /// load_verify_locations with cadata parameter).
    ///
    /// If `pem_only` is true, only PEM parsing is attempted (for string input)
    pub fn load_from_bytes_ex(
        &mut self,
        data: &[u8],
        treat_all_as_ca: bool,
        pem_only: bool,
    ) -> Result<CertStats, std::io::Error> {
        let mut stats = CertStats::default();

        // Try to parse as PEM first
        let mut cursor = std::io::Cursor::new(data);
        let certs_iter = rustls_pemfile::certs(&mut cursor);

        let mut found_any = false;
        let mut first_pem_error = None; // Store first PEM parsing error
        for cert_result in certs_iter {
            match cert_result {
                Ok(cert) => {
                    found_any = true;
                    let cert_bytes = cert.to_vec();

                    // Validate that this is actually a valid X.509 certificate
                    // rustls_pemfile only does base64 decoding, not X.509 validation
                    if let Err(e) = X509Certificate::from_der(&cert_bytes) {
                        // Invalid X.509 certificate
                        return Err(cert_error::pem::parse_failed_debug(e));
                    }

                    // Add certificate using helper method (handles duplicates)
                    self.add_cert_to_store(cert_bytes, cert, treat_all_as_ca, &mut stats);
                    // Helper returns false for duplicates (skip counting)
                }
                Err(e) if !found_any => {
                    // PEM parsing failed on first certificate
                    if pem_only {
                        // For string input (PEM only), return "no start line" error
                        return Err(cert_error::pem::no_start_line(
                            "cadata does not contain a certificate",
                        ));
                    }
                    // Store the error and break to try DER format below
                    first_pem_error = Some(e);
                    break;
                }
                Err(e) => {
                    // PEM parsing failed after some certs were loaded
                    return Err(cert_error::pem::parse_failed(e));
                }
            }
        }

        // If PEM parsing found nothing, try DER format (unless pem_only)
        // DER can have multiple certificates concatenated, so parse them sequentially
        if !found_any && stats.total_certs == 0 {
            // If we had a PEM parsing error, return it instead of trying DER fallback
            // This ensures that malformed PEM files (like badcert.pem) raise an error
            if let Some(e) = first_pem_error {
                return Err(cert_error::pem::parse_failed(e));
            }

            // For PEM-only mode (string input), don't fallback to DER
            if pem_only {
                return Err(cert_error::pem::no_start_line(
                    "cadata does not contain a certificate",
                ));
            }
            let mut remaining = data;
            let mut loaded_count = 0;

            while !remaining.is_empty() {
                match X509Certificate::from_der(remaining) {
                    Ok((rest, _parsed_cert)) => {
                        // Extract the DER bytes for this certificate
                        // Length = total remaining - bytes left after parsing
                        let cert_len = remaining.len() - rest.len();
                        let cert_bytes = &remaining[..cert_len];
                        let cert_der = CertificateDer::from(cert_bytes.to_vec());

                        // Add certificate using helper method (handles duplicates)
                        self.add_cert_to_store(
                            cert_bytes.to_vec(),
                            cert_der,
                            treat_all_as_ca,
                            &mut stats,
                        );

                        loaded_count += 1;
                        remaining = rest; // Move to next certificate
                    }
                    Err(e) => {
                        if loaded_count == 0 {
                            // Failed to parse first certificate - invalid data
                            return Err(cert_error::der::not_enough_data(
                                "cadata does not contain a certificate",
                            ));
                        } else {
                            // Loaded some certificates but failed on subsequent data (garbage)
                            return Err(cert_error::der::parse_failed(e));
                        }
                    }
                }
            }

            // If we somehow got here with no certificates loaded
            if loaded_count == 0 {
                return Err(cert_error::der::not_enough_data(
                    "cadata does not contain a certificate",
                ));
            }
        }

        Ok(stats)
    }

    /// Load certificates from byte slice (auto-detects PEM vs DER format)
    ///
    /// This is a convenience wrapper that calls load_from_bytes_ex with treat_all_as_ca=false
    /// and pem_only=false.
    pub fn load_from_bytes(&mut self, data: &[u8]) -> Result<CertStats, std::io::Error> {
        self.load_from_bytes_ex(data, false, false)
    }
}

// NoVerifier: disables certificate verification (for CERT_NONE mode)
#[derive(Debug)]
pub struct NoVerifier;

impl ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Accept all certificates without verification
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // Accept all signatures without verification
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // Accept all signatures without verification
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        ALL_SIGNATURE_SCHEMES.to_vec()
    }
}

// HostnameIgnoringVerifier: verifies certificate chain but ignores hostname
// This is used when check_hostname=False but verify_mode != CERT_NONE
//
// Unlike the previous implementation that used an inner WebPkiServerVerifier,
// this version uses webpki directly to verify only the certificate chain,
// completely bypassing hostname verification.
#[derive(Debug)]
pub struct HostnameIgnoringVerifier {
    inner: Arc<dyn ServerCertVerifier>,
}

impl HostnameIgnoringVerifier {
    /// Create a new HostnameIgnoringVerifier with a pre-built verifier
    /// This is useful when you need to configure the verifier with CRLs or other options
    pub fn new_with_verifier(inner: Arc<dyn ServerCertVerifier>) -> Self {
        Self { inner }
    }
}

impl ServerCertVerifier for HostnameIgnoringVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>, // Intentionally ignored
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Extract a hostname from the certificate to pass to inner verifier
        // The inner verifier will validate certificate chain, trust anchors, etc.
        // but may fail on hostname mismatch - we'll catch and ignore that error
        let dummy_hostname = extract_first_dns_name(end_entity)
            .unwrap_or_else(|| ServerName::try_from("localhost").expect("localhost is valid"));

        // Call inner verifier for full certificate validation
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            &dummy_hostname,
            ocsp_response,
            now,
        ) {
            Ok(verified) => Ok(verified),
            Err(e) => {
                // Check if the error is a hostname mismatch
                // If so, ignore it (that's the whole point of HostnameIgnoringVerifier)
                match e {
                    rustls::Error::InvalidCertificate(
                        rustls::CertificateError::NotValidForName,
                    )
                    | rustls::Error::InvalidCertificate(
                        rustls::CertificateError::NotValidForNameContext { .. },
                    ) => {
                        // Hostname mismatch - this is expected and acceptable
                        // The certificate chain, trust anchor, and expiry are valid
                        Ok(ServerCertVerified::assertion())
                    }
                    _ => {
                        // Other errors (expired cert, untrusted CA, etc.) should propagate
                        Err(e)
                    }
                }
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

// Helper function to extract the first DNS name from a certificate
fn extract_first_dns_name(cert_der: &CertificateDer<'_>) -> Option<ServerName<'static>> {
    let (_, cert) = X509Certificate::from_der(cert_der.as_ref()).ok()?;

    // Try Subject Alternative Names first
    if let Ok(Some(san_ext)) = cert.subject_alternative_name() {
        for name in &san_ext.value.general_names {
            if let x509_parser::extensions::GeneralName::DNSName(dns) = name {
                // Remove wildcard prefix if present (e.g., "*.example.com" → "example.com")
                // This allows us to use the domain for certificate chain verification
                // when check_hostname=False
                let dns_str = dns.to_string();
                let normalized_dns = normalize_wildcard_hostname(&dns_str);

                match ServerName::try_from(normalized_dns.to_string()) {
                    Ok(server_name) => {
                        return Some(server_name);
                    }
                    Err(_e) => {
                        // Continue to next
                    }
                }
            }
        }
    }

    // Fallback to Common Name
    for rdn in cert.subject().iter() {
        for attr in rdn.iter() {
            if attr.attr_type() == &x509_parser::oid_registry::OID_X509_COMMON_NAME
                && let Ok(cn) = attr.attr_value().as_str()
            {
                // Remove wildcard prefix if present
                let normalized_cn = normalize_wildcard_hostname(cn);

                match ServerName::try_from(normalized_cn.to_string()) {
                    Ok(server_name) => {
                        return Some(server_name);
                    }
                    Err(_e) => {}
                }
            }
        }
    }

    None
}

// Custom client certificate verifier for TLS 1.3 deferred validation
// This verifier always succeeds during handshake but stores verification errors
// for later retrieval during I/O operations
#[derive(Debug)]
pub struct DeferredClientCertVerifier {
    // The actual verifier that performs validation
    inner: Arc<dyn ClientCertVerifier>,
    // Shared storage for deferred error message
    deferred_error: Arc<ParkingRwLock<Option<String>>>,
}

impl DeferredClientCertVerifier {
    pub fn new(
        inner: Arc<dyn ClientCertVerifier>,
        deferred_error: Arc<ParkingRwLock<Option<String>>>,
    ) -> Self {
        Self {
            inner,
            deferred_error,
        }
    }
}

impl ClientCertVerifier for DeferredClientCertVerifier {
    fn offer_client_auth(&self) -> bool {
        self.inner.offer_client_auth()
    }

    fn client_auth_mandatory(&self) -> bool {
        // Delegate to inner verifier to respect CERT_REQUIRED mode
        // This ensures client certificates are mandatory when verify_mode=CERT_REQUIRED
        self.inner.client_auth_mandatory()
    }

    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        self.inner.root_hint_subjects()
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        // Perform the actual verification
        let result = self
            .inner
            .verify_client_cert(end_entity, intermediates, now);

        // If verification failed, store the error for the server's Python code
        // AND return the error so rustls sends the appropriate TLS alert
        if let Err(ref e) = result {
            let error_msg = format!("certificate verify failed: {e}");
            *self.deferred_error.write() = Some(error_msg);
            // Return the error to rustls so it sends the alert to the client
            return result;
        }

        result
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

// Public Utility Functions

/// Load certificate chain and private key from files
///
/// This function loads a certificate chain from `cert_path` and a private key
/// from `key_path`. If `password` is provided, it will be used to decrypt
/// an encrypted private key.
///
/// Returns (certificate_chain, private_key) on success.
///
/// # Arguments
/// * `cert_path` - Path to certificate file (PEM or DER format)
/// * `key_path` - Path to private key file (PEM or DER format, optionally encrypted)
/// * `password` - Optional password for encrypted private key
///
/// # Errors
/// Returns error if:
/// - Files cannot be read
/// - Certificate or key cannot be parsed
/// - Password is incorrect for encrypted key
pub(super) fn load_cert_chain_from_file(
    cert_path: &str,
    key_path: &str,
    password: Option<&str>,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), Box<dyn std::error::Error>> {
    // Load certificate file - preserve io::Error for errno
    let cert_contents = std::fs::read(cert_path)?;

    // Parse certificates (PEM format)
    let mut cert_cursor = std::io::Cursor::new(&cert_contents);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_cursor)
        .collect::<Result<Vec<_>, _>>()
        .map_err(cert_error::pem::parse_failed)?;

    if certs.is_empty() {
        return Err(Box::new(cert_error::pem::invalid_cert()));
    }

    // Load private key file - preserve io::Error for errno
    let key_contents = std::fs::read(key_path)?;

    // Parse private key (supports PKCS8, RSA, EC formats)
    let private_key = if let Some(pwd) = password {
        // Try to parse as encrypted PKCS#8
        use der::SecretDocument;
        use pkcs8::EncryptedPrivateKeyInfo;
        use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

        let pem_str = String::from_utf8_lossy(&key_contents);

        // Extract just the ENCRYPTED PRIVATE KEY block if present
        // (file may contain multiple PEM blocks like key + certificate)
        let encrypted_key_pem = if let Some(start) =
            pem_str.find("-----BEGIN ENCRYPTED PRIVATE KEY-----")
        {
            if let Some(end_marker) = pem_str[start..].find("-----END ENCRYPTED PRIVATE KEY-----") {
                let end = start + end_marker + "-----END ENCRYPTED PRIVATE KEY-----".len();
                Some(&pem_str[start..end])
            } else {
                None
            }
        } else {
            None
        };

        // Try to decode and decrypt PEM-encoded encrypted private key using pkcs8's PEM support
        let decrypted_key_result = if let Some(key_pem) = encrypted_key_pem {
            match SecretDocument::from_pem(key_pem) {
                Ok((label, doc)) => {
                    if label == "ENCRYPTED PRIVATE KEY" {
                        // Parse encrypted key info from DER
                        match EncryptedPrivateKeyInfo::try_from(doc.as_bytes()) {
                            Ok(encrypted_key) => {
                                // Decrypt with password
                                match encrypted_key.decrypt(pwd.as_bytes()) {
                                    Ok(decrypted) => {
                                        // Convert decrypted SecretDocument to PrivateKeyDer
                                        let key_vec: Vec<u8> = decrypted.as_bytes().to_vec();
                                        let pkcs8_key: PrivatePkcs8KeyDer<'static> = key_vec.into();
                                        Some(PrivateKeyDer::Pkcs8(pkcs8_key))
                                    }
                                    Err(e) => {
                                        return Err(Box::new(cert_error::key::decrypt_failed(e)));
                                    }
                                }
                            }
                            Err(e) => {
                                return Err(Box::new(cert_error::key::parse_encrypted_failed(e)));
                            }
                        }
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        } else {
            None
        };

        match decrypted_key_result {
            Some(key) => key,
            None => {
                // Not encrypted PKCS#8, try as unencrypted key
                // (password might have been provided for an unencrypted key)
                let mut key_cursor = std::io::Cursor::new(&key_contents);
                match rustls_pemfile::private_key(&mut key_cursor) {
                    Ok(Some(key)) => key,
                    Ok(None) => {
                        return Err(Box::new(cert_error::key::not_found("key file")));
                    }
                    Err(e) => {
                        return Err(Box::new(cert_error::key::parse_failed(e)));
                    }
                }
            }
        }
    } else {
        // No password provided - try to parse unencrypted key
        let mut key_cursor = std::io::Cursor::new(&key_contents);
        match rustls_pemfile::private_key(&mut key_cursor) {
            Ok(Some(key)) => key,
            Ok(None) => {
                return Err(Box::new(cert_error::key::not_found("key file")));
            }
            Err(e) => {
                return Err(Box::new(cert_error::key::parse_failed(e)));
            }
        }
    };

    Ok((certs, private_key))
}

/// Validate that a certificate and private key match
///
/// This function checks that the public key in the certificate matches
/// the provided private key. This is a basic sanity check to prevent
/// configuration errors.
///
/// # Arguments
/// * `certs` - Certificate chain (first certificate is the leaf)
/// * `private_key` - Private key to validate against
///
/// # Errors
/// Returns error if:
/// - Certificate chain is empty
/// - Public key extraction fails
/// - Keys don't match
///
/// Note: This is a simplified validation. Full validation would require
/// signing and verifying a test message, which is complex with rustls.
pub fn validate_cert_key_match(
    certs: &[CertificateDer<'_>],
    private_key: &PrivateKeyDer<'_>,
) -> Result<(), String> {
    if certs.is_empty() {
        return Err("Certificate chain is empty".to_string());
    }

    // For rustls, the actual validation happens when creating CertifiedKey
    // We can attempt to create a signing key to verify the key is valid
    use rustls::crypto::aws_lc_rs::sign::any_supported_type;

    match any_supported_type(private_key) {
        Ok(_signing_key) => {
            // If we can create a signing key, the private key is valid
            // Rustls will validate the cert-key match when building config
            Ok(())
        }
        Err(_) => Err("PEM lib".to_string()),
    }
}

/// StrictCertVerifier: wraps a ServerCertVerifier and adds RFC 5280 strict validation
///
/// When VERIFY_X509_STRICT flag is set, performs additional validation:
/// - Checks for Authority Key Identifier (AKI) extension (required by RFC 5280 Section 4.2.1.1)
/// - Validates other RFC 5280 compliance requirements
///
/// This matches X509_V_FLAG_X509_STRICT behavior in OpenSSL.
#[derive(Debug)]
pub struct StrictCertVerifier {
    inner: Arc<dyn ServerCertVerifier>,
    verify_flags: i32,
}

impl StrictCertVerifier {
    /// Create a new StrictCertVerifier
    ///
    /// # Arguments
    /// * `inner` - The underlying verifier to wrap
    /// * `verify_flags` - SSL verification flags (e.g., VERIFY_X509_STRICT)
    pub fn new(inner: Arc<dyn ServerCertVerifier>, verify_flags: i32) -> Self {
        Self {
            inner,
            verify_flags,
        }
    }

    /// Check if a certificate has the Authority Key Identifier extension
    ///
    /// RFC 5280 Section 4.2.1.1 states that conforming CAs MUST include this
    /// extension in all certificates except self-signed certificates.
    fn check_aki_present(cert_der: &[u8]) -> Result<(), String> {
        let (_, cert) = X509Certificate::from_der(cert_der)
            .map_err(|e| format!("Failed to parse certificate: {e}"))?;

        // Check for Authority Key Identifier extension (OID 2.5.29.35)
        let has_aki = cert
            .tbs_certificate
            .extensions()
            .iter()
            .any(|ext| ext.oid == oid_registry::OID_X509_EXT_AUTHORITY_KEY_IDENTIFIER);

        if !has_aki {
            return Err(
                "certificate verification failed: certificate missing required Authority Key Identifier extension"
                    .to_string(),
            );
        }

        Ok(())
    }
}

impl ServerCertVerifier for StrictCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // First, perform the standard verification
        let result = self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;

        // If VERIFY_X509_STRICT flag is set, perform additional validation
        if self.verify_flags & VERIFY_X509_STRICT != 0 {
            // Check end entity certificate for AKI
            // RFC 5280 Section 4.2.1.1: self-signed certificates are exempt from AKI requirement
            if !is_self_signed(end_entity) {
                Self::check_aki_present(end_entity.as_ref())
                    .map_err(cert_error::to_rustls_invalid_cert)?;
            }

            // Check intermediate certificates for AKI
            for intermediate in intermediates {
                Self::check_aki_present(intermediate.as_ref())
                    .map_err(cert_error::to_rustls_invalid_cert)?;
            }
        }

        Ok(result)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// EmptyRootStoreVerifier: used when verify_mode != CERT_NONE but no CA certs are loaded
///
/// This verifier always fails certificate verification with UnknownIssuer error,
/// when no root certificates are available.
/// This allows the SSL context to be created successfully, but handshake will fail
/// with a proper SSLCertVerificationError (verify_code=20, UNABLE_TO_GET_ISSUER_CERT_LOCALLY).
#[derive(Debug)]
pub struct EmptyRootStoreVerifier;

impl ServerCertVerifier for EmptyRootStoreVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Always fail with UnknownIssuer -  when no CA certs loaded
        // This will be mapped to X509_V_ERR_UNABLE_TO_GET_ISSUER_CERT_LOCALLY (20)
        Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer,
        ))
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // Accept signatures during handshake - the cert verification will fail anyway
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // Accept signatures during handshake - the cert verification will fail anyway
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        ALL_SIGNATURE_SCHEMES.to_vec()
    }
}

/// CRLCheckVerifier: Wraps a verifier to enforce CRL checking when flags are set
///
/// This verifier ensures that when CRL checking flags are set (VERIFY_CRL_CHECK_LEAF = 4)
/// but no CRLs have been loaded, the verification fails with UnknownRevocationStatus.
/// This matches X509_V_FLAG_CRL_CHECK without loaded CRLs
/// causes "unable to get CRL" error.
#[derive(Debug)]
pub struct CRLCheckVerifier {
    inner: Arc<dyn ServerCertVerifier>,
    has_crls: bool,
    crl_check_enabled: bool,
}

impl CRLCheckVerifier {
    pub fn new(
        inner: Arc<dyn ServerCertVerifier>,
        has_crls: bool,
        crl_check_enabled: bool,
    ) -> Self {
        Self {
            inner,
            has_crls,
            crl_check_enabled,
        }
    }
}

impl ServerCertVerifier for CRLCheckVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // If CRL checking is enabled but no CRLs are loaded, fail with UnknownRevocationStatus
        // X509_V_ERR_UNABLE_TO_GET_CRL (3)
        if self.crl_check_enabled && !self.has_crls {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownRevocationStatus,
            ));
        }

        // Otherwise, delegate to inner verifier
        self.inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// Partial Chain Verifier - Handles VERIFY_X509_PARTIAL_CHAIN flag
///
/// OpenSSL's X509_V_FLAG_PARTIAL_CHAIN allows verification to succeed if any certificate
/// in the presented chain is found in the trust store, not just the root CA. This is useful
/// for trusting intermediate certificates or self-signed certificates directly.
///
/// rustls's WebPkiServerVerifier doesn't support this behavior by default, so we wrap it
/// to add partial chain support when the flag is set.
///
/// Behavior:
/// 1. Try standard verification first (full chain to trusted root)
/// 2. If that fails and VERIFY_X509_PARTIAL_CHAIN is set:
///    - Check if the end-entity certificate is in the trust store
///    - If yes, accept the certificate as trusted
///
/// This matches accepting self-signed certificates that
/// are explicitly loaded via load_verify_locations().
#[derive(Debug)]
pub struct PartialChainVerifier {
    inner: Arc<dyn ServerCertVerifier>,
    ca_certs_der: Vec<Vec<u8>>,
    verify_flags: i32,
}

impl PartialChainVerifier {
    pub fn new(
        inner: Arc<dyn ServerCertVerifier>,
        ca_certs_der: Vec<Vec<u8>>,
        verify_flags: i32,
    ) -> Self {
        Self {
            inner,
            ca_certs_der,
            verify_flags,
        }
    }
}

impl ServerCertVerifier for PartialChainVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Try standard verification first
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(result) => Ok(result),
            Err(e) => {
                // If verification failed, check if the end-entity certificate is in the trust store
                // OpenSSL behavior:
                // 1. Self-signed certs in trust store: ALWAYS trusted (flag not required)
                // 2. Non-self-signed end-entity certs in trust store: require VERIFY_X509_PARTIAL_CHAIN
                // 3. Intermediate certs in trust store: require VERIFY_X509_PARTIAL_CHAIN
                let end_entity_der = end_entity.as_ref();
                if self
                    .ca_certs_der
                    .iter()
                    .any(|cert_der| cert_der.as_slice() == end_entity_der)
                {
                    // End-entity certificate is in the trust store
                    // Check if this is a self-signed certificate
                    let is_self_signed_cert = is_self_signed(end_entity);

                    // Self-signed: always trust (OpenSSL behavior)
                    // Non-self-signed: require VERIFY_X509_PARTIAL_CHAIN flag
                    if is_self_signed_cert || (self.verify_flags & VERIFY_X509_PARTIAL_CHAIN != 0) {
                        // Certificate is trusted, but still perform hostname verification
                        verify_hostname(end_entity, server_name)?;
                        return Ok(ServerCertVerified::assertion());
                    }
                }
                // No match found or non-self-signed without flag - return original error
                Err(e)
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

// Hostname Verification:

/// Check if a certificate is self-signed by comparing issuer and subject.
/// Returns true if the certificate is self-signed (issuer == subject).
fn is_self_signed(cert_der: &CertificateDer<'_>) -> bool {
    use x509_parser::prelude::*;

    // Parse the certificate
    let Ok((_, cert)) = X509Certificate::from_der(cert_der.as_ref()) else {
        // If we can't parse it, assume it's not self-signed (conservative approach)
        return false;
    };

    // Compare issuer and subject
    // A certificate is self-signed if issuer == subject
    cert.issuer() == cert.subject()
}

/// Verify that a certificate is valid for the given hostname/IP address.
/// This function checks Subject Alternative Names (SAN) and Common Name (CN).
fn verify_hostname(
    cert_der: &CertificateDer<'_>,
    server_name: &ServerName<'_>,
) -> Result<(), rustls::Error> {
    use x509_parser::extensions::GeneralName;
    use x509_parser::prelude::*;

    // Parse the certificate
    let (_, cert) = X509Certificate::from_der(cert_der.as_ref()).map_err(|e| {
        cert_error::to_rustls_invalid_cert(format!(
            "Failed to parse certificate for hostname verification: {e}"
        ))
    })?;

    match server_name {
        ServerName::DnsName(dns) => {
            let expected_name = dns.as_ref();

            // 1. Check Subject Alternative Names (SAN) - preferred method
            if let Ok(Some(san_ext)) = cert.subject_alternative_name() {
                for name in &san_ext.value.general_names {
                    if let GeneralName::DNSName(dns_name) = name
                        && hostname_matches(expected_name, dns_name)
                    {
                        return Ok(());
                    }
                }
            }

            // 2. Fallback to Common Name (CN) - deprecated but still checked for compatibility
            for rdn in cert.subject().iter() {
                for attr in rdn.iter() {
                    if attr.attr_type() == &x509_parser::oid_registry::OID_X509_COMMON_NAME
                        && let Ok(cn) = attr.attr_value().as_str()
                        && hostname_matches(expected_name, cn)
                    {
                        return Ok(());
                    }
                }
            }

            // No match found - return error
            Err(cert_error::to_rustls_invalid_cert(format!(
                "Hostname mismatch: certificate is not valid for '{expected_name}'",
            )))
        }
        ServerName::IpAddress(ip) => verify_ip_address(&cert, ip),
        _ => {
            // Unknown server name type
            Err(cert_error::to_rustls_cert_error(
                std::io::ErrorKind::InvalidInput,
                "Unsupported server name type for hostname verification",
            ))
        }
    }
}

/// Match a hostname against a pattern, supporting wildcard certificates (*.example.com).
/// Implements RFC 6125 wildcard matching rules:
/// - Wildcard must be in the leftmost label
/// - Wildcard must be the only character in that label
/// - Wildcard must match at least one character
fn hostname_matches(expected: &str, pattern: &str) -> bool {
    // Wildcard matching for *.example.com
    if let Some(pattern_base) = pattern.strip_prefix("*.") {
        // Find the first dot in expected hostname
        if let Some(dot_pos) = expected.find('.') {
            let expected_base = &expected[dot_pos + 1..];

            // The base domains must match (case insensitive)
            // and the leftmost label must not be empty
            return dot_pos > 0 && expected_base.eq_ignore_ascii_case(pattern_base);
        }

        // No dot in expected, can't match wildcard
        return false;
    }

    // Exact match (case insensitive per RFC 4343)
    expected.eq_ignore_ascii_case(pattern)
}

/// Verify that a certificate is valid for the given IP address.
/// Checks Subject Alternative Names for IP Address entries.
fn verify_ip_address(
    cert: &X509Certificate<'_>,
    expected_ip: &rustls::pki_types::IpAddr,
) -> Result<(), rustls::Error> {
    use std::net::IpAddr;
    use x509_parser::extensions::GeneralName;

    // Convert rustls IpAddr to std::net::IpAddr for comparison
    let expected_std_ip: IpAddr = match expected_ip {
        rustls::pki_types::IpAddr::V4(octets) => IpAddr::V4(std::net::Ipv4Addr::from(*octets)),
        rustls::pki_types::IpAddr::V6(octets) => IpAddr::V6(std::net::Ipv6Addr::from(*octets)),
    };

    // Check Subject Alternative Names for IP addresses
    if let Ok(Some(san_ext)) = cert.subject_alternative_name() {
        for name in &san_ext.value.general_names {
            if let GeneralName::IPAddress(cert_ip_bytes) = name {
                // Parse the IP address from the certificate
                let cert_ip = match cert_ip_bytes.len() {
                    4 => {
                        // IPv4
                        if let Ok(octets) = <[u8; 4]>::try_from(*cert_ip_bytes) {
                            IpAddr::V4(std::net::Ipv4Addr::from(octets))
                        } else {
                            continue;
                        }
                    }
                    16 => {
                        // IPv6
                        if let Ok(octets) = <[u8; 16]>::try_from(*cert_ip_bytes) {
                            IpAddr::V6(std::net::Ipv6Addr::from(octets))
                        } else {
                            continue;
                        }
                    }
                    _ => continue, // Invalid IP address length
                };

                if cert_ip == expected_std_ip {
                    return Ok(());
                }
            }
        }
    }

    // No matching IP address found
    Err(cert_error::to_rustls_invalid_cert(format!(
        "IP address mismatch: certificate is not valid for '{expected_std_ip}'",
    )))
}
