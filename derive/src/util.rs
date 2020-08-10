use super::Diagnostic;
use std::collections::HashMap;
use syn::{Attribute, Ident, Lit, Meta, NestedMeta, Path};

pub(crate) fn path_eq(path: &Path, s: &str) -> bool {
    path.get_ident().map_or(false, |id| id == s)
}

pub(crate) fn def_to_name(
    attr_name: &'static str,
    ident: &Ident,
    attrs: &[NestedMeta],
) -> Result<String, Diagnostic> {
    optional_attribute_arg(attr_name, "name", attrs)
        .transpose()
        .unwrap_or_else(|| Ok(ident.to_string()))
}

pub(crate) fn attribute_arg(
    attr_name: &'static str,
    arg_name: &'static str,
    attrs: &[NestedMeta],
) -> Result<String, Diagnostic> {
    if let Some(r) = optional_attribute_arg(attr_name, arg_name, attrs).transpose() {
        r
    } else {
        bail_span!(
            attrs[0],
            "#[{}({} = ...)] must exist but not found",
            attr_name,
            arg_name
        )
    }
}

pub(crate) fn optional_attribute_arg(
    attr_name: &'static str,
    arg_name: &'static str,
    attrs: &[NestedMeta],
) -> Result<Option<String>, Diagnostic> {
    let mut arg_value = None;
    for attr in attrs {
        match attr {
            NestedMeta::Meta(Meta::NameValue(name_value))
                if path_eq(&name_value.path, arg_name) =>
            {
                if let Lit::Str(lit) = &name_value.lit {
                    if arg_value.is_some() {
                        bail_span!(
                            name_value.lit,
                            "#[{}({} = ...)] must be unique but found multiple times",
                            attr_name,
                            arg_name
                        );
                    }
                    arg_value = Some(lit.value());
                } else {
                    bail_span!(
                        name_value.lit,
                        "#[{}({} = ...)] must be a string",
                        attr_name,
                        arg_name
                    );
                }
            }
            _ => continue,
        }
    }
    Ok(arg_value)
}

pub(crate) fn meta_into_nesteds(meta: Meta) -> Result<Vec<NestedMeta>, Meta> {
    match meta {
        Meta::Path(_) => Ok(Vec::new()),
        Meta::List(list) => Ok(list.nested.into_iter().collect()),
        Meta::NameValue(_) => Err(meta),
    }
}

#[derive(PartialEq)]
pub(crate) enum ItemType {
    Fn,
    Method,
    Struct,
    Enum,
    Const,
}

pub(crate) struct ItemIdent<'a> {
    pub typ: ItemType,
    pub attrs: &'a mut Vec<Attribute>,
    pub ident: &'a Ident,
}

pub(crate) struct ItemMeta<'a> {
    ident: &'a Ident,
    parent_type: &'static str,
    meta: HashMap<String, Option<Lit>>,
}

impl<'a> ItemMeta<'a> {
    pub const SIMPLE_NAMES: &'static [&'static str] = &["name"];
    pub const STRUCT_SEQUENCE_NAMES: &'static [&'static str] = &["module", "name"];
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

    pub fn optional_name(&self) -> Option<String> {
        self.simple_name().ok()
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
                if let Some(name) = sig_name.strip_prefix("set_") {
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
