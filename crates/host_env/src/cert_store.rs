use std::io;

use schannel::{RawPointer, cert_context::ValidUses, cert_store::CertStore};
use windows_sys::Win32::Security::Cryptography::{
    CERT_CONTEXT, CRL_CONTEXT, CertCloseStore, CertEnumCRLsInStore, CertOpenSystemStoreW,
    PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
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
    match raw {
        X509_ASN_ENCODING => EncodingType::X509Asn,
        PKCS_7_ASN_ENCODING => EncodingType::Pkcs7Asn,
        other => EncodingType::Other(other),
    }
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
