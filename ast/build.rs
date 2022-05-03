use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn main() {
    println!("cargo:rerun-if-changed=Python.asdl");
    println!("cargo:rerun-if-changed=asdl_rs.py");
    // println!("cargo:rerun-if-changed=../scripts/update_asdl.sh");

    let out_dir: PathBuf = std::env::var("OUT_DIR").unwrap().into();
    // println!("cargo:warning={out_dir:?}");
    let build_dir = {
        let mut path = out_dir;
        path.pop();
        path.pop();
        path
    };

    let def_path = build_dir
        .join("ast_def.rs")
        .to_str()
        .expect("def path is not str representable")
        .to_owned();
    let mod_path = build_dir
        .join("ast_mod.rs")
        .to_str()
        .expect("mod path is not str representable")
        .to_owned();

    let mut cmd = std::process::Command::new("python3");
    let output = cmd
        .args([
            "./asdl_rs.py",
            "-D",
            &def_path,
            "-M",
            &mod_path,
            "./Python.asdl",
        ])
        .output();
    let success = output.as_ref().map_or(false, |o| o.status.success());
    if success {
        // failing fmt is ok
        let _ = std::process::Command::new("rustfmt")
            .args([&def_path, &mod_path])
            .output();
        let c1 = file_changed(&def_path, "bootstrap/ast_def.rs").unwrap();
        let c2 = file_changed(&mod_path, "bootstrap/ast_mod.rs").unwrap();
        if c1 || c2 {
            // ensure bootstrap files are latest
            let _ = std::fs::copy(def_path, "bootstrap/ast_def.rs");
            let _ = std::fs::copy(mod_path, "bootstrap/ast_mod.rs");
        }
    } else {
        let mut error = match output {
            Ok(out) => std::str::from_utf8(out.stderr.as_slice())
                .expect("stderr is not utf8")
                .to_owned(),
            Err(e) => format!("{cmd:?}\n{e:?}"),
        };
        error.insert(0, '\n');
        let error = error.replace('\n', "\n//    ");
        fn bootstrap(
            source: impl AsRef<Path>,
            target: impl AsRef<Path>,
            msg: &str,
        ) -> std::io::Result<()> {
            let content = read_file_content(source)?;
            let mut file = File::create(target)?;
            writeln!(file, "// NEVER COMMIT THIS FILE TO REPOSITORY!")?;
            writeln!(
                file,
                "// This content copied from `ast/bootstrap/` because generation failed."
            )?;
            writeln!(file, "// To prevent bootstrapping, build RustPython then copy `rustpython` binary into executable path as name of `python3`")?;
            writeln!(file, "// Error:{msg}")?;
            writeln!(file, "//")?;
            file.write_all(content.as_bytes())?;
            Ok(())
        }
        bootstrap("bootstrap/ast_def.rs", "src/ast_gen.rs", &error)
            .expect("failed to copy bootstrap ast_def.rs");
        bootstrap(
            "bootstrap/ast_mod.rs",
            "../vm/src/stdlib/ast/gen.rs",
            &error,
        )
        .expect("failed to copy bootstrap ast_mod.rs");
    }
}

fn read_file_content(path: impl AsRef<Path>) -> std::io::Result<String> {
    let path = path.as_ref();
    let mut file = File::open(path)?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(content)
}

fn file_changed(path1: impl AsRef<Path>, path2: impl AsRef<Path>) -> std::io::Result<bool> {
    fn into_headless(mut content: String) -> String {
        content.replace_range(..content.find('\n').unwrap(), "");
        content
    }
    let content1 = into_headless(read_file_content(path1)?);
    let content2 = into_headless(read_file_content(path2)?);
    Ok(content1 != content2)
}
