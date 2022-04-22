use once_cell::sync::Lazy;
use std::collections::HashMap;
pub(crate) type Result = std::result::Result<Option<&'static str>, ()>;

pub(crate) fn try_read(path: &str) -> Result {
    static DATABASE: Lazy<HashMap<&str, Option<&str>>> = Lazy::new(|| {
        let data = include!("../docs.rsinc");
        let mut map = HashMap::with_capacity(data.len());
        for (item, doc) in data {
            map.insert(item, doc);
        }
        map
    });
    DATABASE.get(path).copied().ok_or(())
}

pub(crate) fn try_module_item(module: &str, item: &str) -> Result {
    try_read(&format!("{}.{}", module, item))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_module_item() {
        let doc = try_module_item("array", "_array_reconstructor").unwrap();
        assert!(doc.is_some());
    }
}
