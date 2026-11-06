//! Resource indexes bound to a particular registration of frozen modules.

use crate::frozen::FrozenModule;
use alloc::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use core::fmt;
use std::collections::HashMap;

/// A package-relative logical POSIX path and its embedded contents.
pub type FrozenResource = (&'static str, &'static [u8]);
/// A frozen package name and the resources registered with its code.
pub type FrozenResourceBundle = (&'static str, &'static [FrozenResource]);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceError {
    InvalidPath(String),
    DuplicatePath(String),
    FileDirectoryCollision(String),
    DuplicateModule(String),
    DuplicateOwner(String),
    MissingOwner(String),
    OwnerNotPackage(String),
    NotFound(String),
    IsDirectory(String),
    NotDirectory(String),
}

impl fmt::Display for ResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (description, path) = match self {
            Self::InvalidPath(path) => ("invalid relative resource path", path),
            Self::DuplicatePath(path) => ("duplicate resource path", path),
            Self::FileDirectoryCollision(path) => ("resource is both a file and directory", path),
            Self::DuplicateModule(path) => {
                ("duplicate frozen module in resource registration", path)
            }
            Self::DuplicateOwner(path) => ("duplicate frozen resource owner", path),
            Self::MissingOwner(path) => ("resource owner is absent from this registration", path),
            Self::OwnerNotPackage(path) => ("resource owner is not a package", path),
            Self::NotFound(path) => ("resource does not exist", path),
            Self::IsDirectory(path) => ("resource is a directory", path),
            Self::NotDirectory(path) => ("resource is not a directory", path),
        };
        write!(f, "{description}: {path:?}")
    }
}

impl core::error::Error for ResourceError {}

fn normalize_relative(path: &str) -> Result<String, ResourceError> {
    if path.starts_with('/') || path.contains(['\\', '\0', ':']) {
        return Err(ResourceError::InvalidPath(path.to_owned()));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(ResourceError::InvalidPath(path.to_owned())),
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

/// An immutable tree of embedded files, with explicit inferred directories.
#[derive(Debug)]
pub struct ResourceIndex {
    root_name: &'static str,
    files: BTreeMap<String, &'static [u8]>,
    directories: BTreeSet<String>,
}

impl ResourceIndex {
    fn add_directory(&mut self, path: &str) -> Result<(), ResourceError> {
        let normalized = normalize_relative(path)?;
        if normalized != path {
            return Err(ResourceError::InvalidPath(path.to_owned()));
        }
        let mut directory = path;
        loop {
            if self.files.contains_key(directory) {
                return Err(ResourceError::FileDirectoryCollision(directory.to_owned()));
            }
            self.directories.insert(directory.to_owned());
            let Some((parent, _)) = directory.rsplit_once('/') else {
                break;
            };
            directory = parent;
        }
        Ok(())
    }

    fn new(owner: &'static str, resources: &[FrozenResource]) -> Result<Self, ResourceError> {
        let mut index = Self {
            root_name: owner.rsplit('.').next().unwrap_or(owner),
            files: BTreeMap::new(),
            directories: BTreeSet::from([String::new()]),
        };
        for &(path, bytes) in resources {
            let normalized = normalize_relative(path)?;
            // Manifest paths are canonical; query paths may contain harmless dots or slashes.
            if normalized.is_empty() || normalized != path {
                return Err(ResourceError::InvalidPath(path.to_owned()));
            }
            if index.files.contains_key(path) {
                return Err(ResourceError::DuplicatePath(path.to_owned()));
            }
            if index.directories.contains(path) {
                return Err(ResourceError::FileDirectoryCollision(path.to_owned()));
            }
            let mut parent = path;
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                if index.files.contains_key(prefix) {
                    return Err(ResourceError::FileDirectoryCollision(prefix.to_owned()));
                }
                index.directories.insert(prefix.to_owned());
                parent = prefix;
            }
            index.files.insert(normalized, bytes);
        }
        Ok(index)
    }
}

/// An immutable snapshot of one directory or file in an embedded resource tree.
#[derive(Clone, Debug)]
pub struct ResourceView {
    index: Arc<ResourceIndex>,
    prefix: String,
}

impl ResourceView {
    #[must_use]
    pub fn name(&self) -> &str {
        if self.prefix.is_empty() {
            self.index.root_name
        } else {
            self.prefix.rsplit('/').next().unwrap_or(&self.prefix)
        }
    }

    #[must_use]
    pub fn is_file(&self) -> bool {
        self.index.files.contains_key(&self.prefix)
    }

    #[must_use]
    pub fn is_dir(&self) -> bool {
        self.index.directories.contains(&self.prefix)
    }

    pub fn join(&self, descendant: &str) -> Result<Self, ResourceError> {
        let descendant = normalize_relative(descendant)?;
        let prefix = match (self.prefix.is_empty(), descendant.is_empty()) {
            (_, true) => self.prefix.clone(),
            (true, false) => descendant,
            (false, false) => format!("{}/{descendant}", self.prefix),
        };
        Ok(Self {
            index: self.index.clone(),
            prefix,
        })
    }

    pub fn children(&self) -> Result<Vec<Self>, ResourceError> {
        if !self.is_dir() {
            return Err(if self.is_file() {
                ResourceError::NotDirectory(self.prefix.clone())
            } else {
                ResourceError::NotFound(self.prefix.clone())
            });
        }
        let prefix = if self.prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", self.prefix)
        };
        let children: BTreeSet<_> = self
            .index
            .files
            .keys()
            .chain(self.index.directories.iter())
            .filter(|path| {
                path.strip_prefix(prefix.as_str())
                    .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
            })
            .cloned()
            .collect();
        Ok(children
            .into_iter()
            .map(|prefix| Self {
                index: self.index.clone(),
                prefix,
            })
            .collect())
    }

    pub fn bytes(&self) -> Result<&'static [u8], ResourceError> {
        self.index.files.get(&self.prefix).copied().ok_or_else(|| {
            if self.is_dir() {
                ResourceError::IsDirectory(self.prefix.clone())
            } else {
                ResourceError::NotFound(self.prefix.clone())
            }
        })
    }
}

