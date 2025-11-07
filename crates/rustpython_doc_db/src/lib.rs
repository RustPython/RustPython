#[cfg(windows)]
include!("./win32.inc.rs");

#[cfg(any(target_os = "linux", target_os = "android"))]
include!("./linux.inc.rs");

#[cfg(any(target_os = "macos", target_os = "ios"))]
include!("./darwin.inc.rs");

#[cfg(test)]
mod test {
    use super::DB;

    #[test]
    fn test_db() {
        let doc = DB.get("array._array_reconstructor");
        assert!(doc.is_some());
    }
}
