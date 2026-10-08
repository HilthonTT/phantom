//! A service that implements [`Service`](super::Service) but is missing from
//! `Services::build` compiles, passes clippy and never runs: nothing references
//! it, so nothing fails. This compares the two by reading the source.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

/// Each `impl ... Service for Service` in the crate, keyed by the last segment
/// of its module path, which is how `Services::build` names it too.
fn implemented(src: &Path) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    let mut dirs = vec![src.to_path_buf()];

    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).expect("readable source directory") {
            let path = entry.expect("readable directory entry").path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }

            let source = fs::read_to_string(&path).expect("readable source file");
            let implements = source.lines().any(|line| {
                let line = line.trim();
                line == "impl crate::Service for Service {" || line == "impl Service for Service {"
            });

            if !implements || path.starts_with(src.join("runtime")) {
                continue;
            }

            let module = path
                .strip_prefix(src)
                .expect("path inside src")
                .with_extension("")
                .to_string_lossy()
                .trim_end_matches("/mod")
                .replace('/', "::");

            let leaf = module.rsplit("::").next().unwrap_or(&module).to_owned();
            if let Some(other) = found.insert(leaf.clone(), module.clone()) {
                panic!("{module} and {other} share the name {leaf:?}; this test needs updating");
            }
        }
    }

    found
}

/// The module of each `build!(...)` in `Services::build`, by its last segment.
fn built(services: &str) -> BTreeSet<String> {
    services
        .split("build!(")
        .skip(1)
        .filter_map(|rest| rest.split_once(')'))
        .filter_map(|(ty, _)| ty.strip_suffix("::Service"))
        .map(|module| module.rsplit("::").next().unwrap_or(module).to_owned())
        .collect()
}

#[test]
fn every_service_is_built() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let services = fs::read_to_string(src.join("runtime/services.rs")).expect("services.rs");

    let implemented = implemented(&src);
    let built = built(&services);

    // Guard against the scan itself breaking and comparing two empty sets.
    assert!(!implemented.is_empty(), "found no Service implementations");
    assert!(
        !built.is_empty(),
        "found no build! calls in Services::build"
    );

    let unbuilt: Vec<_> = implemented
        .iter()
        .filter(|(leaf, _)| !built.contains(*leaf))
        .map(|(_, module)| module.as_str())
        .collect();

    assert!(
        unbuilt.is_empty(),
        "services implemented but never added to Services::build: {unbuilt:?}"
    );
}

mod dependencies {
    use std::{
        any::{TypeId, type_name},
        collections::BTreeMap,
        sync::{Arc, RwLock},
    };

    use async_trait::async_trait;
    use phantom_core::Result;

    use super::super::{
        Args, Map, Service,
        contract::Requested,
        registry::{self, try_get},
        services::check_dependencies,
    };

    struct Built;
    struct NeverBuilt;

    #[async_trait]
    impl Service for Built {
        fn build(_: Args<'_>) -> Result<Arc<Self>> {
            Ok(Arc::new(Self))
        }

        fn name(&self) -> &str {
            "built"
        }
    }

    #[async_trait]
    impl Service for NeverBuilt {
        fn build(_: Args<'_>) -> Result<Arc<Self>> {
            Ok(Arc::new(Self))
        }

        fn name(&self) -> &str {
            "never_built"
        }
    }

    fn requested<T: 'static>() -> Requested {
        Requested {
            type_id: TypeId::of::<T>(),
            type_name: type_name::<T>(),
        }
    }

    fn map_with_built() -> (Map, Arc<Built>) {
        let map: Map = RwLock::new(BTreeMap::new());
        let built = Arc::new(Built);
        registry::add(&map, built.clone(), built.clone());

        (map, built)
    }

    #[test]
    fn a_dependency_resolves_by_type() {
        let (map, built) = map_with_built();

        assert!(Arc::ptr_eq(
            &try_get::<Built>(&map).expect("registered"),
            &built
        ));
        assert!(try_get::<NeverBuilt>(&map).is_err());
    }

    #[test]
    fn met_dependencies_pass() {
        let (map, _built) = map_with_built();

        check_dependencies(&map, &[("dependent", requested::<Built>())]).expect("all built");
    }

    #[test]
    fn an_unbuilt_dependency_fails_naming_both_sides() {
        let (map, _built) = map_with_built();
        let dependencies = [
            (
                "phantom_service::rooms::timeline::Service",
                requested::<Built>(),
            ),
            (
                "phantom_service::rooms::timeline::Service",
                requested::<NeverBuilt>(),
            ),
        ];

        let error = check_dependencies(&map, &dependencies)
            .expect_err("NeverBuilt is missing")
            .to_string();

        assert!(error.contains("rooms::timeline -> "), "{error}");
        assert!(error.contains("NeverBuilt"), "{error}");
        assert_eq!(
            error.matches(" -> ").count(),
            1,
            "only the unmet one: {error}"
        );
    }
}
