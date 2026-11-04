use core::ffi::CStr;
use std::io;

use schannel::{RawPointer, cert_context::ValidUses, cert_store::CertStore};
use windows_sys::Win32::{
    Foundation::{CRYPT_E_NOT_FOUND, GetLastError},
    Security::Cryptography::{
        CERT_CONTEXT, CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG, CERT_FIND_PROP_ONLY_ENHKEY_USAGE_FLAG,
        CERT_STORE_PROV_SYSTEM_A, CERT_STORE_READONLY_FLAG, CERT_SYSTEM_STORE_LOCAL_MACHINE,
        CRL_CONTEXT, CTL_USAGE, CertCloseStore, CertEnumCRLsInStore, CertEnumCertificatesInStore,
        CertFreeCertificateContext, CertGetEnhancedKeyUsage, CertOpenStore, CertOpenSystemStoreW,
        HCERTSTORE, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodingType {
    X509Asn,
    Pkcs7Asn,
    Other(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CertificateUses {
    All,
    Oids(Vec<String>),
}

#[derive(Debug)]
pub struct CertificateEntry {
    pub der: Vec<u8>,
    pub encoding: EncodingType,
    pub valid_uses: io::Result<CertificateUses>,
}

#[derive(Debug)]
pub struct CertificateEntries {
    pub had_open_store: bool,
    pub entries: Vec<CertificateEntry>,
}

#[derive(Debug)]
pub struct CrlEntry {
    pub der: Vec<u8>,
    pub encoding: EncodingType,
}

fn encoding_type(raw: u32) -> EncodingType {
    if raw & X509_ASN_ENCODING != 0 {
        EncodingType::X509Asn
    } else if raw & PKCS_7_ASN_ENCODING != 0 {
        EncodingType::Pkcs7Asn
    } else {
        EncodingType::Other(raw)
    }
}

fn encoding_type_exact(raw: u32) -> EncodingType {
    match raw {
        X509_ASN_ENCODING => EncodingType::X509Asn,
        PKCS_7_ASN_ENCODING => EncodingType::Pkcs7Asn,
        other => EncodingType::Other(other),
    }
}

fn open_local_machine(store_name: &CStr) -> io::Result<HCERTSTORE> {
    let store = unsafe {
        CertOpenStore(
            CERT_STORE_PROV_SYSTEM_A,
            0,
            0,
            CERT_SYSTEM_STORE_LOCAL_MACHINE | CERT_STORE_READONLY_FLAG,
            store_name.as_ptr().cast(),
        )
    };
    if store.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(store)
    }
}

unsafe fn certificate_uses_for_flags(
    cert: *const CERT_CONTEXT,
    flags: u32,
) -> io::Result<Option<CertificateUses>> {
    let mut size = 0;
    if unsafe { CertGetEnhancedKeyUsage(cert, flags, core::ptr::null_mut(), &mut size) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(CRYPT_E_NOT_FOUND) {
            return Ok(None);
        }
        return Err(error);
    }

    let words = (size as usize)
        .max(core::mem::size_of::<CTL_USAGE>())
        .div_ceil(core::mem::size_of::<usize>());
    let mut buffer = vec![0usize; words];
    let usage = buffer.as_mut_ptr().cast::<CTL_USAGE>();
    if unsafe { CertGetEnhancedKeyUsage(cert, flags, usage, &mut size) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(CRYPT_E_NOT_FOUND) {
            return Ok(None);
        }
        return Err(error);
    }

    let usage = unsafe { &*usage };
    let mut oids = Vec::with_capacity(usage.cUsageIdentifier as usize);
    for index in 0..usage.cUsageIdentifier as usize {
        let oid = unsafe { *usage.rgpszUsageIdentifier.add(index) };
        if !oid.is_null() {
            oids.push(
                unsafe { CStr::from_ptr(oid.cast()) }
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    Ok(Some(CertificateUses::Oids(oids)))
}

unsafe fn certificate_uses(cert: *const CERT_CONTEXT) -> io::Result<CertificateUses> {
    if let Some(uses) =
        unsafe { certificate_uses_for_flags(cert, CERT_FIND_PROP_ONLY_ENHKEY_USAGE_FLAG) }?
    {
        return Ok(uses);
    }
    Ok(
        unsafe { certificate_uses_for_flags(cert, CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG) }?
            .unwrap_or(CertificateUses::All),
    )
}

/// Enumerate one read-only local-machine system store, matching the store
/// selected by CPython's Windows SSL helpers. Opening and closing the store
/// are both fallible; a close failure takes precedence over an enumeration
/// failure because it happens last.
pub fn enum_certificates_local_machine(store_name: &CStr) -> io::Result<Vec<CertificateEntry>> {
    let store = open_local_machine(store_name)?;
    let result = (|| {
        let mut entries = Vec::new();
        let mut context: *const CERT_CONTEXT = core::ptr::null();
        loop {
            context = unsafe { CertEnumCertificatesInStore(store, context) };
            if context.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(CRYPT_E_NOT_FOUND) {
                    break;
                }
                return Err(error);
            }
            let cert = unsafe { &*context };
            let valid_uses = unsafe { certificate_uses(context) };
            if let Err(error) = valid_uses {
                unsafe { CertFreeCertificateContext(context) };
                return Err(error);
            }
            entries.push(CertificateEntry {
                der: unsafe {
                    core::slice::from_raw_parts(cert.pbCertEncoded, cert.cbCertEncoded as usize)
                }
                .to_vec(),
                encoding: encoding_type_exact(cert.dwCertEncodingType),
                valid_uses,
            });
        }
        Ok(entries)
    })();
    if unsafe { CertCloseStore(store, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    result
}

/// Enumerate certificate-revocation lists from one read-only local-machine
/// system store.
pub fn enum_crls_local_machine(store_name: &CStr) -> io::Result<Vec<CrlEntry>> {
    let store = open_local_machine(store_name)?;
    let result = (|| {
        let mut entries = Vec::new();
        let mut context: *const CRL_CONTEXT = core::ptr::null();
        loop {
            context = unsafe { CertEnumCRLsInStore(store, context) };
            if context.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(CRYPT_E_NOT_FOUND) {
                    break;
                }
                return Err(error);
            }
            let crl = unsafe { &*context };
            entries.push(CrlEntry {
                der: unsafe {
                    core::slice::from_raw_parts(crl.pbCrlEncoded, crl.cbCrlEncoded as usize)
                }
                .to_vec(),
                encoding: encoding_type_exact(crl.dwCertEncodingType),
            });
        }
        Ok(entries)
    })();
    if unsafe { CertCloseStore(store, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    result
}

pub fn enum_certificates(store_name: &str) -> CertificateEntries {
    let open_fns = [CertStore::open_current_user, CertStore::open_local_machine];
    let mut had_open_store = false;
    let mut entries = Vec::new();

    for open in open_fns {
        let Ok(store) = open(store_name) else {
            continue;
        };
        had_open_store = true;

        for cert in store.certs() {
            let encoding = unsafe {
                let ptr = cert.as_ptr() as *const CERT_CONTEXT;
                encoding_type((*ptr).dwCertEncodingType)
            };
            let valid_uses = cert.valid_uses().map_or_else(
                |err| Err(io::Error::other(err)),
                |uses| {
                    Ok(match uses {
                        ValidUses::All => CertificateUses::All,
                        ValidUses::Oids(oids) => CertificateUses::Oids(oids.into_iter().collect()),
                    })
                },
            );
            entries.push(CertificateEntry {
                der: cert.to_der().to_owned(),
                encoding,
                valid_uses,
            });
        }
    }

    CertificateEntries {
        had_open_store,
        entries,
    }
}

pub fn enum_crls(store_name: &str) -> io::Result<Vec<CrlEntry>> {
    let store_name_wide: Vec<u16> = store_name
        .encode_utf16()
        .chain(core::iter::once(0))
        .collect();

    let store = unsafe { CertOpenSystemStoreW(0, store_name_wide.as_ptr()) };
    if store.is_null() {
        return Err(io::Error::last_os_error());
    }

    let mut result = Vec::new();
    let mut crl_context: *const CRL_CONTEXT = core::ptr::null();
    loop {
        crl_context = unsafe { CertEnumCRLsInStore(store, crl_context) };
        if crl_context.is_null() {
            let err = unsafe { GetLastError() };
            if err != CRYPT_E_NOT_FOUND as u32 {
                unsafe {
                    CertCloseStore(store, 0);
                }
                return Err(io::Error::from_raw_os_error(err as i32));
            }
            break;
        }

        let crl = unsafe { &*crl_context };
        let der =
            unsafe { core::slice::from_raw_parts(crl.pbCrlEncoded, crl.cbCrlEncoded as usize) }
                .to_vec();
        result.push(CrlEntry {
            der,
            encoding: encoding_type(crl.dwCertEncodingType),
        });
    }

    unsafe {
        CertCloseStore(store, 0);
    }

    Ok(result)
}
