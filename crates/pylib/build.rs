const CRATE_ROOT: &str = "../..";

fn main() {
    #[cfg(feature = "freeze-stdlib")]
    embed_profiling_resources();
    process_python_libs(format!("{CRATE_ROOT}/vm/Lib/python_builtins/*").as_str());
    process_python_libs(format!("{CRATE_ROOT}/vm/Lib/core_modules/*").as_str());

    #[cfg(feature = "freeze-stdlib")]
    if cfg!(windows) {
        process_python_libs(format!("{CRATE_ROOT}/Lib/**/*").as_str());
    } else {
        process_python_libs("./Lib/**/*");
    }

    if cfg!(windows) {
        // On Windows, the Lib entry can be either:
        // 1. A text file containing the relative path (git without symlink support)
        // 2. A proper symlink (git with symlink support)
        // We handle both cases to resolve to the actual Lib directory.
        let lib_path = if let Ok(real_path) = std::fs::read_to_string("Lib") {
            // Case 1: Text file containing relative path
            std::path::PathBuf::from(real_path.trim())
        } else {
            // Case 2: Symlink or directory - canonicalize directly
            std::path::PathBuf::from("Lib")
        };

        if let Ok(canonicalized_path) = std::fs::canonicalize(&lib_path) {
            // Strip the extended path prefix (\\?\) that canonicalize adds on Windows
            let path_str = canonicalized_path.to_str().unwrap();
            let path_str = path_str.strip_prefix(r"\\?\").unwrap_or(path_str);
            println!("cargo:rustc-env=win_lib_path={path_str}");
        }
    }
}

// remove *.pyc files and add *.py to watch list
fn process_python_libs(pattern: &str) {
    let glob = glob::glob(pattern).unwrap_or_else(|e| panic!("failed to glob {pattern:?}: {e}"));
    for entry in glob.flatten() {
        if entry.is_dir() {
            continue;
        }
        let display = entry.display();
        if display.to_string().ends_with(".pyc") {
            if std::fs::remove_file(&entry).is_err() {
                println!("cargo:warning=failed to remove {display}")
            }
            continue;
        }
        println!("cargo:rerun-if-changed={display}");
    }
}

// spell-checker: ignore pyfile
// Only these canonical CPython 3.15.0 assets are bundled. This is deliberately
// not a recursive embedding of Lib data or tests. Keep the notices in
// crates/pylib/LICENSE-profiling with every distribution of these bytes.
#[cfg(feature = "freeze-stdlib")]
const PROFILING_RESOURCES: &[(&str, &str)] = &[
    (
        "profiling/sampling/_assets/tachyon-logo.png",
        "b72b0e28079ee7c033458f279e1c721d4d689db43f6e616848504d09ebe9c85d",
    ),
    (
        "profiling/sampling/_flamegraph_assets/flamegraph.css",
        "a57f0512fe7071ca7915ac063e45df2173d7cea3b1fdded36fbb481cc14dafbb",
    ),
    (
        "profiling/sampling/_flamegraph_assets/flamegraph.js",
        "9b39e61bfeadb6e4f9132c6dedb3b6f62f421821dc98261079ca6942f7649995",
    ),
    (
        "profiling/sampling/_flamegraph_assets/flamegraph_template.html",
        "6469b5f46e27eaccf9b212e2d30d30807bca15873ea106a66726cbafc0babf39",
    ),
    (
        "profiling/sampling/_heatmap_assets/heatmap.css",
        "17dd5a24ea930b2c8e3e51cd16dda20b68434273b9400ef3f965159e31213608",
    ),
    (
        "profiling/sampling/_heatmap_assets/heatmap.js",
        "335f02d7b8baa94f0294afc0a71a3a4cbc5799187524524ce2a62ee18d16023f",
    ),
    (
        "profiling/sampling/_heatmap_assets/heatmap_index.js",
        "d75ff0fe6781bba58b3a2ce3af63a329da39a7002f3eabf1adcc3cdc14c4d3fd",
    ),
    (
        "profiling/sampling/_heatmap_assets/heatmap_index_template.html",
        "152b8b80c6cd318403c8e4a7a6d3b9256ae2dc3459698c606398bb58e199cffe",
    ),
    (
        "profiling/sampling/_heatmap_assets/heatmap_pyfile_template.html",
        "ed68e3905f19b4535d561628359464ef7d575332a126a827fdc7b70ca4c17099",
    ),
    (
        "profiling/sampling/_heatmap_assets/heatmap_shared.js",
        "15e4ba8b7830fb5e33f9aec95c42acf62b950df11210884aee62eddd580a68ab",
    ),
    (
        "profiling/sampling/_shared_assets/base.css",
        "f09f2a549ce69a31e6a3fc29979a32204781320b377bd6488f00f71c3811af23",
    ),
    (
        "profiling/sampling/_shared_assets/base.js",
        "b6f0d0a0720a7f3b20b14be710b7db257a89098178f3ab785d77da76eb019b18",
    ),
    (
        "profiling/sampling/_vendor/d3/7.8.5/d3.min.js",
        "d6b03aefc9f6c44c7bc78713679c78c295028fa914319119e5cc4b4954855b1c",
    ),
    (
        "profiling/sampling/_vendor/d3-flame-graph/4.1.3/d3-flamegraph-tooltip.min.js",
        "105738fc5fe7ca67327cf9a91a63fbd54e5fd3d2becd0e07650ca919b5678ac9",
    ),
    (
        "profiling/sampling/_vendor/d3-flame-graph/4.1.3/d3-flamegraph.css",
        "7d75d68cb662819a37401248863100dd9db351f4bebf3310845d9196fbab7e2a",
    ),
    (
        "profiling/sampling/_vendor/d3-flame-graph/4.1.3/d3-flamegraph.min.js",
        "1846f1e636ed1bd2c32fc87d1e24d63aff7e4f18629b4bbd819351f09ea22353",
    ),
];

#[cfg(feature = "freeze-stdlib")]
fn embed_profiling_resources() {
    use core::fmt::Write;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;

    // Match LIB_PATH's Windows checkout-link handling without relying on Git's
    // ability to create a symlink. Canonicalize the root before containment checks.
    let lib = if cfg!(windows) {
        std::fs::read_to_string("Lib")
            .map_or_else(|_| PathBuf::from("Lib"), |link| PathBuf::from(link.trim()))
    } else {
        PathBuf::from("Lib")
    };
    let lib = lib
        .canonicalize()
        .expect("resolve canonical Lib resource root");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let mut source = String::from(
        "pub const FROZEN_STDLIB_RESOURCES: &[FrozenResourceBundle] = &[(\"profiling\", &[\n",
    );
    println!("cargo:rerun-if-changed=Lib");
    for (index, &(relative, expected)) in PROFILING_RESOURCES.iter().enumerate() {
        let path = lib.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        let canonical = path.canonicalize().expect("resolve profiling resource");
        assert!(
            canonical.starts_with(&lib),
            "profiling resource escapes Lib: {relative}"
        );
        assert!(
            canonical.is_file(),
            "profiling resource is not a file: {relative}"
        );
        let bytes = std::fs::read(&canonical).expect("read profiling resource");
        let mut actual = String::with_capacity(64);
        for byte in Sha256::digest(&bytes) {
            write!(actual, "{byte:02x}").expect("write digest hex");
        }
        assert_eq!(
            actual, expected,
            "profiling resource differs from pinned CPython: {relative}"
        );
        // Snapshot exactly the bytes we hashed, rather than asking include_bytes!
        // to reread a mutable source file after this build script has returned.
        let snapshot = out.join(format!("profiling-resource-{index}.bin"));
        std::fs::write(&snapshot, bytes).expect("write owned resource snapshot");
        let logical = relative
            .strip_prefix("profiling/")
            .expect("profiling resource path");
        writeln!(source, "({logical:?}, include_bytes!({snapshot:?})),")
            .expect("write resource source");
    }
    source.push_str("])];\n");
    std::fs::write(out.join("frozen_resources.rs"), source).expect("write resource sidecar");
}
