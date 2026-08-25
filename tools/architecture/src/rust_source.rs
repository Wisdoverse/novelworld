use anyhow::{bail, Context, Result};
use proc_macro2::Span;
use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    Attribute, Block, Expr, ExprCall, ExprMethodCall, File, ImplItemFn, ItemConst, ItemFn,
    ItemMacro, ItemMod, ItemUse, Local, Macro, Path as SynPath, Stmt, Token, UseTree,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSpec {
    pub package: String,
    pub root: String,
    pub liveness_path: String,
    pub readiness_path: String,
    pub required_env: Vec<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Diagnostic {
    pub code: &'static str,
    pub path: String,
    pub line: usize,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct SqlCall {
    pub service: String,
    pub path: String,
    pub line: usize,
    pub function: String,
    pub sql: String,
}

#[derive(Default)]
pub struct SourceAudit {
    pub diagnostics: Vec<Diagnostic>,
    pub sql_calls: Vec<SqlCall>,
    pub rust_files: usize,
    pub domain_files: usize,
    pub application_files: usize,
    pub explicit_references: usize,
    runtime_facts: BTreeMap<String, RuntimeFacts>,
}

#[derive(Default)]
struct RuntimeFacts {
    env_keys: BTreeSet<String>,
    routes: BTreeMap<String, String>,
    paths: BTreeSet<String>,
    methods: BTreeSet<String>,
    strings: BTreeSet<String>,
    functions: BTreeMap<String, FunctionFacts>,
}

#[derive(Default)]
struct FunctionFacts {
    paths: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
struct CargoTargets {
    production: Vec<PathBuf>,
    custom_build: Vec<PathBuf>,
}

#[derive(Clone, Default)]
struct SqlAliases {
    crate_names: HashSet<String>,
    functions: HashMap<String, String>,
    executor_imported: bool,
    executor_names: HashSet<String>,
    executor_functions: HashMap<String, String>,
    issues: Vec<(Span, String)>,
}

#[derive(Clone, Default)]
struct InternalAliases {
    paths: HashMap<String, Vec<String>>,
    issues: Vec<(Span, String)>,
}

fn collect_internal_aliases(file: &File) -> InternalAliases {
    collect_internal_aliases_with_dependencies(file, &HashMap::new())
}

fn collect_internal_aliases_with_dependencies(
    file: &File,
    dependency_packages: &HashMap<String, String>,
) -> InternalAliases {
    let mut aliases = InternalAliases::default();
    for _ in 0..4 {
        aliases.issues.clear();
        collect_sensitive_aliases_from_items(&file.items, &mut aliases, dependency_packages, false);
    }
    aliases
}

fn collect_sensitive_aliases_from_items(
    items: &[syn::Item],
    aliases: &mut InternalAliases,
    dependency_packages: &HashMap<String, String>,
    test_only: bool,
) {
    for item in items {
        let item_test_only = test_only || item_attributes(item).is_some_and(item_is_test);
        if item_test_only {
            continue;
        }
        match item {
            syn::Item::Use(item) => {
                collect_internal_use_aliases(&item.tree, &[], aliases, dependency_packages)
            }
            syn::Item::ExternCrate(item) => {
                let Some((_, rename)) = &item.rename else {
                    continue;
                };
                let source = item.ident.to_string();
                let target = if source == "self" {
                    vec!["crate".into()]
                } else {
                    vec![source.clone()]
                };
                if alias_sensitive_root(&source, dependency_packages) {
                    aliases.paths.insert(rename.to_string(), target);
                    aliases.issues.push((
                        item.span(),
                        "aliases for architecture roots obscure layer or capability checks and are forbidden"
                            .into(),
                    ));
                }
            }
            syn::Item::Mod(module) => {
                if let Some((_, nested)) = &module.content {
                    collect_sensitive_aliases_from_items(
                        nested,
                        aliases,
                        dependency_packages,
                        false,
                    );
                }
            }
            _ => {}
        }
    }
}

fn collect_internal_use_aliases(
    tree: &UseTree,
    prefix: &[String],
    aliases: &mut InternalAliases,
    dependency_packages: &HashMap<String, String>,
) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix.to_vec();
            next.push(path.ident.to_string());
            collect_internal_use_aliases(&path.tree, &next, aliases, dependency_packages);
        }
        UseTree::Rename(rename) => {
            let mut full = prefix.to_vec();
            full.push(rename.ident.to_string());
            let mut expanded = expand_internal_alias(&full, &aliases.paths);
            if expanded.len() > 1 && expanded.last().is_some_and(|part| part == "self") {
                expanded.pop();
            }
            let internal_root = expanded
                .first()
                .is_some_and(|root| matches!(root.as_str(), "crate" | "self" | "super"));
            let sensitive_root_alias = expanded.len() == 1
                && expanded
                    .first()
                    .is_some_and(|root| alias_sensitive_root(root, dependency_packages));
            if internal_root || sensitive_root_alias {
                aliases.paths.insert(rename.rename.to_string(), expanded);
                aliases.issues.push((
                    rename.span(),
                    "aliases for architecture roots obscure layer or capability checks and are forbidden"
                        .into(),
                ));
            }
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_internal_use_aliases(item, prefix, aliases, dependency_packages);
            }
        }
        UseTree::Name(_) | UseTree::Glob(_) => {}
    }
}

fn alias_sensitive_root(root: &str, dependency_packages: &HashMap<String, String>) -> bool {
    if matches!(root, "crate" | "self" | "super" | "std" | "tokio") {
        return true;
    }
    let package = dependency_packages
        .get(root)
        .map(String::as_str)
        .unwrap_or(root);
    package == "reqwest"
        || package == "bcrypt"
        || matches!(
            package,
            "dotenvy" | "config" | "metrics-exporter-prometheus"
        )
        || helper_persistence_capability(package)
        || forbidden_outbound_transport(package)
}

fn expand_internal_alias(
    segments: &[String],
    aliases: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let Some((first, tail)) = segments.split_first() else {
        return Vec::new();
    };
    let Some(prefix) = aliases.get(first) else {
        return segments.to_vec();
    };
    let mut expanded = prefix.clone();
    expanded.extend_from_slice(tail);
    expanded
}

#[derive(Clone, Copy)]
enum SqlApi {
    Literal {
        function_index: usize,
        macro_index: usize,
    },
    Unsupported,
}

fn collect_sql_aliases(file: &File) -> SqlAliases {
    collect_sql_aliases_with_dependencies(file, &HashMap::new())
}

fn collect_sql_aliases_with_dependencies(
    file: &File,
    dependency_packages: &HashMap<String, String>,
) -> SqlAliases {
    let mut aliases = SqlAliases::default();
    aliases.crate_names.insert("sqlx".into());
    aliases.crate_names.extend(
        dependency_packages
            .iter()
            .filter(|(_, package)| package.as_str() == "sqlx")
            .map(|(source, _)| source.clone()),
    );
    let mut collector = SqlAliasCollector {
        aliases: &mut aliases,
    };
    collector.visit_file(file);
    // Rust imports are order-independent. The second pass resolves aliases such
    // as `use sqlx as db; use db::query as q` even when declarations are reversed.
    aliases.issues.clear();
    let mut collector = SqlAliasCollector {
        aliases: &mut aliases,
    };
    collector.visit_file(file);
    aliases
}

struct SqlAliasCollector<'a> {
    aliases: &'a mut SqlAliases,
}

impl Visit<'_> for SqlAliasCollector<'_> {
    fn visit_item_use(&mut self, item: &ItemUse) {
        let public = !matches!(item.vis, syn::Visibility::Inherited);
        collect_sql_use(&item.tree, &[], public, self.aliases);
    }

    fn visit_item_extern_crate(&mut self, item: &syn::ItemExternCrate) {
        if self.aliases.crate_names.contains(&item.ident.to_string()) {
            let alias = item
                .rename
                .as_ref()
                .map(|(_, alias)| alias.to_string())
                .unwrap_or_else(|| "sqlx".into());
            self.aliases.crate_names.insert(alias);
            if !matches!(item.vis, syn::Visibility::Inherited) {
                self.aliases.issues.push((
                    item.span(),
                    "public sqlx re-exports are outside the local symbol proof".into(),
                ));
            }
        }
    }
}

fn collect_sql_use(tree: &UseTree, prefix: &[String], public: bool, aliases: &mut SqlAliases) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix.to_vec();
            next.push(path.ident.to_string());
            collect_sql_use(&path.tree, &next, public, aliases);
        }
        UseTree::Name(name) => {
            let mut full = prefix.to_vec();
            full.push(name.ident.to_string());
            register_sql_use(&full, name.ident.to_string(), public, name.span(), aliases);
        }
        UseTree::Rename(rename) => {
            let mut full = prefix.to_vec();
            full.push(rename.ident.to_string());
            register_sql_use(
                &full,
                rename.rename.to_string(),
                public,
                rename.span(),
                aliases,
            );
        }
        UseTree::Glob(glob) => {
            if prefix
                .first()
                .is_some_and(|segment| aliases.crate_names.contains(segment))
            {
                aliases.issues.push((
                    glob.span(),
                    "sqlx glob imports are outside the local symbol proof".into(),
                ));
            }
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_sql_use(item, prefix, public, aliases);
            }
        }
    }
}

fn register_sql_use(
    full: &[String],
    local: String,
    public: bool,
    span: Span,
    aliases: &mut SqlAliases,
) {
    let Some(root) = full.first() else {
        return;
    };
    let from_sqlx = root == "sqlx" || aliases.crate_names.contains(root);
    let from_executor = aliases.executor_names.contains(root);
    let aliased_function = (full.len() == 1)
        .then(|| aliases.functions.get(root).cloned())
        .flatten();
    if !from_sqlx && !from_executor && aliased_function.is_none() {
        return;
    }
    if public {
        aliases.issues.push((
            span,
            "public sqlx re-exports are outside the local symbol proof".into(),
        ));
    }
    if full.len() == 1 {
        if from_executor {
            aliases.executor_imported = true;
            aliases.executor_names.insert(local);
        } else if let Some(canonical) = aliased_function {
            aliases.functions.insert(local, canonical);
        } else {
            aliases.crate_names.insert(local);
        }
    } else if let Some(canonical) = full.last() {
        if canonical == "Executor" {
            aliases.executor_imported = true;
            aliases.executor_names.insert(local.clone());
        } else if (from_executor
            || (full.len() >= 3
                && full
                    .get(full.len() - 2)
                    .is_some_and(|name| name == "Executor")))
            && executor_api(canonical)
        {
            aliases.executor_imported = true;
            aliases
                .executor_functions
                .insert(local.clone(), canonical.clone());
        }
        if sql_api(canonical).is_some() {
            aliases.functions.insert(local, canonical.clone());
        }
    }
}

fn audit_composition_root(file: &File, relative: &str, diagnostics: &mut Vec<Diagnostic>) {
    const ALLOWED_FUNCTIONS: [&str; 4] =
        ["main", "run_body", "trace_middleware", "shutdown_signal"];
    for item in &file.items {
        if item_attributes(item).is_some_and(item_is_test) {
            continue;
        }
        let allowed = match item {
            syn::Item::Use(_) => true,
            syn::Item::Mod(module) => {
                module.content.is_none()
                    && matches!(
                        module.ident.to_string().as_str(),
                        "domain" | "application" | "infrastructure" | "interface"
                    )
            }
            syn::Item::Fn(function) => {
                ALLOWED_FUNCTIONS.contains(&function.sig.ident.to_string().as_str())
            }
            _ => false,
        };
        if !allowed {
            diagnostics.push(Diagnostic {
                code: "LAYER008",
                path: relative.into(),
                line: item.span().start().line.max(1),
                message: "service main/lib roots are composition-only; move types, impls, and business functions into a DDD layer".into(),
            });
        }
    }
}

fn item_attributes(item: &syn::Item) -> Option<&[Attribute]> {
    match item {
        syn::Item::Const(item) => Some(&item.attrs),
        syn::Item::Enum(item) => Some(&item.attrs),
        syn::Item::ExternCrate(item) => Some(&item.attrs),
        syn::Item::Fn(item) => Some(&item.attrs),
        syn::Item::ForeignMod(item) => Some(&item.attrs),
        syn::Item::Impl(item) => Some(&item.attrs),
        syn::Item::Macro(item) => Some(&item.attrs),
        syn::Item::Mod(item) => Some(&item.attrs),
        syn::Item::Static(item) => Some(&item.attrs),
        syn::Item::Struct(item) => Some(&item.attrs),
        syn::Item::Trait(item) => Some(&item.attrs),
        syn::Item::TraitAlias(item) => Some(&item.attrs),
        syn::Item::Type(item) => Some(&item.attrs),
        syn::Item::Union(item) => Some(&item.attrs),
        syn::Item::Use(item) => Some(&item.attrs),
        syn::Item::Verbatim(_) => None,
        _ => None,
    }
}

pub fn audit_sources(root: &Path, runtimes: &[RuntimeSpec]) -> Result<SourceAudit> {
    let metadata = load_cargo_metadata(root)?;
    let dependencies = dependency_source_maps(&metadata)?;
    let targets = cargo_targets_from_metadata(&metadata)?;
    audit_sources_with_dependencies_and_targets(root, runtimes, &dependencies, &targets)
}

fn audit_sources_with_dependencies(
    root: &Path,
    runtimes: &[RuntimeSpec],
    dependencies: &BTreeMap<String, HashMap<String, String>>,
) -> Result<SourceAudit> {
    let mut targets = BTreeMap::new();
    for runtime in runtimes {
        let source_root = root.join(&runtime.root).join("src");
        let production = [source_root.join("main.rs"), source_root.join("lib.rs")]
            .into_iter()
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        targets.insert(
            runtime.package.clone(),
            CargoTargets {
                production,
                custom_build: Vec::new(),
            },
        );
    }
    audit_sources_with_dependencies_and_targets(root, runtimes, dependencies, &targets)
}

