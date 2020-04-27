use super::Diagnostic;
use std::collections::HashMap;
use syn::{Attribute, AttributeArgs, Ident, Lit, Meta, NestedMeta, Path};

pub fn path_eq(path: &Path, s: &str) -> bool {
    path.get_ident().map_or(false, |id| id == s)
}

pub fn def_to_name(
    ident: &Ident,
    attr_name: &'static str,
    attr: AttributeArgs,
) -> Result<String, Diagnostic> {
    let mut name = None;
    for attr in attr {
        if let NestedMeta::Meta(meta) = attr {
            if let Meta::NameValue(name_value) = meta {
                if path_eq(&name_value.path, "name") {
                    if let Lit::Str(s) = name_value.lit {
                        name = Some(s.value());
                    } else {
                        bail_span!(
                            name_value.lit,
                            "#[{}(name = ...)] must be a string",
                            attr_name
                        );
                    }
                }
            }
        }
    }
    Ok(name.unwrap_or_else(|| ident.to_string()))
}

pub fn strip_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.starts_with(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

pub struct ItemIdent<'a> {
    pub attrs: &'a mut Vec<Attribute>,
    pub ident: &'a Ident,
}

pub struct ItemMeta<'a> {
    ident: &'a Ident,
    parent_type: &'static str,
    meta: HashMap<String, Option<Lit>>,
}

impl<'a> ItemMeta<'a> {
    pub const SIMPLE_NAMES: &'static [&'static str] = &["name"];
    pub const ATTRIBUTE_NAMES: &'static [&'static str] = &["name", "magic"];
    pub const PROPERTY_NAMES: &'static [&'static str] = &["name", "magic", "setter"];

    pub fn from_nested_meta(
        parent_type: &'static str,
        ident: &'a Ident,
        nested_meta: &[NestedMeta],
        names: &[&'static str],
    ) -> Result<Self, Diagnostic> {
        let mut extracted = Self {
            ident,
            parent_type,
            meta: HashMap::new(),
        };

        let validate_name = |name: &str, extracted: &Self| -> Result<(), Diagnostic> {
            if names.contains(&name) {
                if extracted.meta.contains_key(name) {
                    bail_span!(ident, "#[{}] must have only one '{}'", parent_type, name);
                } else {
                    Ok(())
                }
            } else {
                bail_span!(
                    ident,
                    "#[{}({})] is not one of allowed attributes {}",
                    parent_type,
                    name,
                    names.join(", ")
                );
            }
        };

        for meta in nested_meta {
            let meta = match meta {
                NestedMeta::Meta(meta) => meta,
                NestedMeta::Lit(_) => continue,
            };

            match meta {
                Meta::NameValue(name_value) => {
                    if let Some(ident) = name_value.path.get_ident() {
                        let name = ident.to_string();
                        validate_name(&name, &extracted)?;
                        extracted.meta.insert(name, Some(name_value.lit.clone()));
                    }
                }
                Meta::Path(path) => {
                    if let Some(ident) = path.get_ident() {
                        let name = ident.to_string();
                        validate_name(&name, &extracted)?;
                        extracted.meta.insert(name, None);
                    } else {
                        continue;
                    }
                }
                _ => (),
            }
        }

        Ok(extracted)
    }

    fn _str(&self, key: &str) -> Result<Option<String>, Diagnostic> {
        Ok(match self.meta.get(key) {
            Some(Some(lit)) => {
                if let Lit::Str(s) = lit {
                    Some(s.value())
                } else {
                    bail_span!(
                        &self.ident,
                        "#[{}({} = ...)] must be a string",
                        self.parent_type,
                        key
                    );
                }
            }
            Some(None) => {
                bail_span!(
                    &self.ident,
                    "#[{}({} = ...)] is expected",
                    self.parent_type,
                    key,
                );
            }
            None => None,
        })
    }

    fn _bool(&self, key: &str) -> Result<bool, Diagnostic> {
        Ok(match self.meta.get(key) {
            Some(Some(_)) => {
                bail_span!(&self.ident, "#[{}({})] is expected", self.parent_type, key,);
            }
            Some(None) => true,
            None => false,
        })
    }

    pub fn simple_name(&self) -> Result<String, Diagnostic> {
        Ok(self._str("name")?.unwrap_or_else(|| self.ident.to_string()))
    }

    pub fn method_name(&self) -> Result<String, Diagnostic> {
        let name = self._str("name")?;
        let magic = self._bool("magic")?;
        Ok(if let Some(name) = name {
            name
        } else {
            let name = self.ident.to_string();
            if magic {
                format!("__{}__", name)
            } else {
                name
            }
        })
    }

    pub fn property_name(&self) -> Result<String, Diagnostic> {
        let magic = self._bool("magic")?;
        let setter = self._bool("setter")?;
        let name = self._str("name")?;

        Ok(if let Some(name) = name {
            name
        } else {
            let sig_name = self.ident.to_string();
            let name = if setter {
                if let Some(name) = strip_prefix(&sig_name, "set_") {
                    if name.is_empty() {
                        bail_span!(
                            &self.ident,
                            "A #[{}(setter)] fn with a set_* name must \
                             have something after \"set_\"",
                            self.parent_type
                        )
                    }
                    name.to_string()
                } else {
                    bail_span!(
                        &self.ident,
                        "A #[{}(setter)] fn must either have a `name` \
                         parameter or a fn name along the lines of \"set_*\"",
                        self.parent_type
                    )
                }
            } else {
                sig_name
            };
            if magic {
                format!("__{}__", name)
            } else {
                name
            }
        })
    }

    pub fn setter(&self) -> Result<bool, Diagnostic> {
        self._bool("setter")
    }
}
