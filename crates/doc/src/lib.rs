#![no_std]

include!("./data.inc.rs");

#[cfg(test)]
mod test {
    use super::DB;

    #[test]
    fn db_basic() {
        let doc = DB.get("array._array_reconstructor");
        assert!(doc.is_some());
    }
}