fn audit_sources_with_dependencies_and_targets(
    root: &Path,
    runtimes: &[RuntimeSpec],
    dependencies: &BTreeMap<String, HashMap<String, String>>,
    targets: &BTreeMap<String, CargoTargets>,
) -> Result<SourceAudit> {
    let mut audit = SourceAudit::default();
    for runtime in runtimes {
        let runtime_targets = targets.get(&runtime.package).with_context(|| {
            format!("runtime {} has no cargo target inventory", runtime.package)
        })?;
        if runtime_targets.production.is_empty() {
            bail!("runtime {} has no normal lib/bin target", runtime.package);
        }
        let facts = audit
            .runtime_facts
            .entry(runtime.package.clone())
            .or_default();

        for configured_target in &runtime_targets.production {
            let target = configured_target.canonicalize().with_context(|| {
                format!("canonicalize cargo target {}", configured_target.display())
            })?;
            let source_root = target
                .parent()
                .context("cargo target has no source parent")?;
            let files = module_closure_from_entries(std::slice::from_ref(&target))?;
            for (file_path, test_only) in files {
                let relative = normalize_path(file_path.strip_prefix(root).unwrap_or(&file_path));
                let source = fs::read_to_string(&file_path)
                    .with_context(|| format!("read Rust source {relative}"))?;
                let syntax = syn::parse_file(&source)
                    .with_context(|| format!("parse Rust source {relative}"))?;
                audit.rust_files += 1;
                let module = if file_path == target {
                    Vec::new()
                } else {
                    module_for(&file_path, source_root)
                };
                let layer = module.first().and_then(|segment| Layer::from_name(segment));
                let composition_root =
                    !test_only && runtime.package != "gateway" && file_path == target;
                if composition_root {
                    audit_composition_root(&syntax, &relative, &mut audit.diagnostics);
                }
                if !test_only
                    && runtime.package != "gateway"
                    && !module.is_empty()
                    && layer.is_none()
                {
                    audit.diagnostics.push(Diagnostic {
                    code: "LAYER007",
                    path: relative.clone(),
                    line: 1,
                    message: format!(
                        "production module {} must live under domain, application, infrastructure, or interface",
                        module.join("::")
                    ),
                });
                }
                if layer == Some(Layer::Domain) {
                    audit.domain_files += 1;
                } else if layer == Some(Layer::Application) {
                    audit.application_files += 1;
                }

                let constants = collect_constants(&syntax);
                let dependency_packages = dependencies
                    .get(&runtime.package)
                    .cloned()
                    .unwrap_or_default();
                let sql_aliases =
                    collect_sql_aliases_with_dependencies(&syntax, &dependency_packages);
                let internal_aliases =
                    collect_internal_aliases_with_dependencies(&syntax, &dependency_packages);
                let mut analyzer = Analyzer {
                    service: &runtime.package,
                    relative: &relative,
                    module,
                    layer,
                    diagnostics: &mut audit.diagnostics,
                    sql_calls: &mut audit.sql_calls,
                    explicit_references: &mut audit.explicit_references,
                    facts,
                    constants,
                    sql_aliases,
                    internal_aliases,
                    local_names: collect_local_names(&syntax),
                    dependency_packages,
                    scopes: Vec::new(),
                    query_scopes: Vec::new(),
                    current_function: "<module>".into(),
                    facts_enabled: !test_only,
                    in_use: false,
                    composition_root,
                    saw_reqwest: false,
                    saw_internal_http_marker: false,
                };
                for (span, message) in analyzer.sql_aliases.issues.clone() {
                    analyzer.push("SQL006", span, message);
                }
                for (span, message) in analyzer.internal_aliases.issues.clone() {
                    analyzer.push("LAYER010", span, message);
                }
                analyzer.visit_file(&syntax);
                analyzer.finish_file();
            }
        }
    }
    Ok(audit)
}

pub type WorkspaceHelperPolicy = BTreeMap<String, BTreeSet<String>>;

pub fn audit_cargo_with_workspace_helpers(
    root: &Path,
    runtimes: &[RuntimeSpec],
    helpers: &WorkspaceHelperPolicy,
) -> Result<Vec<Diagnostic>> {
    let metadata = load_cargo_metadata(root)?;
    cargo_diagnostics_from_metadata_with_helpers(root, runtimes, &metadata, helpers)
}

fn load_cargo_metadata(root: &Path) -> Result<serde_json::Value> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--locked", "--format-version", "1"])
        .current_dir(root)
        .output()
        .context("run cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).context("parse cargo metadata JSON")
}

fn dependency_source_maps(
    metadata: &serde_json::Value,
) -> Result<BTreeMap<String, HashMap<String, String>>> {
    let workspace_members = metadata["workspace_members"]
        .as_array()
        .context("cargo metadata workspace_members is not an array")?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<HashSet<_>>();
    let packages = metadata["packages"]
        .as_array()
        .context("cargo metadata packages is not an array")?;
    let mut maps = BTreeMap::new();
    for package in packages {
        let id = package["id"].as_str().context("package id is missing")?;
        if !workspace_members.contains(id) {
            continue;
        }
        let package_name = package["name"]
            .as_str()
            .context("package name is missing")?
            .to_string();
        let dependencies = package["dependencies"]
            .as_array()
            .context("package dependencies is not an array")?;
        let mut source_names = HashMap::new();
        for dependency in dependencies {
            let actual = dependency["name"]
                .as_str()
                .context("dependency name is missing")?;
            let source = dependency["rename"]
                .as_str()
                .unwrap_or(actual)
                .replace('-', "_");
            source_names.insert(source, actual.to_string());
        }
        maps.insert(package_name, source_names);
    }
    Ok(maps)
}

fn cargo_targets_from_metadata(
    metadata: &serde_json::Value,
) -> Result<BTreeMap<String, CargoTargets>> {
    let workspace_members = metadata["workspace_members"]
        .as_array()
        .context("cargo metadata workspace_members is not an array")?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<HashSet<_>>();
    let packages = metadata["packages"]
        .as_array()
        .context("cargo metadata packages is not an array")?;
    let mut targets = BTreeMap::new();
    for package in packages {
        let id = package["id"].as_str().context("package id is missing")?;
        if !workspace_members.contains(id) && !package["source"].is_null() {
            continue;
        }
        let name = package["name"]
            .as_str()
            .context("package name is missing")?
            .to_string();
        let mut package_targets = CargoTargets::default();
        for target in package["targets"].as_array().into_iter().flatten() {
            let kinds = target["kind"]
                .as_array()
                .context("cargo target kind is not an array")?
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<HashSet<_>>();
            let source = PathBuf::from(
                target["src_path"]
                    .as_str()
                    .context("cargo target src_path is missing")?,
            );
            if kinds.contains("lib") || kinds.contains("bin") {
                package_targets.production.push(source);
            } else if kinds.contains("custom-build") {
                package_targets.custom_build.push(source);
            }
        }
        package_targets.production.sort();
        package_targets.production.dedup();
        package_targets.custom_build.sort();
        package_targets.custom_build.dedup();
        if targets.contains_key(&name) {
            bail!("PKG011 duplicate local/path package name {name} makes Cargo identity ambiguous");
        }
        targets.insert(name, package_targets);
    }
    Ok(targets)
}

fn cargo_diagnostics_from_metadata_with_helpers(
    root: &Path,
    runtimes: &[RuntimeSpec],
    metadata: &serde_json::Value,
    allowed_helpers: &WorkspaceHelperPolicy,
) -> Result<Vec<Diagnostic>> {
    let cargo_targets = cargo_targets_from_metadata(metadata)?;
    let workspace_members = metadata["workspace_members"]
        .as_array()
        .context("cargo metadata workspace_members is not an array")?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<HashSet<_>>();
    let packages = metadata["packages"]
        .as_array()
        .context("cargo metadata packages is not an array")?;

    let mut local_graph = BTreeMap::<String, BTreeSet<String>>::new();
    let mut direct_dependencies = BTreeMap::<String, BTreeSet<String>>::new();
    let mut dependency_sources = BTreeMap::<String, HashMap<String, String>>::new();
    let mut manifests = BTreeMap::<String, String>::new();
    let mut local_names = HashSet::new();
    for package in packages {
        let id = package["id"].as_str().context("package id is missing")?;
        if !workspace_members.contains(id) && !package["source"].is_null() {
            continue;
        }
        let name = package["name"]
            .as_str()
            .context("package name is missing")?
            .to_string();
        local_names.insert(name.clone());
        manifests.insert(
            name,
            package["manifest_path"]
                .as_str()
                .context("package manifest_path is missing")?
                .to_string(),
        );
    }
    for package in packages {
        let id = package["id"].as_str().context("package id is missing")?;
        if !workspace_members.contains(id) && !package["source"].is_null() {
            continue;
        }
        let name = package["name"]
            .as_str()
            .context("package name is missing")?
            .to_string();
        let dependencies = package["dependencies"]
            .as_array()
            .context("package dependencies is not an array")?;
        let graph_entry = local_graph.entry(name.clone()).or_default();
        let direct_entry = direct_dependencies.entry(name.clone()).or_default();
        let source_entry = dependency_sources.entry(name).or_default();
        for dependency in dependencies {
            if dependency["kind"]
                .as_str()
                .is_some_and(|kind| kind != "normal")
            {
                continue;
            }
            let dependency_name = dependency["name"]
                .as_str()
                .context("dependency name is missing")?;
            direct_entry.insert(dependency_name.to_string());
            let source_name = dependency["rename"]
                .as_str()
                .unwrap_or(dependency_name)
                .replace('-', "_");
            source_entry.insert(source_name, dependency_name.to_string());
            if local_names.contains(dependency_name) {
                graph_entry.insert(dependency_name.to_string());
            }
        }
    }

    let runtime_names = runtimes
        .iter()
        .map(|runtime| runtime.package.as_str())
        .collect::<HashSet<_>>();
    let mut diagnostics = Vec::new();
    let mut scanned_helpers = HashSet::new();
    for package in &local_names {
        if reviewed_external_package(package) {
            diagnostics.push(Diagnostic {
                code: "PKG005",
                path: manifests
                    .get(package)
                    .map(|path| normalize_path(Path::new(path)))
                    .unwrap_or_else(|| normalize_path(&root.join("Cargo.toml"))),
                line: 1,
                message: format!(
                    "workspace package {package} collides with a reviewed external dependency name"
                ),
            });
        }
    }
    for runtime in runtimes {
        let Some(manifest) = manifests.get(&runtime.package) else {
            diagnostics.push(Diagnostic {
                code: "PKG001",
                path: "Cargo.toml".into(),
                line: 1,
                message: format!("runtime package {} is missing", runtime.package),
            });
            continue;
        };
        let expected = normalize_path(&root.join(&runtime.root).join("Cargo.toml"));
        if normalize_path(Path::new(manifest)) != expected {
            diagnostics.push(Diagnostic {
                code: "PKG002",
                path: "Cargo.toml".into(),
                line: 1,
                message: format!(
                    "runtime {} manifest is {}, expected {}",
                    runtime.package, manifest, expected
                ),
            });
        }
        if let Some(targets) = cargo_targets.get(&runtime.package) {
            for build_script in &targets.custom_build {
                diagnostics.push(Diagnostic {
                    code: "PKG010",
                    path: normalize_path(build_script),
                    line: 1,
                    message: format!(
                        "runtime {} custom-build targets are outside the static source proof",
                        runtime.package
                    ),
                });
            }
        }

        let mut queue = VecDeque::from([runtime.package.clone()]);
        let mut seen = HashSet::new();
        let reviewed_helpers = allowed_helpers
            .get(&runtime.package)
            .cloned()
            .unwrap_or_default();
        while let Some(package) = queue.pop_front() {
            if !seen.insert(package.clone()) {
                continue;
            }
            if package != runtime.package {
                if !reviewed_helpers.contains(&package) {
                    diagnostics.push(Diagnostic {
                        code: "PKG006",
                        path: manifests
                            .get(&package)
                            .map(|path| normalize_path(Path::new(path)))
                            .unwrap_or_else(|| {
                                normalize_path(&root.join(&runtime.root).join("Cargo.toml"))
                            }),
                        line: 1,
                        message: format!(
                            "runtime {} reaches unreviewed local/path helper package {package}",
                            runtime.package
                        ),
                    });
                }
                if let Some(package_dependencies) = direct_dependencies.get(&package) {
                    for dependency in package_dependencies {
                        if helper_persistence_capability(dependency) {
                            diagnostics.push(Diagnostic {
                                code: "PKG007",
                                path: manifests
                                    .get(&package)
                                    .map(|path| normalize_path(Path::new(path)))
                                    .unwrap_or_else(|| {
                                        normalize_path(
                                            &root.join(&runtime.root).join("Cargo.toml"),
                                        )
                                    }),
                                line: 1,
                                message: format!(
                                    "runtime {} local helper {package} reaches persistence capability {dependency}",
                                    runtime.package
                                ),
                            });
                        }
                        if forbidden_outbound_transport(dependency) {
                            diagnostics.push(Diagnostic {
                                code: "PKG008",
                                path: manifests
                                    .get(&package)
                                    .map(|path| normalize_path(Path::new(path)))
                                    .unwrap_or_else(|| {
                                        normalize_path(
                                            &root.join(&runtime.root).join("Cargo.toml"),
                                        )
                                    }),
                                line: 1,
                                message: format!(
                                    "runtime {} local helper {package} reaches non-HTTP transport capability {dependency}",
                                    runtime.package
                                ),
                            });
                        }
                    }
                }
                if reviewed_helpers.contains(&package) && scanned_helpers.insert(package.clone()) {
                    let manifest = manifests
                        .get(&package)
                        .context("local helper manifest missing")?;
                    let manifest_path = Path::new(manifest);
                    if manifest_path.is_file() {
                        let helper_targets = cargo_targets.get(&package);
                        if let Some(helper_targets) = helper_targets {
                            for build_script in &helper_targets.custom_build {
                                diagnostics.push(Diagnostic {
                                    code: "PKG010",
                                    path: normalize_path(build_script),
                                    line: 1,
                                    message: format!(
                                        "allowed local helper {package} custom-build targets are outside the static source proof"
                                    ),
                                });
                            }
                            if helper_targets.production.is_empty() {
                                diagnostics.push(Diagnostic {
                                    code: "PKG009",
                                    path: normalize_path(manifest_path),
                                    line: 1,
                                    message: format!(
                                        "allowed local helper {package} has no auditable normal lib/bin target"
                                    ),
                                });
                            } else {
                                diagnostics.extend(audit_helper_sources(
                                    root,
                                    &package,
                                    &helper_targets.production,
                                    dependency_sources
                                        .get(&package)
                                        .cloned()
                                        .unwrap_or_default(),
                                )?);
                            }
                        } else {
                            diagnostics.push(Diagnostic {
                                code: "PKG009",
                                path: normalize_path(manifest_path),
                                line: 1,
                                message: format!(
                                    "allowed local helper {package} cargo targets cannot be audited"
                                ),
                            });
                        }
                    } else {
                        diagnostics.push(Diagnostic {
                            code: "PKG009",
                            path: normalize_path(manifest_path),
                            line: 1,
                            message: format!(
                                "allowed local helper {package} source cannot be audited"
                            ),
                        });
                    }
                }
            }
            for dependency in local_graph.get(&package).into_iter().flatten() {
                if dependency != &runtime.package && runtime_names.contains(dependency.as_str()) {
                    diagnostics.push(Diagnostic {
                        code: "PKG003",
                        path: normalize_path(&root.join(&runtime.root).join("Cargo.toml")),
                        line: 1,
                        message: format!(
                            "runtime {} reaches service package {} through the Cargo graph",
                            runtime.package, dependency
                        ),
                    });
                } else {
                    queue.push_back(dependency.clone());
                }
            }
        }
    }

    let mut queue = VecDeque::from(["gateway".to_string()]);
    let mut seen = HashSet::new();
    while let Some(package) = queue.pop_front() {
        if !seen.insert(package.clone()) {
            continue;
        }
        if let Some(package_dependencies) = direct_dependencies.get(&package) {
            for forbidden in ["sqlx", "redis", "deadpool-redis", "aws-sdk-s3"] {
                if !package_dependencies.contains(forbidden) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    code: "PKG004",
                    path: "gateway/Cargo.toml".into(),
                    line: 1,
                    message: format!(
                        "gateway reaches persistence client {forbidden} through workspace package {package}"
                    ),
                });
            }
        }
        queue.extend(local_graph.get(&package).into_iter().flatten().cloned());
    }
    Ok(diagnostics)
}

