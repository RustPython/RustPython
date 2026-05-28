//! Example project to demonstrate how to set a custom rustls provider for RustPython.

// spell-checker: ignore graviola

use std::env;

use rustls::crypto::ring;
use rustpython_pylib::FROZEN_STDLIB;
use rustpython_stdlib::{ssl::providers::CryptoExt, stdlib_module_defs};
use rustpython_vm::Interpreter;

const SCRIPT: &str = r#"
import urllib.request

with urllib.request.urlopen("https://python.org") as response:
    assert response.status == 200
"#;

fn main() {
    let provider = env::args()
        .skip(1)
        .find_map(|arg| match &*arg {
            "--ring" => Some("ring"),
            "--graviola" => Some("graviola"),
            _ => None,
        })
        .unwrap_or("ring");

    match provider {
        "ring" => {
            let ext = CryptoExt {
                all_cipher_suites: Some(ring::ALL_CIPHER_SUITES),
                all_kx_groups: Some(ring::ALL_KX_GROUPS),
                any_supported_key: Some(ring::sign::any_supported_type),
                ticketer: ring::Ticketer::new,
            };
            CryptoExt::set_provider(ring::default_provider(), ext).unwrap();
            println!("Using ring for cryptography");
        }
        "graviola" => {
            let ext = CryptoExt {
                all_cipher_suites: Some(rustls_graviola::suites::ALL_CIPHER_SUITES),
                all_kx_groups: Some(rustls_graviola::kx::ALL_KX_GROUPS),
                any_supported_key: None,
                ticketer: rustls_graviola::Ticketer::new,
            };
            CryptoExt::set_provider(rustls_graviola::default_provider(), ext).unwrap();
            println!("Using Graviola for cryptography");
        }
        unsupported => panic!("Unsupported provider: {unsupported}"),
    }

    let builder = Interpreter::builder(Default::default());
    let defs = stdlib_module_defs(&builder.ctx);
    let result = builder
        .add_native_modules(&defs)
        .add_frozen_modules(FROZEN_STDLIB)
        .build()
        .run(|vm| {
            let scope = vm.new_scope_with_builtins();
            vm.run_block_expr(scope, SCRIPT).map(|_| ())
        });

    assert_eq!(0, result);
}
