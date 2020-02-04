pub fn path_eq(path: &syn::Path, s: &str) -> bool {
    path.get_ident().map_or(false, |id| id == s)
}