fn audit_helper_sources(
    root: &Path,
    package: &str,
    targets: &[PathBuf],
    dependency_packages: HashMap<String, String>,
) -> Result<Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let targets = targets
        .iter()
        .map(|target| {
            target
                .canonicalize()
                .with_context(|| format!("canonicalize cargo target {}", target.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    for (path, test_only) in module_closure_from_entries(&targets)? {
        if test_only {
            continue;
        }
        let relative = normalize_path(path.strip_prefix(root).unwrap_or(&path));
        let source = fs::read_to_string(&path)
            .with_context(|| format!("read allowed helper Rust source {relative}"))?;
        let syntax = syn::parse_file(&source)
            .with_context(|| format!("parse allowed helper Rust source {relative}"))?;
        diagnostics.extend(audit_helper_syntax(
            package,
            &relative,
            &syntax,
            &dependency_packages,
        ));
    }
    Ok(diagnostics)
}

fn audit_helper_syntax(
    package: &str,
    relative: &str,
    syntax: &File,
    dependency_packages: &HashMap<String, String>,
) -> Vec<Diagnostic> {
    let aliases = collect_internal_aliases_with_dependencies(syntax, dependency_packages);
    let mut diagnostics = aliases
        .issues
        .iter()
        .map(|(span, message)| Diagnostic {
            code: "PKG009",
            path: relative.into(),
            line: span.start().line.max(1),
            message: format!("allowed local helper {package}: {message}"),
        })
        .collect::<Vec<_>>();
    let mut analyzer = HelperSourceAnalyzer {
        package,
        relative,
        dependency_packages,
        aliases,
        diagnostics: &mut diagnostics,
        production: true,
    };
    analyzer.visit_file(syntax);
    diagnostics
}

struct HelperSourceAnalyzer<'a> {
    package: &'a str,
    relative: &'a str,
    dependency_packages: &'a HashMap<String, String>,
    aliases: InternalAliases,
    diagnostics: &'a mut Vec<Diagnostic>,
    production: bool,
}

impl HelperSourceAnalyzer<'_> {
    fn push(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            code: "PKG009",
            path: self.relative.into(),
            line: span.start().line.max(1),
            message: format!("allowed local helper {}: {}", self.package, message.into()),
        });
    }

    fn record_path(&mut self, path: &SynPath) {
        if !self.production {
            return;
        }
        let segments = expand_internal_alias(&path_segments(path), &self.aliases.paths);
        let Some(root) = segments.first().map(String::as_str) else {
            return;
        };
        let package = self
            .dependency_packages
            .get(root)
            .map(String::as_str)
            .unwrap_or(root);
        if helper_persistence_capability(package) {
            self.push(
                path.span(),
                format!("persistence capability {package} is forbidden in shared helper source"),
            );
        }
        if forbidden_outbound_transport(package) {
            self.push(
                path.span(),
                format!(
                    "non-reqwest transport capability {package} is forbidden in shared helper source"
                ),
            );
        }
        let second = segments.get(1).map(String::as_str);
        if matches!(
            (package, second),
            ("std", Some("fs" | "net" | "process"))
                | ("tokio", Some("fs" | "net" | "process" | "signal"))
        ) {
            self.push(
                path.span(),
                format!(
                    "process/filesystem/raw-network capability {} is forbidden in shared helper source",
                    segments.iter().take(2).cloned().collect::<Vec<_>>().join("::")
                ),
            );
        }
    }

    fn unsafe_forbidden(&mut self, span: Span, capability: &str) {
        if self.production {
            self.push(
                span,
                format!("{capability} is forbidden in shared helper source"),
            );
        }
    }
}

impl<'ast> Visit<'ast> for HelperSourceAnalyzer<'_> {
    fn visit_path(&mut self, path: &'ast SynPath) {
        self.record_path(path);
        visit::visit_path(self, path);
    }

    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        if !self.production || item_is_test(&item.attrs) {
            return;
        }
        let mut paths = Vec::new();
        flatten_use(&item.tree, Vec::new(), &mut paths);
        for segments in paths {
            if let Ok(path) = syn::parse_str::<SynPath>(&segments.join("::")) {
                self.record_path(&path);
            }
        }
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        let previous = self.production;
        if item_is_test(&item.attrs) {
            self.production = false;
        }
        visit::visit_item_mod(self, item);
        self.production = previous;
    }

    fn visit_item_macro(&mut self, item: &'ast ItemMacro) {
        let previous = self.production;
        if item_is_test(&item.attrs) {
            self.production = false;
        }
        if self.production && item.ident.is_some() && item.mac.path.is_ident("macro_rules") {
            self.push(
                item.span(),
                "local macro_rules definitions are outside the shared helper source proof",
            );
        }
        visit::visit_item_macro(self, item);
        self.production = previous;
    }

    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        let previous = self.production;
        if item_is_test(&item.attrs) {
            self.production = false;
        }
        if item.sig.unsafety.is_some() || item.sig.abi.is_some() {
            self.unsafe_forbidden(item.span(), "unsafe or foreign-ABI function");
        }
        visit::visit_item_fn(self, item);
        self.production = previous;
    }

    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        let previous = self.production;
        if item_is_test(&item.attrs) {
            self.production = false;
        }
        if item.sig.unsafety.is_some() || item.sig.abi.is_some() {
            self.unsafe_forbidden(item.span(), "unsafe or foreign-ABI method");
        }
        visit::visit_impl_item_fn(self, item);
        self.production = previous;
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        if item.sig.unsafety.is_some() || item.sig.abi.is_some() {
            self.unsafe_forbidden(item.span(), "unsafe or foreign-ABI trait method");
        }
        visit::visit_trait_item_fn(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if item.unsafety.is_some() {
            self.unsafe_forbidden(item.span(), "unsafe impl");
        }
        visit::visit_item_impl(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        if item.unsafety.is_some() {
            self.unsafe_forbidden(item.span(), "unsafe trait");
        }
        visit::visit_item_trait(self, item);
    }

    fn visit_expr_unsafe(&mut self, expression: &'ast syn::ExprUnsafe) {
        self.unsafe_forbidden(expression.span(), "unsafe block");
        visit::visit_expr_unsafe(self, expression);
    }

    fn visit_item_foreign_mod(&mut self, item: &'ast syn::ItemForeignMod) {
        if !item_is_test(&item.attrs) {
            self.unsafe_forbidden(item.span(), "foreign ABI declaration");
        }
        visit::visit_item_foreign_mod(self, item);
    }

    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if self.production && attribute.path().is_ident("path") {
            self.push(
                attribute.span(),
                "#[path] source redirection is forbidden in shared helper source",
            );
        }
        if self.production && attribute.path().is_ident("derive") {
            if let Ok(paths) =
                attribute.parse_args_with(Punctuated::<SynPath, Token![,]>::parse_terminated)
            {
                for path in paths {
                    self.record_path(&path);
                }
            }
        }
        if self.production
            && attribute.path().is_ident("cfg_attr")
            && (tokens_contain_ident(
                attribute
                    .meta
                    .require_list()
                    .map_or(&TokenStream::new(), |list| &list.tokens),
                "path",
            ) || self
                .dependency_packages
                .iter()
                .filter(|(_, package)| helper_persistence_capability(package))
                .any(|(source, _)| {
                    attribute
                        .meta
                        .require_list()
                        .is_ok_and(|list| tokens_contain_ident(&list.tokens, source))
                }))
        {
            self.push(
                attribute.span(),
                "cfg_attr can enable forbidden helper source capabilities",
            );
        }
        visit::visit_attribute(self, attribute);
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        self.record_path(&mac.path);
        if self.production
            && mac.path.segments.last().is_some_and(|segment| {
                matches!(
                    segment.ident.to_string().as_str(),
                    "asm" | "global_asm" | "include"
                )
            })
        {
            self.push(
                mac.span(),
                "inline assembly and include! are forbidden in shared helper source",
            );
        }
        visit::visit_macro(self, mac);
    }
}

pub fn validate_runtime_hooks(audit: &SourceAudit, runtimes: &[RuntimeSpec]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for runtime in runtimes {
        let Some(facts) = audit.runtime_facts.get(&runtime.package) else {
            diagnostics.push(runtime_diagnostic(
                runtime,
                "RUNTIME001",
                "runtime has no audited Rust sources",
            ));
            continue;
        };
        for key in &runtime.required_env {
            if !facts.env_keys.contains(key) {
                diagnostics.push(runtime_diagnostic(
                    runtime,
                    "RUNTIME002",
                    &format!("required configuration {key} is not read from the environment"),
                ));
            }
        }
        let liveness = facts.routes.get(&runtime.liveness_path);
        let readiness = facts.routes.get(&runtime.readiness_path);
        if liveness.is_none() {
            diagnostics.push(runtime_diagnostic(
                runtime,
                "RUNTIME003",
                &format!("missing liveness route {}", runtime.liveness_path),
            ));
        }
        if readiness.is_none() {
            diagnostics.push(runtime_diagnostic(
                runtime,
                "RUNTIME004",
                &format!("missing readiness route {}", runtime.readiness_path),
            ));
        }
        if liveness.is_some() && liveness == readiness {
            diagnostics.push(runtime_diagnostic(
                runtime,
                "RUNTIME005",
                "liveness and readiness must use different handlers",
            ));
        }
        if !facts.routes.contains_key("/metrics")
            || !facts
                .paths
                .iter()
                .any(|path| path.ends_with("init_metrics") || path.ends_with("install_metrics"))
        {
            diagnostics.push(runtime_diagnostic(
                runtime,
                "RUNTIME006",
                "metrics must be installed and exposed at /metrics",
            ));
        }
        if !facts.methods.contains("json")
            || !facts.methods.contains("init")
            || !facts.paths.iter().any(|path| path.ends_with("info_span"))
            || !facts
                .strings
                .iter()
                .any(|value| value.eq_ignore_ascii_case("x-trace-id"))
        {
            diagnostics.push(runtime_diagnostic(
                runtime,
                "RUNTIME007",
                "structured JSON tracing with request trace propagation is required",
            ));
        }
        if !facts.methods.contains("with_graceful_shutdown")
            || !facts.paths.iter().any(|path| path.ends_with("ctrl_c"))
            || !facts.paths.iter().any(|path| path.ends_with("terminate"))
        {
            diagnostics.push(runtime_diagnostic(
                runtime,
                "RUNTIME008",
                "Axum graceful shutdown must handle Ctrl-C and SIGTERM",
            ));
        }

        if liveness.is_some_and(|name| match facts.functions.get(name) {
            Some(handler) => !handler
                .paths
                .iter()
                .any(|path| path.ends_with("StatusCode.OK")),
            None => true,
        }) {
            diagnostics.push(runtime_diagnostic(
                runtime,
                "RUNTIME009",
                "liveness handler must have a same-file process-local OK response",
            ));
        }
        if readiness.is_some_and(|name| match facts.functions.get(name) {
            Some(handler) => !handler
                .paths
                .iter()
                .any(|path| path.ends_with("StatusCode.SERVICE_UNAVAILABLE")),
            None => true,
        }) {
            diagnostics.push(runtime_diagnostic(
                runtime,
                "RUNTIME010",
                "readiness handler must expose a same-file unavailable response",
            ));
        }
    }
    diagnostics
}

fn runtime_diagnostic(runtime: &RuntimeSpec, code: &'static str, message: &str) -> Diagnostic {
    Diagnostic {
        code,
        path: format!("{}/src/main.rs", runtime.root),
        line: 1,
        message: message.into(),
    }
}

struct Analyzer<'a> {
    service: &'a str,
    relative: &'a str,
    module: Vec<String>,
    layer: Option<Layer>,
    diagnostics: &'a mut Vec<Diagnostic>,
    sql_calls: &'a mut Vec<SqlCall>,
    explicit_references: &'a mut usize,
    facts: &'a mut RuntimeFacts,
    constants: HashMap<String, Vec<String>>,
    sql_aliases: SqlAliases,
    internal_aliases: InternalAliases,
    local_names: HashSet<String>,
    dependency_packages: HashMap<String, String>,
    scopes: Vec<HashMap<String, Vec<String>>>,
    query_scopes: Vec<HashSet<String>>,
    current_function: String,
    facts_enabled: bool,
    in_use: bool,
    composition_root: bool,
    saw_reqwest: bool,
    saw_internal_http_marker: bool,
}

