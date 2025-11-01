pub type Result = std::result::Result<Option<&'static str>, ()>;

pub struct Database {
    inner: phf::Map<&'static str, Option<&'static str>>,
}

impl Database {
    pub fn shared() -> &'static Self {
        static DATABASE: Database = {
            #[cfg(windows)]
            let data = include!("./win32.inc.rs");

            #[cfg(any(target_os = "linux", target_os = "android"))]
            let data = include!("./linux.inc.rs");

            #[cfg(any(target_os = "macos", target_os = "ios"))]
            let data = include!("./darwin.inc.rs");

            Database { inner: data }
        };

        &DATABASE
    }

    pub fn try_path(&self, path: &str) -> Result {
        self.inner.get(path).copied().ok_or(())
    }

    pub fn try_module_item(&self, module: &str, item: &str) -> Result {
        self.try_path(&format!("{}.{}", module, item))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_module_item() {
        let doc = Database::shared()
            .try_module_item("array", "_array_reconstructor")
            .unwrap();
        assert!(doc.is_some());
    }
}
