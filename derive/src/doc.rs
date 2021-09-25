pub(crate) type Result = std::result::Result<Option<&'static str>, ()>;

pub(crate) fn try_read(path: &str) -> Result {
    static DATABASE: once_cell::sync::OnceCell<std::collections::HashMap<String, Option<String>>> =
        once_cell::sync::OnceCell::new();
    let db = DATABASE.get_or_init(|| {
        let raw = include_str!("../docs.json");
        serde_json::from_str(raw).expect("docs.json must be a valid json file")
    });
    let data = db.get(path).ok_or(())?;
    Ok(data.as_ref().map(|s| s.as_str()))
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