impl Analyzer<'_> {
    fn push(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            code,
            path: self.relative.to_string(),
            line: span.start().line.max(1),
            message: message.into(),
        });
    }

    fn runtime_function_key(&self, function: &str) -> String {
        format!("{}::{}::{function}", self.relative, self.module.join("::"))
    }

    fn record_path(&mut self, path: &SynPath) {
        let raw_segments = path_segments(path);
        if raw_segments.is_empty() {
            return;
        }
        let segments = expand_internal_alias(&raw_segments, &self.internal_aliases.paths);
        *self.explicit_references += 1;
        let rendered = segments.join(".");
        if self.facts_enabled {
            let function_key = self.runtime_function_key(&self.current_function);
            self.facts.paths.insert(rendered.clone());
            self.facts
                .functions
                .entry(function_key)
                .or_default()
                .paths
                .insert(rendered.clone());
        }

        let root = segments[0].as_str();
        let actual_package = self
            .dependency_packages
            .get(root)
            .map(String::as_str)
            .unwrap_or(root)
            .to_string();
        if actual_package == "reqwest" {
            self.saw_reqwest = true;
        }
        if actual_package == "tonic" {
            self.push(
                "HTTP002",
                path.span(),
                "runtime services communicate with each other over HTTP, not tonic",
            );
        }
        if self.service != "gateway"
            && self.facts_enabled
            && forbidden_outbound_transport(&actual_package)
        {
            self.push(
                "HTTP004",
                path.span(),
                format!(
                    "outbound transport capability {actual_package} is outside the reviewed reqwest HTTP adapter contract"
                ),
            );
        }
        if self.service != "gateway"
            && self.facts_enabled
            && raw_network_capability(&segments)
            && !address_only_network_path(&segments)
            && !self.inbound_listener_path(&segments)
        {
            self.push(
                "HTTP004",
                path.span(),
                format!(
                    "raw network capability {} is outside the reviewed reqwest HTTP adapter contract",
                    segments.iter().take(3).cloned().collect::<Vec<_>>().join("::")
                ),
            );
        }
        if segments.iter().any(|segment| segment == "QueryBuilder") && actual_package == "sqlx" {
            self.push(
                "SQL002",
                path.span(),
                "sqlx::QueryBuilder is outside the statically auditable SQL contract",
            );
        }
        if unsupported_database_client(&actual_package) {
            self.push(
                "SQL007",
                path.span(),
                format!(
                    "database client {actual_package} is outside the reviewed sqlx ownership contract"
                ),
            );
        }

        if self.service == "gateway" && helper_persistence_capability(&actual_package) {
            self.push(
                "LAYER004",
                path.span(),
                format!("gateway must not access persistence capability {actual_package}"),
            );
        }

        if let Some(layer) = self.layer {
            if let Some(destination) = resolve_internal_layer(&segments, &self.module) {
                if !layer.allows(destination) {
                    self.push(
                        "LAYER001",
                        path.span(),
                        format!(
                            "{} cannot depend on {}",
                            layer.as_str(),
                            destination.as_str()
                        ),
                    );
                }
            }
            if let Some(message) = layer.dependency_violation(
                &segments,
                self.in_use,
                &self.local_names,
                &self.dependency_packages,
                !self.facts_enabled,
            ) {
                self.push("LAYER002", path.span(), message);
            }
        }
    }

    fn inbound_listener_path(&self, segments: &[String]) -> bool {
        self.composition_root
            && self.relative.ends_with("/src/main.rs")
            && segments
                .get(2)
                .is_some_and(|segment| segment == "TcpListener")
    }

    fn record_sql(&mut self, expression: &Expr, span: Span) {
        if self.facts_enabled && !self.relative.contains("/src/infrastructure/persistence/") {
            self.push(
                "SQL008",
                span,
                "production SQL must live under infrastructure/persistence",
            );
        }
        if self.composition_root {
            self.push(
                "LAYER009",
                span,
                "service composition roots must not access SQL",
            );
        }
        match self.resolve_strings(expression) {
            Some(values) if !values.is_empty() => {
                for sql in values {
                    self.sql_calls.push(SqlCall {
                        service: self.service.into(),
                        path: self.relative.into(),
                        line: span.start().line.max(1),
                        function: self.current_function.clone(),
                        sql,
                    });
                }
            }
            _ => self.push(
                "SQL001",
                span,
                "sqlx queries must resolve to a closed set of string literals",
            ),
        }
    }

    fn resolve_strings(&self, expression: &Expr) -> Option<Vec<String>> {
        let values = match expression {
            Expr::Lit(value) => match &value.lit {
                syn::Lit::Str(value) => vec![value.value()],
                _ => return None,
            },
            Expr::Paren(value) => return self.resolve_strings(&value.expr),
            Expr::Group(value) => return self.resolve_strings(&value.expr),
            Expr::Reference(value) => return self.resolve_strings(&value.expr),
            Expr::Block(value) => {
                return block_value(&value.block).and_then(|value| self.resolve_strings(value));
            }
            Expr::Path(value) => {
                let name = value.path.segments.last()?.ident.to_string();
                self.scopes
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(&name).cloned())
                    .or_else(|| self.constants.get(&name).cloned())?
            }
            Expr::If(value) => {
                let mut values = block_value(&value.then_branch)
                    .and_then(|expression| self.resolve_strings(expression))?;
                let (_, otherwise) = value.else_branch.as_ref()?;
                values.extend(self.resolve_strings(otherwise)?);
                values
            }
            Expr::Match(value) => {
                let mut values = Vec::new();
                for arm in &value.arms {
                    values.extend(self.resolve_strings(&arm.body)?);
                }
                values
            }
            Expr::Macro(value) if value.mac.path.is_ident("concat") => {
                let expressions = Punctuated::<Expr, Token![,]>::parse_terminated
                    .parse2(value.mac.tokens.clone())
                    .ok()?;
                let mut products = vec![String::new()];
                for expression in expressions {
                    let fragments = self.resolve_strings(&expression)?;
                    let mut next = Vec::new();
                    for prefix in &products {
                        for fragment in &fragments {
                            next.push(format!("{prefix}{fragment}"));
                            if next.len() > 32 {
                                return None;
                            }
                        }
                    }
                    products = next;
                }
                products
            }
            _ => return None,
        };
        let unique = values.into_iter().collect::<BTreeSet<_>>();
        (unique.len() <= 32).then(|| unique.into_iter().collect())
    }

    fn record_string(&mut self, value: &str) {
        if self.facts_enabled {
            self.facts.strings.insert(value.to_string());
        }
        if value.eq_ignore_ascii_case("X-Internal-Service-Token")
            || value.ends_with("_SERVICE_URL")
            || value.contains("/internal/")
        {
            self.saw_internal_http_marker = true;
        }
    }

    fn audit_cfg_attr(&mut self, attribute: &Attribute) {
        let syn::Meta::List(list) = &attribute.meta else {
            self.push(
                "LAYER011",
                attribute.span(),
                "cfg_attr arguments are outside the static architecture proof",
            );
            return;
        };
        self.audit_cfg_attr_tokens(&list.tokens, attribute.span());
    }

    fn audit_cfg_attr_tokens(&mut self, tokens: &TokenStream, span: Span) {
        let Ok(arguments) =
            Punctuated::<syn::Meta, Token![,]>::parse_terminated.parse2(tokens.clone())
        else {
            self.push(
                "LAYER011",
                span,
                "cfg_attr arguments are outside the static architecture proof",
            );
            return;
        };
        if arguments.len() < 2 {
            self.push(
                "LAYER011",
                span,
                "cfg_attr must include a condition and an auditable nested attribute",
            );
            return;
        }
        for nested in arguments.iter().skip(1) {
            self.audit_cfg_attr_meta(nested);
        }
    }

    fn audit_cfg_attr_meta(&mut self, meta: &syn::Meta) {
        match meta {
            syn::Meta::List(list) if list.path.is_ident("derive") => {
                match Punctuated::<SynPath, Token![,]>::parse_terminated.parse2(list.tokens.clone())
                {
                    Ok(paths) => {
                        for path in paths {
                            self.record_path(&path);
                        }
                    }
                    Err(_) => self.push(
                        "LAYER011",
                        list.span(),
                        "cfg_attr derive paths are not statically analyzable",
                    ),
                }
            }
            syn::Meta::List(list) if list.path.is_ident("cfg_attr") => {
                self.audit_cfg_attr_tokens(&list.tokens, list.span());
            }
            syn::Meta::NameValue(value) if value.path.is_ident("path") => self.push(
                "LAYER003",
                value.span(),
                "cfg_attr path can bypass physical layer ownership and is forbidden",
            ),
            syn::Meta::Path(path)
                if path.is_ident("test")
                    || path.is_ident("inline")
                    || path.is_ident("cold")
                    || path.is_ident("must_use")
                    || path.is_ident("deprecated")
                    || path.is_ident("no_mangle") => {}
            syn::Meta::List(list)
                if list.path.is_ident("allow")
                    || list.path.is_ident("warn")
                    || list.path.is_ident("deny")
                    || list.path.is_ident("forbid")
                    || list.path.is_ident("repr")
                    || list.path.is_ident("doc") => {}
            syn::Meta::NameValue(value) if value.path.is_ident("doc") => {}
            other => {
                let path = other.path();
                if path.segments.len() > 1 {
                    self.record_path(path);
                }
                self.push(
                    "LAYER011",
                    other.span(),
                    "cfg_attr nested attribute is not in the statically reviewed set",
                );
            }
        }
    }

    fn finish_file(&mut self) {
        let service_http_adapter = self.relative.contains("/src/infrastructure/http/");
        if self.saw_reqwest
            && self.saw_internal_http_marker
            && self.service != "gateway"
            && !service_http_adapter
        {
            self.push(
                "HTTP001",
                Span::call_site(),
                "service-to-service reqwest adapters must live under infrastructure/http",
            );
        }
        let approved_non_service_adapter = (self.service == "novel-service"
            && (self.relative.contains("/src/infrastructure/llm/")
                || self
                    .relative
                    .ends_with("/src/infrastructure/object_storage.rs")))
            || (self.service == "user-service"
                && self.relative.ends_with("/src/infrastructure/llm_usage.rs"));
        if self.saw_reqwest
            && self.service != "gateway"
            && !service_http_adapter
            && !approved_non_service_adapter
        {
            self.push(
                "HTTP003",
                Span::call_site(),
                "reqwest is only allowed in infrastructure/http or an explicitly reviewed non-service adapter",
            );
        }
    }

    fn sql_api_for_path(&self, path: &SynPath) -> Option<SqlApi> {
        let segments = path_segments(path);
        let canonical = if segments.len() == 1 {
            self.sql_aliases
                .functions
                .get(&segments[0])
                .map(String::as_str)
        } else if segments
            .first()
            .is_some_and(|root| self.sql_aliases.crate_names.contains(root))
        {
            segments.last().map(String::as_str)
        } else {
            None
        }?;
        sql_api(canonical)
    }

    fn executor_api_for_path(&self, path: &SynPath) -> bool {
        let segments = path_segments(path);
        if segments.len() == 1 {
            return self
                .sql_aliases
                .executor_functions
                .get(&segments[0])
                .is_some_and(|name| executor_api(name));
        }
        let Some(method) = segments.last() else {
            return false;
        };
        if !executor_api(method) {
            return false;
        }
        let direct = segments
            .first()
            .is_some_and(|root| self.sql_aliases.crate_names.contains(root))
            && segments
                .get(segments.len() - 2)
                .is_some_and(|name| name == "Executor");
        let aliased = segments.len() == 2
            && segments
                .first()
                .is_some_and(|root| self.sql_aliases.executor_names.contains(root));
        direct || aliased
    }

    fn expression_is_sql_query(&self, expression: &Expr) -> bool {
        match expression {
            Expr::Call(call) => match &*call.func {
                Expr::Path(function) => self.sql_api_for_path(&function.path).is_some(),
                _ => false,
            },
            Expr::Macro(expression) => self.sql_api_for_path(&expression.mac.path).is_some(),
            Expr::MethodCall(call) => self.expression_is_sql_query(&call.receiver),
            Expr::Paren(expression) => self.expression_is_sql_query(&expression.expr),
            Expr::Group(expression) => self.expression_is_sql_query(&expression.expr),
            Expr::Reference(expression) => self.expression_is_sql_query(&expression.expr),
            Expr::Await(expression) => self.expression_is_sql_query(&expression.base),
            Expr::Try(expression) => self.expression_is_sql_query(&expression.expr),
            Expr::Path(expression) => expression.path.segments.last().is_some_and(|segment| {
                let name = segment.ident.to_string();
                self.query_scopes
                    .iter()
                    .rev()
                    .any(|scope| scope.contains(&name))
            }),
            _ => false,
        }
    }

    fn tokens_contain_sql_symbol(&self, tokens: &TokenStream) -> bool {
        self.sql_aliases
            .crate_names
            .iter()
            .chain(self.sql_aliases.functions.keys())
            .any(|name| tokens_contain_ident(tokens, name))
    }

    fn record_sql_api<'a>(
        &mut self,
        api: SqlApi,
        mut arguments: impl Iterator<Item = &'a Expr>,
        macro_call: bool,
        span: Span,
    ) {
        match api {
            SqlApi::Literal {
                function_index,
                macro_index,
            } => {
                let index = if macro_call {
                    macro_index
                } else {
                    function_index
                };
                if let Some(sql) = arguments.nth(index) {
                    self.record_sql(sql, span);
                } else {
                    self.push("SQL001", span, "sqlx call is missing inline SQL");
                }
            }
            SqlApi::Unsupported => {
                if self.composition_root {
                    self.push(
                        "LAYER009",
                        span,
                        "service composition roots must not access SQL",
                    );
                }
                self.push(
                    "SQL005",
                    span,
                    "sqlx query_file and unchecked variants are outside the inline SQL ownership proof",
                );
            }
        }
    }
}