pub(crate) struct FrozenRegistration {
    modules: Vec<(&'static str, FrozenModule)>,
    resources: BTreeMap<&'static str, Arc<ResourceIndex>>,
}

impl FrozenRegistration {
    pub(crate) fn code_only(
        modules: impl IntoIterator<Item = (&'static str, FrozenModule)>,
    ) -> Self {
        Self {
            modules: modules.into_iter().collect(),
            resources: BTreeMap::new(),
        }
    }

    pub(crate) fn with_resources(
        modules: impl IntoIterator<Item = (&'static str, FrozenModule)>,
        resources: &[FrozenResourceBundle],
    ) -> Result<Self, ResourceError> {
        let mut registration = Self::code_only(modules);
        let mut names = BTreeMap::new();
        for &(name, module) in &registration.modules {
            if names.insert(name, module.package).is_some() {
                return Err(ResourceError::DuplicateModule(name.to_owned()));
            }
        }
        for &(owner, files) in resources {
            if owner.is_empty()
                || owner.split('.').any(str::is_empty)
                || owner.contains(['/', '\\', '\0', ':'])
            {
                return Err(ResourceError::InvalidPath(owner.to_owned()));
            }
            match names.get(owner) {
                None => return Err(ResourceError::MissingOwner(owner.to_owned())),
                Some(false) => return Err(ResourceError::OwnerNotPackage(owner.to_owned())),
                Some(true) => {}
            }
            if registration.resources.contains_key(owner) {
                return Err(ResourceError::DuplicateOwner(owner.to_owned()));
            }
            let mut index = ResourceIndex::new(owner, files)?;
            for (&name, &package) in &names {
                if package
                    && let Some(suffix) = name.strip_prefix(owner)
                    && let Some(relative) = suffix.strip_prefix('.')
                {
                    index.add_directory(&relative.replace('.', "/"))?;
                }
            }
            registration.resources.insert(owner, Arc::new(index));
        }
        Ok(registration)
    }
}

#[derive(Clone, Copy)]
struct FrozenEntry {
    module: FrozenModule,
    registration: u64,
}

#[derive(Clone)]
struct ResourceBinding {
    owner: &'static str,
    owner_registration: u64,
    anchor_registration: u64,
    view: ResourceView,
}

/// Frozen modules with resource ownership invalidated on every replacement.
///
/// Mutable map access is deliberately not exposed: even identical-byte replacements
/// are distinct registrations and must not inherit another registration's resources.
#[derive(Clone, Default)]
pub struct FrozenModuleRegistry {
    modules: HashMap<&'static str, FrozenEntry, rapidhash::quality::RandomState>,
    bindings: BTreeMap<&'static str, ResourceBinding>,
    next_registration: u64,
}

impl FrozenModuleRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&FrozenModule> {
        self.modules.get(name).map(|entry| &entry.module)
    }

    pub fn keys(&self) -> impl Iterator<Item = &&'static str> {
        self.modules.keys()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    #[must_use]
    pub fn contains_key(&self, name: &str) -> bool {
        self.modules.contains_key(name)
    }

    fn next_token(&mut self) -> u64 {
        self.next_registration = self
            .next_registration
            .checked_add(1)
            .expect("frozen module registration counter exhausted");
        self.next_registration
    }

    fn invalidate(&mut self, name: &str) {
        self.bindings.retain(|anchor, binding| {
            *anchor != name
                && binding.owner != name
                && !anchor
                    .strip_prefix(name)
                    .is_some_and(|suffix| suffix.starts_with('.'))
        });
    }

    pub fn insert(&mut self, name: &'static str, module: FrozenModule) -> Option<FrozenModule> {
        let registration = self.next_token();
        self.invalidate(name);
        self.modules
            .insert(
                name,
                FrozenEntry {
                    module,
                    registration,
                },
            )
            .map(|entry| entry.module)
    }

    pub fn remove(&mut self, name: &str) -> Option<FrozenModule> {
        self.invalidate(name);
        self.modules.remove(name).map(|entry| entry.module)
    }

    pub fn clear(&mut self) {
        self.bindings.clear();
        self.modules.clear();
    }

    pub fn register_with_resources(
        &mut self,
        modules: impl IntoIterator<Item = (&'static str, FrozenModule)>,
        resources: &[FrozenResourceBundle],
    ) -> Result<(), ResourceError> {
        let registration = FrozenRegistration::with_resources(modules, resources)?;
        self.apply_registration(registration);
        Ok(())
    }

    pub(crate) fn apply_registration(&mut self, batch: FrozenRegistration) {
        let registration = self.next_token();
        for &(name, module) in &batch.modules {
            self.invalidate(name);
            self.modules.insert(
                name,
                FrozenEntry {
                    module,
                    registration,
                },
            );
        }
        for &(anchor, module) in &batch.modules {
            let package = if module.package {
                anchor
            } else if let Some((package, _)) = anchor.rsplit_once('.') {
                package
            } else {
                continue;
            };
            let owner = batch
                .resources
                .keys()
                .copied()
                .filter(|owner| {
                    package == *owner
                        || package
                            .strip_prefix(*owner)
                            .is_some_and(|suffix| suffix.starts_with('.'))
                })
                .max_by_key(|owner| owner.len());
            let Some(owner) = owner else {
                continue;
            };
            // Every containing package down to the owner must belong to this batch.
            let mut current = package;
            let eligible =
                loop {
                    if !self.modules.get(current).is_some_and(|entry| {
                        entry.registration == registration && entry.module.package
                    }) {
                        break false;
                    }
                    if current == owner {
                        break true;
                    }
                    let Some((parent, _)) = current.rsplit_once('.') else {
                        break false;
                    };
                    current = parent;
                };
            if !eligible {
                continue;
            }
            let prefix = package
                .strip_prefix(owner)
                .unwrap_or_default()
                .trim_start_matches('.')
                .replace('.', "/");
            let view = ResourceView {
                index: batch.resources[owner].clone(),
                prefix,
            };
            self.bindings.insert(
                anchor,
                ResourceBinding {
                    owner,
                    owner_registration: registration,
                    anchor_registration: registration,
                    view,
                },
            );
        }
    }

    /// Acquire a snapshot only if both the exact anchor and owner are still current.
    #[must_use]
    pub fn resource_view(&self, fullname: &str) -> Option<ResourceView> {
        let binding = self.bindings.get(fullname)?;
        let anchor = self.modules.get(fullname)?;
        let owner = self.modules.get(binding.owner)?;
        (anchor.registration == binding.anchor_registration
            && owner.registration == binding.owner_registration
            && owner.module.package)
            .then(|| binding.view.clone())
    }
}

impl Extend<(&'static str, FrozenModule)> for FrozenModuleRegistry {
    fn extend<T: IntoIterator<Item = (&'static str, FrozenModule)>>(&mut self, iter: T) {
        self.apply_registration(FrozenRegistration::code_only(iter));
    }
}

impl FromIterator<(&'static str, FrozenModule)> for FrozenModuleRegistry {
    fn from_iter<T: IntoIterator<Item = (&'static str, FrozenModule)>>(iter: T) -> Self {
        let mut registry = Self::new();
        registry.extend(iter);
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frozen::FrozenCodeObject;

    const ASSETS: &[FrozenResource] =
        &[("data.txt", b"hello"), ("nested/4.1.3/data.bin", b"\0\xff")];
    const BUNDLES: &[FrozenResourceBundle] = &[("sample", ASSETS)];

    fn module(package: bool) -> FrozenModule {
        FrozenModule {
            code: FrozenCodeObject {
                bytes: b"same-code",
            },
            package,
        }
    }

    fn registered() -> FrozenModuleRegistry {
        let mut registry = FrozenModuleRegistry::new();
        registry
            .register_with_resources(
                [("sample", module(true)), ("sample.worker", module(false))],
                BUNDLES,
            )
            .unwrap();
        registry
    }

    #[test]
    fn replacement_never_inherits_identical_code_resources() {
        let mut registry = registered();
        let snapshot = registry.resource_view("sample").unwrap();
        registry.insert("sample", module(true));
        assert!(registry.resource_view("sample").is_none());
        assert!(registry.resource_view("sample.worker").is_none());
        assert_eq!(
            snapshot.join("data.txt").unwrap().bytes().unwrap(),
            b"hello"
        );
        registry
            .register_with_resources([("sample", module(true))], BUNDLES)
            .unwrap();
        assert!(registry.resource_view("sample").is_some());
        assert!(registry.resource_view("sample.worker").is_none());
    }

    #[test]
    fn anchor_replacement_and_unrelated_registration() {
        let mut registry = registered();
        registry.extend([("other", module(true)), ("sample.worker", module(false))]);
        assert!(registry.resource_view("sample").is_some());
        assert!(registry.resource_view("sample.worker").is_none());
        assert!(registry.resource_view("sample.unknown").is_none());
        assert!(registry.resource_view("samplex").is_none());
    }

    #[test]
    fn clones_have_independent_ownership() {
        let parent = registered();
        let mut child = parent.clone();
        child.remove("sample");
        assert!(parent.resource_view("sample").is_some());
        assert!(child.resource_view("sample").is_none());
        child.insert("sample", module(true));
        assert!(child.resource_view("sample").is_none());
    }

    #[test]
    fn resource_replacement_and_clear_do_not_merge_old_assets() {
        const REPLACEMENT: &[FrozenResourceBundle] = &[("sample", &[("new.txt", b"new")])];
        let mut registry = registered();
        registry
            .register_with_resources([("sample", module(true))], REPLACEMENT)
            .unwrap();
        let view = registry.resource_view("sample").unwrap();
        assert!(!view.join("data.txt").unwrap().is_file());
        assert_eq!(view.join("new.txt").unwrap().bytes().unwrap(), b"new");
        registry.clear();
        registry.extend([("sample", module(true))]);
        assert!(registry.resource_view("sample").is_none());
    }

    #[test]
    fn invalid_batch_is_atomic() {
        let mut registry = registered();
        assert_eq!(
            registry.register_with_resources([("other", module(true))], BUNDLES),
            Err(ResourceError::MissingOwner("sample".to_owned()))
        );
        assert!(!registry.contains_key("other"));
        assert!(registry.resource_view("sample").is_some());
        assert_eq!(
            registry.register_with_resources([("sample", module(false))], BUNDLES),
            Err(ResourceError::OwnerNotPackage("sample".to_owned()))
        );
        assert_eq!(
            registry.register_with_resources(
                [("sample", module(true)), ("sample", module(true))],
                BUNDLES
            ),
            Err(ResourceError::DuplicateModule("sample".to_owned()))
        );
    }

    #[test]
    fn tree_queries_are_component_based() {
        let registry = registered();
        let root = registry.resource_view("sample").unwrap();
        assert_eq!(root.name(), "sample");
        assert!(root.is_dir());
        assert_eq!(
            root.children()
                .unwrap()
                .iter()
                .map(ResourceView::name)
                .collect::<Vec<_>>(),
            ["data.txt", "nested"]
        );
        assert_eq!(
            root.join("./nested//4.1.3/data.bin")
                .unwrap()
                .bytes()
                .unwrap(),
            b"\0\xff"
        );
        assert!(!root.join("nest").unwrap().is_dir());
        assert!(matches!(root.bytes(), Err(ResourceError::IsDirectory(_))));
        assert!(matches!(
            root.join("data.txt").unwrap().children(),
            Err(ResourceError::NotDirectory(_))
        ));
        let missing = root.join("missing").unwrap();
        assert!(!missing.is_file() && !missing.is_dir());
        assert!(matches!(missing.bytes(), Err(ResourceError::NotFound(_))));
        assert!(matches!(
            missing.children(),
            Err(ResourceError::NotFound(_))
        ));
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        let root = registered().resource_view("sample").unwrap();
        for path in [
            "../data.txt",
            "nested/../data.txt",
            "/data.txt",
            "C:data.txt",
            "C:/data.txt",
            "\\\\host\\file",
            "nested\\data.txt",
            "nul\0name",
        ] {
            assert!(matches!(
                root.join(path),
                Err(ResourceError::InvalidPath(_))
            ));
        }
        assert_eq!(root.join("").unwrap().name(), "sample");
    }

    #[test]
    fn manifests_reject_collisions_in_either_order() {
        for entries in [
            vec![("a", b"x".as_slice()), ("a/b", b"x".as_slice())],
            vec![("a/b", b"x".as_slice()), ("a", b"x".as_slice())],
        ] {
            assert!(matches!(
                ResourceIndex::new("sample", &entries),
                Err(ResourceError::FileDirectoryCollision(_))
            ));
        }
        assert!(matches!(
            ResourceIndex::new("sample", &[("a", b"x"), ("a", b"y")]),
            Err(ResourceError::DuplicatePath(_))
        ));
        assert!(matches!(
            ResourceIndex::new("sample", &[("./a", b"x")]),
            Err(ResourceError::InvalidPath(_))
        ));
    }

    #[test]
    fn nested_package_anchors_require_same_registration() {
        let mut registry = FrozenModuleRegistry::new();
        registry.insert("sample.nested", module(true));
        registry
            .register_with_resources(
                [
                    ("sample", module(true)),
                    ("sample.nested.worker", module(false)),
                ],
                BUNDLES,
            )
            .unwrap();
        assert!(registry.resource_view("sample.nested.worker").is_none());
        registry
            .register_with_resources(
                [
                    ("sample", module(true)),
                    ("sample.nested", module(true)),
                    ("sample.nested.worker", module(false)),
                ],
                BUNDLES,
            )
            .unwrap();
        let view = registry.resource_view("sample.nested.worker").unwrap();
        assert_eq!(view.name(), "nested");
        assert!(view.is_dir());
        registry.insert("sample.nested", module(true));
        assert!(registry.resource_view("sample.nested.worker").is_none());
    }

    #[test]
    fn package_without_files_has_an_empty_directory_view() {
        let mut registry = FrozenModuleRegistry::new();
        registry
            .register_with_resources(
                [("sample", module(true)), ("sample.empty", module(true))],
                BUNDLES,
            )
            .unwrap();
        let root = registry.resource_view("sample.empty").unwrap();
        assert!(root.is_dir());
        assert!(!root.is_file());
        assert!(root.children().unwrap().is_empty());
        assert!(matches!(root.bytes(), Err(ResourceError::IsDirectory(_))));
    }

    #[test]
    fn package_directories_cannot_collide_with_embedded_files() {
        const FILE: &[FrozenResourceBundle] = &[("sample", &[("nested", b"file")])];
        let mut registry = FrozenModuleRegistry::new();
        assert_eq!(
            registry.register_with_resources(
                [("sample", module(true)), ("sample.nested", module(true))],
                FILE,
            ),
            Err(ResourceError::FileDirectoryCollision("nested".to_owned()))
        );
        assert!(registry.is_empty());
    }
}