impl<'ast> Visit<'ast> for Analyzer<'_> {
    fn visit_path(&mut self, path: &'ast SynPath) {
        self.record_path(path);
        visit::visit_path(self, path);
    }

    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        let previous = self.in_use;
        self.in_use = true;
        let mut paths = Vec::new();
        flatten_use(&item.tree, Vec::new(), &mut paths);
        for segments in paths {
            if let Ok(path) = syn::parse_str::<SynPath>(&segments.join("::")) {
                self.record_path(&path);
            }
        }
        self.in_use = previous;
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        if self.facts_enabled && !item_is_test(&item.attrs) {
            self.push(
                "LAYER012",
                item.span(),
                "production extern crate declarations obscure actual package capabilities and are forbidden",
            );
        }
        visit::visit_item_extern_crate(self, item);
    }

    fn visit_item_foreign_mod(&mut self, item: &'ast syn::ItemForeignMod) {
        if self.facts_enabled
            && matches!(
                self.layer,
                Some(Layer::Domain | Layer::Application | Layer::Interface)
            )
        {
            self.push(
                "LAYER013",
                item.span(),
                "foreign ABI declarations are only allowed in infrastructure adapters",
            );
        }
        visit::visit_item_foreign_mod(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if self.facts_enabled
            && item.unsafety.is_some()
            && matches!(
                self.layer,
                Some(Layer::Domain | Layer::Application | Layer::Interface)
            )
        {
            self.push(
                "LAYER013",
                item.span(),
                "unsafe impls are only allowed in infrastructure adapters",
            );
        }
        visit::visit_item_impl(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        if self.facts_enabled
            && item.unsafety.is_some()
            && matches!(
                self.layer,
                Some(Layer::Domain | Layer::Application | Layer::Interface)
            )
        {
            self.push(
                "LAYER013",
                item.span(),
                "unsafe traits are only allowed in infrastructure adapters",
            );
        }
        visit::visit_item_trait(self, item);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        if self.facts_enabled
            && (item.sig.unsafety.is_some() || item.sig.abi.is_some())
            && matches!(
                self.layer,
                Some(Layer::Domain | Layer::Application | Layer::Interface)
            )
        {
            self.push(
                "LAYER013",
                item.span(),
                "unsafe or foreign-ABI trait methods are only allowed in infrastructure adapters",
            );
        }
        visit::visit_trait_item_fn(self, item);
    }

    fn visit_expr_unsafe(&mut self, expression: &'ast syn::ExprUnsafe) {
        if self.facts_enabled
            && matches!(
                self.layer,
                Some(Layer::Domain | Layer::Application | Layer::Interface)
            )
        {
            self.push(
                "LAYER013",
                expression.span(),
                "unsafe blocks are only allowed in infrastructure adapters",
            );
        }
        visit::visit_expr_unsafe(self, expression);
    }

    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if attribute.path().is_ident("path") {
            self.push(
                "LAYER003",
                attribute.span(),
                "#[path] can bypass physical layer ownership and is forbidden",
            );
        }
        if attribute.path().is_ident("derive") {
            if let Ok(paths) =
                attribute.parse_args_with(Punctuated::<SynPath, Token![,]>::parse_terminated)
            {
                for path in paths {
                    self.record_path(&path);
                }
            }
        }
        if attribute.path().is_ident("cfg_attr") {
            self.audit_cfg_attr(attribute);
        }
        visit::visit_attribute(self, attribute);
    }

    fn visit_item_macro(&mut self, item: &'ast ItemMacro) {
        if item.ident.is_some() && item.mac.path.is_ident("macro_rules") {
            self.push(
                "LAYER005",
                item.span(),
                "local macro_rules definitions are outside the static module proof",
            );
        }
        visit::visit_item_macro(self, item);
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        let previous_facts = self.facts_enabled;
        let previous_module = self.module.clone();
        let previous_layer = self.layer;
        if item_is_test(&item.attrs) {
            self.facts_enabled = false;
        }
        if item.content.is_some() {
            self.module.push(item.ident.to_string());
            self.layer = self
                .module
                .first()
                .and_then(|segment| Layer::from_name(segment));
        }
        visit::visit_item_mod(self, item);
        self.facts_enabled = previous_facts;
        self.module = previous_module;
        self.layer = previous_layer;
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        self.record_path(&mac.path);
        if self.facts_enabled
            && mac.path.segments.last().is_some_and(|segment| {
                matches!(segment.ident.to_string().as_str(), "asm" | "global_asm")
            })
            && matches!(
                self.layer,
                Some(Layer::Domain | Layer::Application | Layer::Interface)
            )
        {
            self.push(
                "LAYER013",
                mac.span(),
                "inline assembly is only allowed in infrastructure adapters",
            );
        }
        if let Some(api) = self.sql_api_for_path(&mac.path) {
            match Punctuated::<Expr, Token![,]>::parse_terminated.parse2(mac.tokens.clone()) {
                Ok(arguments) => self.record_sql_api(api, arguments.iter(), true, mac.span()),
                Err(_) => self.push(
                    "SQL001",
                    mac.span(),
                    "sqlx macro arguments are not statically analyzable",
                ),
            }
        }
        if mac.path.is_ident("include") {
            self.push(
                "LAYER006",
                mac.span(),
                "include! generated Rust is outside the static module proof",
            );
        }
        if mac.path.is_ident("format") {
            if let Ok(arguments) =
                Punctuated::<Expr, Token![,]>::parse_terminated.parse2(mac.tokens.clone())
            {
                for argument in arguments {
                    self.visit_expr(&argument);
                }
            }
        } else if mac.path.is_ident("matches") {
            match first_macro_expression(&mac.tokens) {
                Some(expression) => self.visit_expr(&expression),
                None if self.tokens_contain_sql_symbol(&mac.tokens) => self.push(
                    "SQL004",
                    mac.span(),
                    "matches! contains a sqlx symbol that cannot be parsed as a Rust expression",
                ),
                None => {}
            }
        } else if self.tokens_contain_sql_symbol(&mac.tokens) {
            let block_tokens = TokenStream::from(TokenTree::Group(Group::new(
                Delimiter::Brace,
                mac.tokens.clone(),
            )));
            match syn::parse2::<Block>(block_tokens) {
                Ok(block) => self.visit_block(&block),
                Err(_) => self.push(
                    "SQL004",
                    mac.span(),
                    "macro contains hidden sqlx symbols outside the static SQL proof",
                ),
            }
        }
        visit::visit_macro(self, mac);
    }

    fn visit_lit_str(&mut self, literal: &'ast syn::LitStr) {
        self.record_string(&literal.value());
    }

    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        let previous = std::mem::replace(&mut self.current_function, item.sig.ident.to_string());
        let previous_facts = self.facts_enabled;
        if item_is_test(&item.attrs) {
            self.facts_enabled = false;
        }
        if self.facts_enabled
            && (item.sig.unsafety.is_some() || item.sig.abi.is_some())
            && matches!(
                self.layer,
                Some(Layer::Domain | Layer::Application | Layer::Interface)
            )
        {
            self.push(
                "LAYER013",
                item.span(),
                "unsafe or foreign-ABI functions are only allowed in infrastructure adapters",
            );
        }
        if self.facts_enabled {
            let function_key = self.runtime_function_key(&self.current_function);
            self.facts.functions.entry(function_key).or_default();
        }
        visit::visit_item_fn(self, item);
        self.facts_enabled = previous_facts;
        self.current_function = previous;
    }

    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        let previous = std::mem::replace(&mut self.current_function, item.sig.ident.to_string());
        let previous_facts = self.facts_enabled;
        if item_is_test(&item.attrs) {
            self.facts_enabled = false;
        }
        if self.facts_enabled
            && (item.sig.unsafety.is_some() || item.sig.abi.is_some())
            && matches!(
                self.layer,
                Some(Layer::Domain | Layer::Application | Layer::Interface)
            )
        {
            self.push(
                "LAYER013",
                item.span(),
                "unsafe or foreign-ABI methods are only allowed in infrastructure adapters",
            );
        }
        if self.facts_enabled {
            let function_key = self.runtime_function_key(&self.current_function);
            self.facts.functions.entry(function_key).or_default();
        }
        visit::visit_impl_item_fn(self, item);
        self.facts_enabled = previous_facts;
        self.current_function = previous;
    }

    fn visit_block(&mut self, block: &'ast Block) {
        self.scopes.push(HashMap::new());
        self.query_scopes.push(HashSet::new());
        for statement in &block.stmts {
            self.visit_stmt(statement);
            if let Stmt::Local(local) = statement {
                self.remember_local(local);
            }
        }
        self.scopes.pop();
        self.query_scopes.pop();
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(function) = &*call.func {
            let segments = path_segments(&function.path);
            if segments.ends_with(&["std".into(), "env".into(), "var".into()])
                || segments.ends_with(&["env".into(), "var".into()])
            {
                if let Some(Expr::Lit(value)) = call.args.first() {
                    if let syn::Lit::Str(value) = &value.lit {
                        if self.facts_enabled {
                            self.facts.env_keys.insert(value.value());
                        }
                    }
                }
            }
            if self.executor_api_for_path(&function.path) {
                if let Some(sql) = call.args.iter().nth(1) {
                    self.record_sql(sql, call.span());
                } else {
                    self.push("SQL001", call.span(), "sqlx Executor call is missing SQL");
                }
            } else if let Some(api) = self.sql_api_for_path(&function.path) {
                self.record_sql_api(api, call.args.iter(), false, call.span());
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        let method = call.method.to_string();
        if self.facts_enabled {
            self.facts.methods.insert(method.clone());
        }
        if self.facts_enabled && method == "route" {
            if let (Some(Expr::Lit(path)), Some(handler)) =
                (call.args.first(), call.args.iter().nth(1))
            {
                if let syn::Lit::Str(path) = &path.lit {
                    if let Some(handler) = route_handler(handler) {
                        let handler_key = self.runtime_function_key(&handler);
                        self.facts.routes.insert(path.value(), handler_key);
                    }
                }
            }
        }
        if self.sql_aliases.executor_imported
            && executor_api(&method)
            && !self.expression_is_sql_query(&call.receiver)
        {
            if let Some(sql) = call.args.first() {
                self.record_sql(sql, call.span());
            } else {
                self.push("SQL001", call.span(), "sqlx Executor method is missing SQL");
            }
        }
        visit::visit_expr_method_call(self, call);
    }
}

impl Analyzer<'_> {
    fn remember_local(&mut self, local: &Local) {
        let syn::Pat::Ident(pattern) = &local.pat else {
            return;
        };
        let Some(initializer) = &local.init else {
            return;
        };
        if self.expression_is_sql_query(&initializer.expr) {
            if let Some(scope) = self.query_scopes.last_mut() {
                scope.insert(pattern.ident.to_string());
            }
        }
        if pattern.mutability.is_some() {
            return;
        }
        let Some(values) = self.resolve_strings(&initializer.expr) else {
            return;
        };
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(pattern.ident.to_string(), values);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Layer {
    Domain,
    Application,
    Infrastructure,
    Interface,
}

impl Layer {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "domain" => Some(Self::Domain),
            "application" => Some(Self::Application),
            "infrastructure" => Some(Self::Infrastructure),
            "interface" => Some(Self::Interface),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::Application => "application",
            Self::Infrastructure => "infrastructure",
            Self::Interface => "interface",
        }
    }

    fn allows(self, destination: Self) -> bool {
        match self {
            Self::Domain => destination == Self::Domain,
            Self::Application => matches!(destination, Self::Application | Self::Domain),
            Self::Infrastructure => matches!(destination, Self::Infrastructure | Self::Domain),
            Self::Interface => matches!(
                destination,
                Self::Interface | Self::Application | Self::Domain
            ),
        }
    }

    fn dependency_violation(
        self,
        segments: &[String],
        in_use: bool,
        local_names: &HashSet<String>,
        dependency_packages: &HashMap<String, String>,
        test_only: bool,
    ) -> Option<String> {
        let root = segments.first().map(String::as_str)?;
        if self == Self::Infrastructure {
            return None;
        }
        let second = segments.get(1).map(String::as_str);
        let forbidden_io =
            matches!(
                (root, second),
                ("std", Some("env" | "fs" | "process"))
                    | ("tokio", Some("fs" | "net" | "process" | "signal"))
            ) || (root == "std" && second == Some("net") && !address_only_network_path(segments));
        if forbidden_io {
            return Some(format!(
                "{} cannot access external capability {}",
                self.as_str(),
                segments
                    .iter()
                    .take(2)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("::")
            ));
        }

        let internal = [
            "crate",
            "self",
            "super",
            "domain",
            "application",
            "infrastructure",
            "interface",
        ];
        let built_in = ["std", "core", "alloc"];
        let test_allowed = ["futures", "tokio"];
        let language_primitives = [
            "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8",
            "u16", "u32", "u64", "u128", "usize",
        ];
        if internal.contains(&root) || language_primitives.contains(&root) {
            return None;
        }

        let root_is_local = !in_use
            && (segments.len() == 1
                || root.chars().next().is_some_and(char::is_uppercase)
                || local_names.contains(root));
        if root_is_local {
            return None;
        }
        let actual = dependency_packages.get(root).map(String::as_str);
        if actual.is_none() && built_in.contains(&root) {
            return None;
        }
        if actual.is_some_and(|package| {
            domain_dependency_allowed(package)
                || (self == Self::Application && application_dependency_allowed(package))
                || (self == Self::Interface && interface_dependency_allowed(package))
                || (test_only && test_allowed.contains(&package))
        }) {
            return None;
        }
        Some(format!(
            "{} dependency root {root}{} is not in the fail-closed external crate allowlist",
            self.as_str(),
            actual
                .filter(|package| *package != root)
                .map(|package| format!(" (package {package})"))
                .unwrap_or_default()
        ))
    }
}

fn domain_dependency_allowed(package: &str) -> bool {
    matches!(
        package,
        "anyhow"
            | "async-trait"
            | "bytes"
            | "chrono"
            | "futures"
            | "regex"
            | "serde"
            | "serde_json"
            | "sha2"
            | "thiserror"
            | "uuid"
    )
}

fn application_dependency_allowed(package: &str) -> bool {
    matches!(package, "tokio" | "tokio-stream" | "tracing")
}

fn interface_dependency_allowed(package: &str) -> bool {
    matches!(
        package,
        "anyhow"
            | "async-stream"
            | "async-trait"
            | "axum"
            | "bytes"
            | "futures"
            | "llm-client"
            | "serde"
            | "serde_json"
            | "tokio"
            | "tracing"
            | "uuid"
    )
}

fn reviewed_external_package(package: &str) -> bool {
    domain_dependency_allowed(package)
        || application_dependency_allowed(package)
        || (package != "llm-client" && interface_dependency_allowed(package))
}

fn address_only_network_path(segments: &[String]) -> bool {
    matches!(
        segments.get(2).map(String::as_str),
        Some("IpAddr" | "Ipv4Addr" | "Ipv6Addr" | "SocketAddr" | "SocketAddrV4" | "SocketAddrV6")
    )
}

fn raw_network_capability(segments: &[String]) -> bool {
    matches!(
        (
            segments.first().map(String::as_str),
            segments.get(1).map(String::as_str)
        ),
        (Some("std" | "tokio"), Some("net"))
    )
}

fn unsupported_database_client(package: &str) -> bool {
    matches!(
        package,
        "postgres" | "tokio-postgres" | "diesel" | "sea-orm" | "seaorm"
    )
}

fn helper_persistence_capability(package: &str) -> bool {
    package == "sqlx"
        || unsupported_database_client(package)
        || matches!(package, "redis" | "deadpool-redis" | "aws-sdk-s3")
}

fn forbidden_outbound_transport(package: &str) -> bool {
    matches!(
        package,
        "tonic"
            | "nats"
            | "async-nats"
            | "lapin"
            | "rdkafka"
            | "kafka"
            | "tokio-tungstenite"
            | "tungstenite"
            | "websocket"
            | "ureq"
            | "hyper"
    )
}

fn resolve_internal_layer(segments: &[String], module: &[String]) -> Option<Layer> {
    let mut normalized = Vec::new();
    match segments.first().map(String::as_str) {
        Some("crate") => normalized.extend_from_slice(&segments[1..]),
        Some("self") => {
            normalized.extend_from_slice(module);
            normalized.extend_from_slice(&segments[1..]);
        }
        Some("super") => {
            normalized.extend_from_slice(module);
            let mut index = 0;
            while segments
                .get(index)
                .is_some_and(|segment| segment == "super")
            {
                normalized.pop();
                index += 1;
            }
            normalized.extend_from_slice(&segments[index..]);
        }
        Some(first) if Layer::from_name(first).is_some() => normalized.extend_from_slice(segments),
        _ => return None,
    }
    normalized.first().and_then(|name| Layer::from_name(name))
}

fn collect_constants(file: &File) -> HashMap<String, Vec<String>> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Const(ItemConst { ident, expr, .. }) => {
                literal_expression(expr).map(|values| (ident.to_string(), values))
            }
            _ => None,
        })
        .collect()
}

fn literal_expression(expression: &Expr) -> Option<Vec<String>> {
    match expression {
        Expr::Lit(value) => match &value.lit {
            syn::Lit::Str(value) => Some(vec![value.value()]),
            _ => None,
        },
        Expr::Paren(value) => literal_expression(&value.expr),
        Expr::Group(value) => literal_expression(&value.expr),
        Expr::Reference(value) => literal_expression(&value.expr),
        _ => None,
    }
}

fn block_value(block: &Block) -> Option<&Expr> {
    match block.stmts.last()? {
        Stmt::Expr(expression, None) => Some(expression),
        _ => None,
    }
}

fn route_handler(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        Expr::Call(call) => call.args.first().and_then(route_handler),
        _ => None,
    }
}

fn first_macro_expression(tokens: &TokenStream) -> Option<Expr> {
    let mut expression = TokenStream::new();
    for token in tokens.clone() {
        if matches!(&token, TokenTree::Punct(punctuation) if punctuation.as_char() == ',') {
            break;
        }
        expression.extend([token]);
    }
    syn::parse2(expression).ok()
}

fn tokens_contain_ident(tokens: &TokenStream, expected: &str) -> bool {
    tokens.clone().into_iter().any(|token| match token {
        TokenTree::Ident(ident) => ident == expected,
        TokenTree::Group(group) => tokens_contain_ident(&group.stream(), expected),
        _ => false,
    })
}

fn sql_api(name: &str) -> Option<SqlApi> {
    if matches!(
        name,
        "query_file"
            | "query_file_as"
            | "query_file_scalar"
            | "query_file_unchecked"
            | "query_file_as_unchecked"
            | "query_file_scalar_unchecked"
    ) {
        return Some(SqlApi::Unsupported);
    }
    if matches!(
        name,
        "query_unchecked" | "query_as_unchecked" | "query_scalar_unchecked"
    ) {
        return Some(SqlApi::Unsupported);
    }
    let macro_index = usize::from(matches!(name, "query_as" | "query_as_with"));
    matches!(
        name,
        "query"
            | "query_as"
            | "query_scalar"
            | "query_with"
            | "query_as_with"
            | "query_scalar_with"
            | "raw_sql"
    )
    .then_some(SqlApi::Literal {
        function_index: 0,
        macro_index,
    })
}

fn executor_api(name: &str) -> bool {
    matches!(
        name,
        "execute"
            | "execute_many"
            | "fetch"
            | "fetch_many"
            | "fetch_all"
            | "fetch_one"
            | "fetch_optional"
            | "prepare"
            | "prepare_with"
            | "describe"
    )
}

fn path_segments(path: &SynPath) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

fn flatten_use(tree: &UseTree, mut prefix: Vec<String>, output: &mut Vec<Vec<String>>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            flatten_use(&path.tree, prefix, output);
        }
        UseTree::Name(name) => {
            prefix.push(name.ident.to_string());
            output.push(prefix);
        }
        UseTree::Rename(rename) => {
            prefix.push(rename.ident.to_string());
            output.push(prefix);
        }
        UseTree::Glob(_) => output.push(prefix),
        UseTree::Group(group) => {
            for item in &group.items {
                flatten_use(item, prefix.clone(), output);
            }
        }
    }
}

fn collect_local_names(file: &File) -> HashSet<String> {
    let mut names = HashSet::new();
    for item in &file.items {
        match item {
            syn::Item::Use(item) => collect_use_binding_names(&item.tree, &mut names),
            syn::Item::Mod(item) => {
                names.insert(item.ident.to_string());
            }
            syn::Item::ExternCrate(item) => {
                names.insert(
                    item.rename
                        .as_ref()
                        .map(|(_, alias)| alias.to_string())
                        .unwrap_or_else(|| item.ident.to_string()),
                );
            }
            _ => {}
        }
    }
    names
}

fn collect_use_binding_names(tree: &UseTree, output: &mut HashSet<String>) {
    match tree {
        UseTree::Path(path) => collect_use_binding_names(&path.tree, output),
        UseTree::Name(name) => {
            output.insert(name.ident.to_string());
        }
        UseTree::Rename(rename) => {
            output.insert(rename.rename.to_string());
        }
        UseTree::Glob(_) => {}
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_binding_names(item, output);
            }
        }
    }
}

fn module_for(path: &Path, source_root: &Path) -> Vec<String> {
    let relative = path.strip_prefix(source_root).unwrap_or(path);
    let mut parts = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let Some(file) = parts.pop() else {
        return Vec::new();
    };
    let stem = file.strip_suffix(".rs").unwrap_or(&file);
    if !matches!(stem, "lib" | "main" | "mod") {
        parts.push(stem.to_string());
    }
    parts
}

fn module_closure(source_root: &Path) -> Result<BTreeMap<PathBuf, bool>> {
    let entries = [source_root.join("main.rs"), source_root.join("lib.rs")]
        .into_iter()
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    if entries.is_empty() {
        bail!("{} has neither main.rs nor lib.rs", source_root.display());
    }
    module_closure_from_entries(&entries)
}

fn module_closure_from_entries(entries: &[PathBuf]) -> Result<BTreeMap<PathBuf, bool>> {
    if entries.is_empty() {
        bail!("cargo target inventory has no source entries");
    }
    let mut queue = entries
        .iter()
        .cloned()
        .map(|path| (path, false, true))
        .collect::<VecDeque<_>>();

    let mut files = BTreeMap::<PathBuf, bool>::new();
    let mut visited = HashSet::new();
    while let Some((path, test_only, crate_root)) = queue.pop_front() {
        if !visited.insert((path.clone(), test_only)) {
            continue;
        }
        files
            .entry(path.clone())
            .and_modify(|existing| *existing &= test_only)
            .or_insert(test_only);
        let source = fs::read_to_string(&path)
            .with_context(|| format!("read module root {}", path.display()))?;
        let syntax = syn::parse_file(&source)
            .with_context(|| format!("parse module root {}", path.display()))?;
        let base = if crate_root {
            path.parent()
                .context("cargo target source has no parent")?
                .to_path_buf()
        } else {
            module_base(&path)
        };
        discover_modules(&syntax.items, &base, test_only, &mut queue)?;
    }
    Ok(files)
}

fn discover_modules(
    items: &[syn::Item],
    base: &Path,
    inherited_test: bool,
    queue: &mut VecDeque<(PathBuf, bool, bool)>,
) -> Result<()> {
    for item in items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        let test_only = inherited_test || item_is_test(&module.attrs);
        if let Some((_, nested)) = &module.content {
            discover_modules(
                nested,
                &base.join(module.ident.to_string()),
                test_only,
                queue,
            )?;
            continue;
        }
        if module
            .attrs
            .iter()
            .any(|attribute| attribute.path().is_ident("path"))
        {
            continue;
        }
        let flat = base.join(format!("{}.rs", module.ident));
        let nested = base.join(module.ident.to_string()).join("mod.rs");
        let target = match (flat.is_file(), nested.is_file()) {
            (true, false) => flat,
            (false, true) => nested,
            (false, false) => {
                bail!(
                    "module {} has no {} or {}",
                    module.ident,
                    flat.display(),
                    nested.display()
                )
            }
            (true, true) => {
                bail!(
                    "module {} is ambiguous between {} and {}",
                    module.ident,
                    flat.display(),
                    nested.display()
                )
            }
        };
        queue.push_back((target, test_only, false));
    }
    Ok(())
}

fn module_base(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    match path.file_stem().and_then(|stem| stem.to_str()) {
        Some("main" | "lib" | "mod") => parent.to_path_buf(),
        Some(stem) => parent.join(stem),
        None => parent.to_path_buf(),
    }
}

fn item_is_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("test")
            || (attribute.path().is_ident("cfg")
                && attribute
                    .parse_args::<syn::Meta>()
                    .is_ok_and(|meta| cfg_requires_test(&meta)))
    })
}

fn cfg_requires_test(meta: &syn::Meta) -> bool {
    match meta {
        syn::Meta::Path(path) => path.is_ident("test"),
        syn::Meta::List(list) if list.path.is_ident("all") || list.path.is_ident("any") => {
            let Ok(nested) =
                Punctuated::<syn::Meta, Token![,]>::parse_terminated.parse2(list.tokens.clone())
            else {
                return false;
            };
            if list.path.is_ident("all") {
                nested.iter().any(cfg_requires_test)
            } else {
                !nested.is_empty() && nested.iter().all(cfg_requires_test)
            }
        }
        _ => false,
    }
}

pub fn normalize_path(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized
        .strip_prefix("//?/")
        .unwrap_or(&normalized)
        .to_string()
}

fn analyze_snippet(
    source: &str,
    service: &str,
    relative: &str,
    module: Vec<String>,
    layer: Option<Layer>,
    dependency_packages: HashMap<String, String>,
    facts_enabled: bool,
) -> Result<(Vec<Diagnostic>, Vec<SqlCall>, RuntimeFacts)> {
    let file = syn::parse_file(source)?;
    let mut diagnostics = Vec::new();
    let mut sql_calls = Vec::new();
    let mut references = 0;
    let mut facts = RuntimeFacts::default();
    let composition_root = service != "gateway"
        && matches!(
            Path::new(relative)
                .file_name()
                .and_then(|name| name.to_str()),
            Some("main.rs" | "lib.rs")
        );
    if composition_root {
        audit_composition_root(&file, relative, &mut diagnostics);
    }
    {
        let sql_aliases = collect_sql_aliases_with_dependencies(&file, &dependency_packages);
        let internal_aliases =
            collect_internal_aliases_with_dependencies(&file, &dependency_packages);
        let mut analyzer = Analyzer {
            service,
            relative,
            module,
            layer,
            diagnostics: &mut diagnostics,
            sql_calls: &mut sql_calls,
            explicit_references: &mut references,
            facts: &mut facts,
            constants: collect_constants(&file),
            sql_aliases,
            internal_aliases,
            local_names: collect_local_names(&file),
            dependency_packages,
            scopes: Vec::new(),
            query_scopes: Vec::new(),
            current_function: "<module>".into(),
            facts_enabled,
            in_use: false,
            composition_root,
            saw_reqwest: false,
            saw_internal_http_marker: false,
        };
        for (span, message) in analyzer.sql_aliases.issues.clone() {
            analyzer.push("SQL006", span, message);
        }
        for (span, message) in analyzer.internal_aliases.issues.clone() {
            analyzer.push("LAYER010", span, message);
        }
        analyzer.visit_file(&file);
        analyzer.finish_file();
    }
    Ok((diagnostics, sql_calls, facts))
}

pub fn self_test() -> Result<()> {
    assert_eq!(
        resolve_internal_layer(
            &[
                "crate".into(),
                "infrastructure".into(),
                "persistence".into()
            ],
            &["domain".into(), "services".into()]
        ),
        Some(Layer::Infrastructure)
    );
    assert!(!Layer::Domain.allows(Layer::Infrastructure));
    assert!(Layer::Interface.allows(Layer::Application));

    let file = syn::parse_file(
        r#"
        const FIRST: &str = "SELECT * FROM users";
        fn sample() {
            let sql = if true { "SELECT * FROM users" } else { "SELECT * FROM novels" };
            sqlx::query(sql);
        }
        "#,
    )?;
    let constants = collect_constants(&file);
    assert_eq!(constants["FIRST"], vec!["SELECT * FROM users"]);

    let mut diagnostics = Vec::new();
    let mut sql_calls = Vec::new();
    let mut references = 0;
    let mut facts = RuntimeFacts::default();
    {
        let mut analyzer = Analyzer {
            service: "test-service",
            relative: "services/test-service/src/infrastructure/persistence/test.rs",
            module: vec!["infrastructure".into(), "persistence".into(), "test".into()],
            layer: Some(Layer::Infrastructure),
            diagnostics: &mut diagnostics,
            sql_calls: &mut sql_calls,
            explicit_references: &mut references,
            facts: &mut facts,
            constants,
            sql_aliases: collect_sql_aliases(&file),
            internal_aliases: collect_internal_aliases(&file),
            local_names: collect_local_names(&file),
            dependency_packages: HashMap::new(),
            scopes: Vec::new(),
            query_scopes: Vec::new(),
            current_function: "<module>".into(),
            facts_enabled: true,
            in_use: false,
            composition_root: false,
            saw_reqwest: false,
            saw_internal_http_marker: false,
        };
        analyzer.visit_file(&file);
    }
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(sql_calls.len(), 2);

    let format_file = syn::parse_file(
        r#"fn config() { let _ = format!("{}", std::env::var("RUST_LOG").unwrap()); }"#,
    )?;
    let mut diagnostics = Vec::new();
    let mut sql_calls = Vec::new();
    let mut references = 0;
    let mut facts = RuntimeFacts::default();
    {
        let mut analyzer = Analyzer {
            service: "test-service",
            relative: "services/test-service/src/main.rs",
            module: Vec::new(),
            layer: None,
            diagnostics: &mut diagnostics,
            sql_calls: &mut sql_calls,
            explicit_references: &mut references,
            facts: &mut facts,
            constants: HashMap::new(),
            sql_aliases: collect_sql_aliases(&format_file),
            internal_aliases: collect_internal_aliases(&format_file),
            local_names: collect_local_names(&format_file),
            dependency_packages: HashMap::new(),
            scopes: Vec::new(),
            query_scopes: Vec::new(),
            current_function: "<module>".into(),
            facts_enabled: true,
            in_use: false,
            composition_root: true,
            saw_reqwest: false,
            saw_internal_http_marker: false,
        };
        analyzer.visit_file(&format_file);
    }
    assert!(facts.env_keys.contains("RUST_LOG"));

    let hidden_sql = syn::parse_file(
        r#"
        async fn ready() {
            let _ = matches!(
                sqlx::query_scalar("SELECT id FROM users").fetch_one(&pool).await,
                Ok(true)
            );
        }
        "#,
    )?;
    let mut diagnostics = Vec::new();
    let mut sql_calls = Vec::new();
    let mut references = 0;
    let mut facts = RuntimeFacts::default();
    {
        let mut analyzer = Analyzer {
            service: "test-service",
            relative: "services/test-service/src/infrastructure/persistence/hidden.rs",
            module: vec!["infrastructure".into(), "persistence".into()],
            layer: Some(Layer::Infrastructure),
            diagnostics: &mut diagnostics,
            sql_calls: &mut sql_calls,
            explicit_references: &mut references,
            facts: &mut facts,
            constants: HashMap::new(),
            sql_aliases: collect_sql_aliases(&hidden_sql),
            internal_aliases: collect_internal_aliases(&hidden_sql),
            local_names: collect_local_names(&hidden_sql),
            dependency_packages: HashMap::new(),
            scopes: Vec::new(),
            query_scopes: Vec::new(),
            current_function: "<module>".into(),
            facts_enabled: true,
            in_use: false,
            composition_root: false,
            saw_reqwest: false,
            saw_internal_http_marker: false,
        };
        analyzer.visit_file(&hidden_sql);
    }
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(sql_calls.len(), 1);

    let adversarial_sql = r#"
        use db::query_scalar as scalar;
        use sqlx as db;
        use sqlx::query as q;
        mod local { pub fn query(_: &str) {} }
        fn sample(dynamic: &str) {
            scalar("SELECT id FROM users");
            q("SELECT id FROM novels");
            let _ = matches!(db::query("SELECT id FROM chapters"), _);
            db::raw_sql(dynamic);
            local::query("SELECT must_not_be_a_sqlx_call");
            db::query_as_unchecked!(Row, "SELECT id FROM characters");
            db::query_file!("queries/hidden.sql");
        }
    "#;
    let (diagnostics, sql_calls, _) = analyze_snippet(
        adversarial_sql,
        "test-service",
        "services/test-service/src/infrastructure/persistence/adversarial.rs",
        vec!["infrastructure".into(), "persistence".into()],
        Some(Layer::Infrastructure),
        HashMap::new(),
        true,
    )?;
    assert_eq!(sql_calls.len(), 3, "{sql_calls:?}");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "SQL001")
            .count(),
        1,
        "{diagnostics:?}"
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "SQL005")
            .count(),
        2,
        "{diagnostics:?}"
    );
    assert!(!sql_calls
        .iter()
        .any(|call| call.sql.contains("must_not_be_a_sqlx_call")));

    let executor_sql = r#"
        use sqlx as db;
        use sqlx::Executor as DbExecutor;
        use sqlx::Executor::describe as describe_sql;
        fn sample(pool: &Pool, dynamic: &str) {
            pool.execute(dynamic);
            sqlx::Executor::fetch_one(pool, dynamic);
            db::Executor::execute(pool, dynamic);
            describe_sql(pool, dynamic);
            DbExecutor::fetch_all(pool, "SELECT id FROM users");
            let statement = sqlx::query("SELECT id FROM novels");
            statement.execute(pool);
            sqlx::query("SELECT id FROM chapters").fetch_optional(pool);
        }
    "#;
    let (diagnostics, sql_calls, _) = analyze_snippet(
        executor_sql,
        "test-service",
        "services/test-service/src/infrastructure/executor.rs",
        vec!["infrastructure".into(), "executor".into()],
        Some(Layer::Infrastructure),
        HashMap::new(),
        true,
    )?;
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "SQL001")
            .count(),
        4,
        "{diagnostics:?}"
    );
    assert_eq!(sql_calls.len(), 3, "{sql_calls:?}");

    let (diagnostics, _, _) = analyze_snippet(
        "pub use sqlx::query as hidden_query;",
        "test-service",
        "services/test-service/src/infrastructure/persistence/reexport.rs",
        vec!["infrastructure".into(), "persistence".into()],
        Some(Layer::Infrastructure),
        HashMap::new(),
        true,
    )?;
    assert!(diagnostics.iter().any(|item| item.code == "SQL006"));

    let metadata_root = PathBuf::from("C:/architecture-check-fixture");
    let gateway_id = "gateway 0.1.0 (path+file:///C:/architecture-check-fixture/gateway)";
    let bridge_id = "bridge 0.1.0 (path+file:///C:/architecture-check-fixture/bridge)";
    let novel_id =
        "novel-service 0.1.0 (path+file:///C:/architecture-check-fixture/services/novel-service)";
    let serde_id = "serde 0.1.0 (path+file:///C:/architecture-check-fixture/serde)";
    let db_bridge_id =
        "db-bridge 0.1.0 (path+file:///C:/architecture-check-fixture/local/db-bridge)";
    let metadata = serde_json::json!({
        "workspace_members": [gateway_id, bridge_id, novel_id, serde_id],
        "packages": [
            {
                "id": gateway_id,
                "name": "gateway",
                "manifest_path": "C:/architecture-check-fixture/gateway/Cargo.toml",
                "dependencies": [
                    {"name": "bridge", "rename": null},
                    {"name": "redis", "rename": null}
                ]
            },
            {
                "id": bridge_id,
                "name": "bridge",
                "manifest_path": "C:/architecture-check-fixture/bridge/Cargo.toml",
                "dependencies": [
                    {"name": "novel-service", "rename": null},
                    {"name": "sqlx", "rename": null}
                ]
            },
            {
                "id": novel_id,
                "name": "novel-service",
                "manifest_path": "C:/architecture-check-fixture/services/novel-service/Cargo.toml",
                "targets": [
                    {"kind": ["bin"], "src_path": "C:/architecture-check-fixture/services/novel-service/evil.rs"},
                    {"kind": ["custom-build"], "src_path": "C:/architecture-check-fixture/services/novel-service/build.rs"}
                ],
                "dependencies": [
                    {"name": "shared-adapters", "rename": "serde"},
                    {"name": "sqlx", "rename": "db"},
                    {"name": "reqwest", "rename": "web"},
                    {"name": "tonic", "rename": "rpc"},
                    {"name": "db-bridge", "rename": null, "path": "C:/architecture-check-fixture/local/db-bridge"},
                    {"name": "tokio", "rename": null}
                ]
            },
            {
                "id": serde_id,
                "name": "serde",
                "manifest_path": "C:/architecture-check-fixture/serde/Cargo.toml",
                "dependencies": []
            },
            {
                "id": db_bridge_id,
                "name": "db-bridge",
                "source": null,
                "manifest_path": "C:/architecture-check-fixture/local/db-bridge/Cargo.toml",
                "dependencies": [
                    {"name": "sqlx", "rename": null},
                    {"name": "rdkafka", "rename": null}
                ]
            }
        ]
    });
    let runtime = |package: &str, root: &str| RuntimeSpec {
        package: package.into(),
        root: root.into(),
        liveness_path: "/health/live".into(),
        readiness_path: "/health/ready".into(),
        required_env: Vec::new(),
    };
    let cargo_diagnostics = cargo_diagnostics_from_metadata_with_helpers(
        &metadata_root,
        &[
            runtime("gateway", "gateway"),
            runtime("novel-service", "services/novel-service"),
        ],
        &metadata,
        &WorkspaceHelperPolicy::new(),
    )?;
    assert!(cargo_diagnostics.iter().any(|item| item.code == "PKG003"));
    assert!(cargo_diagnostics
        .iter()
        .any(|item| { item.code == "PKG004" && item.message.contains("redis") }));
    assert!(cargo_diagnostics.iter().any(|item| {
        item.code == "PKG004" && item.message.contains("sqlx") && item.message.contains("bridge")
    }));
    assert!(cargo_diagnostics
        .iter()
        .any(|item| item.code == "PKG005" && item.message.contains("serde")));
    assert!(cargo_diagnostics
        .iter()
        .any(|item| item.code == "PKG006" && item.message.contains("db-bridge")));
    assert!(cargo_diagnostics
        .iter()
        .any(|item| item.code == "PKG007" && item.message.contains("db-bridge")));
    assert!(cargo_diagnostics
        .iter()
        .any(|item| item.code == "PKG008" && item.message.contains("db-bridge")));
    assert!(cargo_diagnostics
        .iter()
        .any(|item| item.code == "PKG010" && item.message.contains("novel-service")));
    let cargo_targets = cargo_targets_from_metadata(&metadata)?;
    assert_eq!(
        normalize_path(&cargo_targets["novel-service"].production[0]),
        "C:/architecture-check-fixture/services/novel-service/evil.rs"
    );

    let duplicate_helper_id =
        "llm-client 0.2.0 (path+file:///C:/architecture-check-fixture/evil/llm-client)";
    let mut duplicate_metadata = metadata.clone();
    duplicate_metadata["packages"]
        .as_array_mut()
        .context("fixture packages missing")?
        .extend([serde_json::json!({
            "id": duplicate_helper_id,
            "name": "llm-client",
            "source": null,
            "manifest_path": "C:/architecture-check-fixture/evil/llm-client/Cargo.toml",
            "targets": [{
                "kind": ["lib"],
                "src_path": "C:/architecture-check-fixture/evil/llm-client/src/lib.rs"
            }],
            "dependencies": []
        }), serde_json::json!({
            "id": "llm-client 0.1.0 (path+file:///C:/architecture-check-fixture/good/llm-client)",
            "name": "llm-client",
            "source": null,
            "manifest_path": "C:/architecture-check-fixture/good/llm-client/Cargo.toml",
            "targets": [{
                "kind": ["lib"],
                "src_path": "C:/architecture-check-fixture/good/llm-client/src/lib.rs"
            }],
            "dependencies": []
        })]);
    let duplicate_error = cargo_targets_from_metadata(&duplicate_metadata)
        .expect_err("duplicate local helper names must fail closed");
    assert!(duplicate_error.to_string().contains("PKG011"));

    let helper_dependencies = HashMap::from([
        ("tokio".into(), "tokio".into()),
        ("sqlx".into(), "sqlx".into()),
    ]);
    let helper_escape = syn::parse_file(
        r#"
        use tokio as rt;
        use sqlx as db;
        macro_rules! hidden_transport { () => { tokio::net::TcpStream }; }
        unsafe extern "C" { fn foreign(); }
        fn escape() {
            let _ = rt::net::TcpStream;
            let _ = std::process::Command::new("program");
            let _ = db::query("SELECT id FROM users");
            unsafe { foreign(); }
            include!("hidden.rs");
            let _ = hidden_transport!();
        }
        "#,
    )?;
    let diagnostics = audit_helper_syntax(
        "llm-client-style-helper",
        "crates/llm-client-style-helper/src/lib.rs",
        &helper_escape,
        &helper_dependencies,
    );
    assert!(
        diagnostics
            .iter()
            .filter(|item| item.code == "PKG009")
            .count()
            >= 7,
        "{diagnostics:?}"
    );
    let helper_allowed = syn::parse_file(
        r#"
        use reqwest::Client;
        fn allowed() {
            let _ = Client::new();
            let _ = std::env::var("LLM_API_URL");
            let _ = tokio::time::sleep(std::time::Duration::from_secs(1));
        }
        #[cfg(test)]
        mod tests {
            fn ignored() {
                let _ = tokio::net::TcpStream;
                unsafe { std::hint::unreachable_unchecked(); }
            }
        }
        "#,
    )?;
    let diagnostics = audit_helper_syntax(
        "llm-client-style-helper",
        "crates/llm-client-style-helper/src/lib.rs",
        &helper_allowed,
        &HashMap::from([
            ("reqwest".into(), "reqwest".into()),
            ("tokio".into(), "tokio".into()),
        ]),
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let dependency_maps = dependency_source_maps(&metadata)?;
    assert_eq!(dependency_maps["novel-service"]["serde"], "shared-adapters");
    assert_eq!(dependency_maps["novel-service"]["db"], "sqlx");
    assert_eq!(dependency_maps["novel-service"]["web"], "reqwest");
    assert_eq!(dependency_maps["novel-service"]["rpc"], "tonic");
    let (diagnostics, sql_calls, _) = analyze_snippet(
        r#"
        use db::query;
        use web::Client;
        use rpc::transport::Channel;
        fn escape(dynamic: &str) {
            query(dynamic);
            let _ = Client::new();
            let _ = Channel::from_static("http://service");
        }
        "#,
        "novel-service",
        "services/novel-service/src/infrastructure/persistence/renamed.rs",
        vec![
            "infrastructure".into(),
            "persistence".into(),
            "renamed".into(),
        ],
        Some(Layer::Infrastructure),
        dependency_maps["novel-service"].clone(),
        true,
    )?;
    assert!(sql_calls.is_empty());
    assert!(diagnostics.iter().any(|item| item.code == "SQL001"));
    assert!(diagnostics.iter().any(|item| item.code == "HTTP002"));
    assert!(diagnostics.iter().any(|item| item.code == "HTTP003"));
    let (diagnostics, _, _) = analyze_snippet(
        "use db::*; fn escape(dynamic: &str) { query(dynamic); }",
        "novel-service",
        "services/novel-service/src/infrastructure/persistence/glob.rs",
        vec!["infrastructure".into(), "persistence".into(), "glob".into()],
        Some(Layer::Infrastructure),
        dependency_maps["novel-service"].clone(),
        true,
    )?;
    assert!(diagnostics.iter().any(|item| item.code == "SQL006"));
    let (diagnostics, _, _) = analyze_snippet(
        "use serde::AdapterClient;",
        "novel-service",
        "services/novel-service/src/domain/ports.rs",
        vec!["domain".into(), "ports".into()],
        Some(Layer::Domain),
        dependency_maps["novel-service"].clone(),
        true,
    )?;
    assert!(diagnostics
        .iter()
        .any(|item| { item.code == "LAYER002" && item.message.contains("shared-adapters") }));

    let domain_source = r#"
        use shared_adapters::Client;
        use std::env;
        use tokio::net::TcpStream;
        use llm_client::LlmClient;
        #[cfg(test)]
        mod tests {
            use super::super::super::infrastructure;
            #[derive(sqlx::Type)]
            struct PersistenceRow;
        }
    "#;
    let dependencies = HashMap::from([
        ("shared_adapters".into(), "shared-adapters".into()),
        ("tokio".into(), "tokio".into()),
        ("llm_client".into(), "llm-client".into()),
        ("sqlx".into(), "sqlx".into()),
    ]);
    let (diagnostics, _, _) = analyze_snippet(
        domain_source,
        "test-service",
        "services/test-service/src/domain/entities/model.rs",
        vec!["domain".into(), "entities".into()],
        Some(Layer::Domain),
        dependencies,
        true,
    )?;
    assert!(
        diagnostics
            .iter()
            .filter(|item| item.code == "LAYER002")
            .count()
            >= 5
    );
    assert!(diagnostics.iter().any(|item| item.code == "LAYER001"));

    let application_source = r#"
        use std::env;
        use std::fs;
        use std::net::{Ipv6Addr, TcpStream};
        use std::process;
        use tokio::fs as async_fs;
        use tokio::net::TcpStream as AsyncTcpStream;
        fn addresses_only() { let _ = Ipv6Addr::LOCALHOST; }
    "#;
    let (diagnostics, _, _) = analyze_snippet(
        application_source,
        "test-service",
        "services/test-service/src/application/handlers/io_escape.rs",
        vec!["application".into(), "handlers".into(), "io_escape".into()],
        Some(Layer::Application),
        HashMap::from([("tokio".into(), "tokio".into())]),
        true,
    )?;
    assert!(
        diagnostics
            .iter()
            .filter(|item| item.code == "LAYER002")
            .count()
            >= 6,
        "{diagnostics:?}"
    );

    let interface_dependencies = HashMap::from([
        ("axum".into(), "axum".into()),
        ("serde".into(), "serde".into()),
        ("redis".into(), "redis".into()),
        ("aws_sdk_s3".into(), "aws-sdk-s3".into()),
        ("bcrypt".into(), "bcrypt".into()),
        ("tokio".into(), "tokio".into()),
    ]);
    let (diagnostics, _, _) = analyze_snippet(
        r#"
        use axum::Json;
        use serde::Serialize;
        use redis::Client;
        use aws_sdk_s3::Client as S3Client;
        use bcrypt::hash;
        use tokio::fs;
        "#,
        "test-service",
        "services/test-service/src/interface/http/escape.rs",
        vec!["interface".into(), "http".into(), "escape".into()],
        Some(Layer::Interface),
        interface_dependencies,
        true,
    )?;
    assert!(
        diagnostics
            .iter()
            .filter(|item| item.code == "LAYER002")
            .count()
            >= 4,
        "{diagnostics:?}"
    );

    let (diagnostics, _, _) = analyze_snippet(
        r#"
        extern crate reqwest as net;
        extern crate sqlx as db;
        "#,
        "test-service",
        "services/test-service/src/domain/ports.rs",
        vec!["domain".into(), "ports".into()],
        Some(Layer::Domain),
        HashMap::from([
            ("reqwest".into(), "reqwest".into()),
            ("sqlx".into(), "sqlx".into()),
        ]),
        true,
    )?;
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "LAYER012")
            .count(),
        2,
        "{diagnostics:?}"
    );

    let (diagnostics, _, _) = analyze_snippet(
        r#"
        unsafe extern "C" { fn foreign(); }
        unsafe trait Marker { unsafe fn mark(); }
        struct Model;
        unsafe impl Marker for Model { unsafe fn mark() {} }
        unsafe fn direct() {}
        fn block() { unsafe { foreign(); } core::arch::asm!("nop"); }
        "#,
        "test-service",
        "services/test-service/src/interface/http/unsafe_escape.rs",
        vec!["interface".into(), "http".into(), "unsafe_escape".into()],
        Some(Layer::Interface),
        HashMap::new(),
        true,
    )?;
    assert!(
        diagnostics
            .iter()
            .filter(|item| item.code == "LAYER013")
            .count()
            >= 6,
        "{diagnostics:?}"
    );

    let cfg_file = syn::parse_file(
        r#"
        #[cfg(all(test, unix))] fn test_and_unix() {}
        #[cfg(any(test, feature = "hooks"))] fn test_or_feature() {}
        #[some_macro::test] fn qualified_test_attribute() {}
        "#,
    )?;
    let syn::Item::Fn(test_and_unix) = &cfg_file.items[0] else {
        unreachable!()
    };
    let syn::Item::Fn(test_or_feature) = &cfg_file.items[1] else {
        unreachable!()
    };
    let syn::Item::Fn(qualified_test_attribute) = &cfg_file.items[2] else {
        unreachable!()
    };
    assert!(item_is_test(&test_and_unix.attrs));
    assert!(!item_is_test(&test_or_feature.attrs));
    assert!(!item_is_test(&qualified_test_attribute.attrs));

    let http_source = r#"
        use reqwest::Client;
        fn call() { let _ = "NOVEL_SERVICE_URL"; let _ = Client::new(); }
    "#;
    let (diagnostics, _, _) = analyze_snippet(
        http_source,
        "agent-service",
        "services/agent-service/src/infrastructure/persistence/client.rs",
        vec!["infrastructure".into(), "persistence".into()],
        Some(Layer::Infrastructure),
        HashMap::new(),
        true,
    )?;
    assert!(diagnostics.iter().any(|item| item.code == "HTTP001"));
    let (diagnostics, _, _) = analyze_snippet(
        http_source,
        "agent-service",
        "services/agent-service/src/infrastructure/http/client.rs",
        vec!["infrastructure".into(), "http".into()],
        Some(Layer::Infrastructure),
        HashMap::new(),
        true,
    )?;
    assert!(!diagnostics.iter().any(|item| item.code == "HTTP001"));
    let (diagnostics, _, _) = analyze_snippet(
        "use reqwest::Client; fn call() { let _ = Client::new(); }",
        "agent-service",
        "services/agent-service/src/infrastructure/persistence/escape.rs",
        vec!["infrastructure".into(), "persistence".into()],
        Some(Layer::Infrastructure),
        HashMap::new(),
        true,
    )?;
    assert!(diagnostics.iter().any(|item| item.code == "HTTP003"));

    for (relative, module, layer) in [
        (
            "services/test-service/src/interface/http/sql_escape.rs",
            vec!["interface".into(), "http".into(), "sql_escape".into()],
            Layer::Interface,
        ),
        (
            "services/test-service/src/infrastructure/http/sql_escape.rs",
            vec!["infrastructure".into(), "http".into(), "sql_escape".into()],
            Layer::Infrastructure,
        ),
    ] {
        let (diagnostics, sql_calls, _) = analyze_snippet(
            "fn escape() { sqlx::query(\"SELECT id FROM users\"); }",
            "test-service",
            relative,
            module,
            Some(layer),
            HashMap::from([("sqlx".into(), "sqlx".into())]),
            true,
        )?;
        assert_eq!(sql_calls.len(), 1, "{diagnostics:?}");
        assert!(
            diagnostics.iter().any(|item| item.code == "SQL008"),
            "{diagnostics:?}"
        );
    }

    let (diagnostics, _, _) = analyze_snippet(
        r#"
        use std::net::{Ipv6Addr, TcpStream};
        use hyper::Client;
        use ureq::Agent;
        fn escape() { let _ = Ipv6Addr::LOCALHOST; }
        "#,
        "test-service",
        "services/test-service/src/infrastructure/persistence/transport.rs",
        vec![
            "infrastructure".into(),
            "persistence".into(),
            "transport".into(),
        ],
        Some(Layer::Infrastructure),
        HashMap::from([
            ("hyper".into(), "hyper".into()),
            ("ureq".into(), "ureq".into()),
        ]),
        true,
    )?;
    assert!(
        diagnostics
            .iter()
            .filter(|item| item.code == "HTTP004")
            .count()
            >= 3,
        "{diagnostics:?}"
    );
    let (diagnostics, _, _) = analyze_snippet(
        r#"
        mod hidden {
            use crate as root;
            fn escape() { let _ = root::infrastructure::Adapter; }
        }
        "#,
        "test-service",
        "services/test-service/src/domain/entities/nested.rs",
        vec!["domain".into(), "entities".into(), "nested".into()],
        Some(Layer::Domain),
        HashMap::new(),
        true,
    )?;
    assert!(diagnostics.iter().any(|item| item.code == "LAYER010"));
    assert!(diagnostics.iter().any(|item| item.code == "LAYER001"));
    let (diagnostics, _, _) = analyze_snippet(
        r#"
        use super::super::super as root;
        fn escape() { let _ = root::infrastructure::Adapter; }
        "#,
        "test-service",
        "services/test-service/src/domain/entities/deep.rs",
        vec!["domain".into(), "entities".into(), "deep".into()],
        Some(Layer::Domain),
        HashMap::new(),
        true,
    )?;
    assert!(diagnostics.iter().any(|item| item.code == "LAYER010"));
    assert!(diagnostics.iter().any(|item| item.code == "LAYER001"));

    let (diagnostics, _, _) = analyze_snippet(
        r#"
        use tokio as rt;
        use std as platform;
        extern crate tokio as async_runtime;
        extern crate std as standard_library;
        fn escape() {
            let _ = rt::net::TcpStream;
            let _ = platform::net::TcpStream;
            let _ = async_runtime::net::TcpStream;
            let _ = standard_library::net::TcpStream;
        }
        "#,
        "test-service",
        "services/test-service/src/infrastructure/persistence/alias_transport.rs",
        vec![
            "infrastructure".into(),
            "persistence".into(),
            "alias_transport".into(),
        ],
        Some(Layer::Infrastructure),
        HashMap::from([("tokio".into(), "tokio".into())]),
        true,
    )?;
    assert!(
        diagnostics
            .iter()
            .filter(|item| item.code == "HTTP004")
            .count()
            >= 4,
        "{diagnostics:?}"
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "LAYER012")
            .count(),
        2,
        "{diagnostics:?}"
    );

    let (diagnostics, _, _) = analyze_snippet(
        r#"
        use std as platform;
        use tokio as rt;
        fn escape() {
            let _ = platform::fs::read("secret");
            let _ = rt::process::Command::new("program");
        }
        "#,
        "test-service",
        "services/test-service/src/application/handlers/alias_io.rs",
        vec!["application".into(), "handlers".into(), "alias_io".into()],
        Some(Layer::Application),
        HashMap::from([("tokio".into(), "tokio".into())]),
        true,
    )?;
    assert!(
        diagnostics
            .iter()
            .filter(|item| item.code == "LAYER002")
            .count()
            >= 2,
        "{diagnostics:?}"
    );
    let (diagnostics, _, _) = analyze_snippet(
        "use tokio::net::TcpListener; async fn main() { let _ = TcpListener::bind(\"127.0.0.1:0\").await; }",
        "test-service",
        "services/test-service/src/main.rs",
        Vec::new(),
        None,
        HashMap::from([("tokio".into(), "tokio".into())]),
        true,
    )?;
    assert!(!diagnostics.iter().any(|item| item.code == "HTTP004"));

    let (diagnostics, _, _) = analyze_snippet(
        r#"
        use sqlx::query;
        struct RootAdapter;
        impl RootAdapter { fn save(&self) {} }
        fn business_logic() {}
        async fn main() { query("SELECT id FROM users"); }
        #[cfg(test)] struct TestOnlyHelper;
        "#,
        "test-service",
        "services/test-service/src/main.rs",
        Vec::new(),
        None,
        HashMap::new(),
        true,
    )?;
    assert!(diagnostics.iter().any(|item| item.code == "LAYER008"));
    assert!(diagnostics.iter().any(|item| item.code == "LAYER009"));
    let (diagnostics, _, _) = analyze_snippet(
        r#"
        mod hidden {
            use crate::infrastructure::persistence;
            struct Adapter;
            fn business_logic() {}
        }
        mod unknown;
        fn main() {}
        "#,
        "test-service",
        "services/test-service/src/main.rs",
        Vec::new(),
        None,
        HashMap::new(),
        true,
    )?;
    assert!(
        diagnostics
            .iter()
            .filter(|item| item.code == "LAYER008")
            .count()
            >= 2,
        "{diagnostics:?}"
    );
    let (diagnostics, _, _) = analyze_snippet(
        "#[some_macro::test] mod escape { struct Adapter; } fn main() {}",
        "test-service",
        "services/test-service/src/main.rs",
        Vec::new(),
        None,
        HashMap::new(),
        true,
    )?;
    assert!(diagnostics.iter().any(|item| item.code == "LAYER008"));
    let (diagnostics, _, _) = analyze_snippet(
        r#"
        pub mod domain;
        pub mod application;
        pub mod infrastructure;
        pub mod interface;
        #[cfg(test)] mod tests { struct Helper; }
        "#,
        "test-service",
        "services/test-service/src/lib.rs",
        Vec::new(),
        None,
        HashMap::new(),
        true,
    )?;
    assert!(!diagnostics.iter().any(|item| item.code == "LAYER008"));

    let alias_dependencies = HashMap::new();
    let (diagnostics, _, _) = analyze_snippet(
        r#"
        use crate as root;
        use root::infrastructure::persistence::Repository;
        extern crate self as package_root;
        use package_root::infrastructure::http::Client;
        "#,
        "test-service",
        "services/test-service/src/domain/entities/model.rs",
        vec!["domain".into(), "entities".into()],
        Some(Layer::Domain),
        alias_dependencies,
        true,
    )?;
    assert!(
        diagnostics
            .iter()
            .filter(|item| item.code == "LAYER010")
            .count()
            >= 2,
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .filter(|item| item.code == "LAYER001")
            .count()
            >= 2,
        "{diagnostics:?}"
    );

    let (diagnostics, _, _) = analyze_snippet(
        r#"
        #[cfg_attr(feature = "db", derive(sqlx::Type))]
        #[cfg_attr(feature = "escape", path = "../infrastructure/adapter.rs")]
        #[cfg_attr(feature = "nested", cfg_attr(feature = "db", derive(sqlx::Type)))]
        #[cfg_attr(feature = "unknown", custom(tokens))]
        struct Model;
        "#,
        "test-service",
        "services/test-service/src/domain/entities/model.rs",
        vec!["domain".into(), "entities".into()],
        Some(Layer::Domain),
        HashMap::from([("sqlx".into(), "sqlx".into())]),
        true,
    )?;
    assert!(
        diagnostics
            .iter()
            .filter(|item| item.code == "LAYER002")
            .count()
            >= 2,
        "{diagnostics:?}"
    );
    assert!(diagnostics.iter().any(|item| item.code == "LAYER003"));
    assert!(diagnostics.iter().any(|item| item.code == "LAYER011"));

    let (diagnostics, _, _) = analyze_snippet(
        "#[path = \"escape.rs\"] mod escape; fn main() {}",
        "test-service",
        "services/test-service/src/main.rs",
        Vec::new(),
        None,
        HashMap::new(),
        true,
    )?;
    assert!(diagnostics.iter().any(|item| item.code == "LAYER003"));

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temporary_root = std::env::temp_dir().join(format!(
        "architecture-check-self-{}-{nonce}",
        std::process::id()
    ));
    let gateway_source = temporary_root.join("gateway/src");
    fs::create_dir_all(&gateway_source)?;
    fs::write(
        gateway_source.join("main.rs"),
        r#"
        mod real;
        #[cfg(test)] mod spoof;
        #[cfg(all(test, windows))] mod spoof_all;
        #[cfg(any(test, feature = "hooks"))] mod conditional;
        fn main() {}
        "#,
    )?;
    fs::write(gateway_source.join("real.rs"), "pub fn real() {}")?;
    let spoof = r#"
        fn live() { let _ = StatusCode::OK; }
        fn ready() { let _ = StatusCode::SERVICE_UNAVAILABLE; }
        fn spoof() {
            let _ = std::env::var("DATABASE_URL");
            Router::new()
                .route("/health/live", get(live))
                .route("/health/ready", get(ready))
                .route("/metrics", get(metrics));
            tracing_subscriber::fmt().json().init();
            init_metrics();
            info_span!("request", trace = "x-trace-id");
            serve().with_graceful_shutdown(async {
                tokio::signal::ctrl_c();
                tokio::signal::unix::terminate();
            });
        }
    "#;
    fs::write(gateway_source.join("spoof.rs"), spoof)?;
    fs::write(gateway_source.join("spoof_all.rs"), spoof)?;
    fs::write(
        gateway_source.join("conditional.rs"),
        "pub fn conditional() {}",
    )?;
    fs::write(gateway_source.join("orphan.rs"), spoof)?;
    let closure = module_closure(&gateway_source)?;
    assert_eq!(closure.get(&gateway_source.join("spoof.rs")), Some(&true));
    assert_eq!(
        closure.get(&gateway_source.join("spoof_all.rs")),
        Some(&true)
    );
    assert_eq!(
        closure.get(&gateway_source.join("conditional.rs")),
        Some(&false)
    );
    assert!(!closure.contains_key(&gateway_source.join("orphan.rs")));
    let mut gateway_runtime = runtime("gateway", "gateway");
    gateway_runtime.required_env = vec!["DATABASE_URL".into()];
    let source_audit = audit_sources_with_dependencies(
        &temporary_root,
        std::slice::from_ref(&gateway_runtime),
        &BTreeMap::new(),
    )?;
    let runtime_diagnostics =
        validate_runtime_hooks(&source_audit, std::slice::from_ref(&gateway_runtime));
    for code in ["RUNTIME002", "RUNTIME003", "RUNTIME006", "RUNTIME008"] {
        assert!(runtime_diagnostics.iter().any(|item| item.code == code));
    }

    fs::write(
        gateway_source.join("main.rs"),
        r#"
        mod real;
        fn main() {
            Router::new().route("/health/ready", get(ready));
        }
        "#,
    )?;
    fs::write(
        gateway_source.join("real.rs"),
        "fn ready() { let _ = StatusCode::SERVICE_UNAVAILABLE; }",
    )?;
    let source_audit = audit_sources_with_dependencies(
        &temporary_root,
        std::slice::from_ref(&gateway_runtime),
        &BTreeMap::new(),
    )?;
    let runtime_diagnostics =
        validate_runtime_hooks(&source_audit, std::slice::from_ref(&gateway_runtime));
    assert!(runtime_diagnostics
        .iter()
        .any(|item| item.code == "RUNTIME010"));

    let service_source = temporary_root.join("services/test-service/src");
    fs::create_dir_all(&service_source)?;
    fs::write(service_source.join("main.rs"), "mod escape; fn main() {}")?;
    fs::write(
        service_source.join("escape.rs"),
        "use sqlx::query; fn adapter() { query(\"SELECT id FROM users\"); }",
    )?;
    let service_runtime = runtime("test-service", "services/test-service");
    let source_audit =
        audit_sources_with_dependencies(&temporary_root, &[service_runtime], &BTreeMap::new())?;
    assert!(source_audit
        .diagnostics
        .iter()
        .any(|item| item.code == "LAYER007" && item.path.ends_with("escape.rs")));

    let custom_package = temporary_root.join("services/custom-target-service");
    fs::create_dir_all(custom_package.join("src"))?;
    fs::write(
        custom_package.join("src/lib.rs"),
        "pub fn benign_default_entry() {}",
    )?;
    let custom_target = custom_package.join("evil.rs");
    fs::write(&custom_target, "fn hidden_business_logic() {}")?;
    let custom_runtime = runtime("custom-target-service", "services/custom-target-service");
    let custom_targets = BTreeMap::from([(
        custom_runtime.package.clone(),
        CargoTargets {
            production: vec![custom_target],
            custom_build: Vec::new(),
        },
    )]);
    let source_audit = audit_sources_with_dependencies_and_targets(
        &temporary_root.canonicalize()?,
        std::slice::from_ref(&custom_runtime),
        &BTreeMap::new(),
        &custom_targets,
    )?;
    assert_eq!(source_audit.rust_files, 1);
    assert!(source_audit.diagnostics.iter().any(|item| {
        item.code == "LAYER008"
            && item
                .path
                .ends_with("services/custom-target-service/evil.rs")
    }));
    fs::remove_dir_all(&temporary_root)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn rust_contracts() {
        super::self_test().unwrap();
    }
}
