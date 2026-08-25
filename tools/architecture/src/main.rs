mod rust_source;
mod sql;

use anyhow::{bail, Context, Result};
use rust_source::{Diagnostic, RuntimeSpec};
use serde::Deserialize;
use sql::{AccessKind, ForeignKey};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    version: u32,
    deployment_profile: String,
    runtimes: Vec<RuntimePolicy>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimePolicy {
    package: String,
    root: String,
    liveness_path: String,
    readiness_path: String,
    required_env: Vec<String>,
    allowed_workspace_helpers: Vec<String>,
}

impl RuntimePolicy {
    fn spec(&self) -> RuntimeSpec {
        RuntimeSpec {
            package: self.package.clone(),
            root: self.root.clone(),
            liveness_path: self.liveness_path.clone(),
            readiness_path: self.readiness_path.clone(),
            required_env: self.required_env.clone(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnershipPolicy {
    version: u32,
    schema: String,
    relations: Vec<RelationRule>,
    routines: Vec<RoutineRule>,
    acknowledged_cross_owner_foreign_keys: Vec<ForeignKey>,
    acknowledged_cross_owner_trigger_bindings: Vec<TriggerBindingException>,
    acknowledged_cross_owner_trigger_accesses: Vec<TriggerAccessException>,
    acknowledged_migration_audit_debts: Vec<MigrationAuditDebt>,
    access_exceptions: Vec<AccessException>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelationRule {
    name: String,
    kind: String,
    owner: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineRule {
    name: String,
    owner: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerAccessException {
    id: String,
    trigger: String,
    on_relation: String,
    function: String,
    relation: String,
    access: String,
    trigger_definition_sha256: String,
    body_sha256: String,
    reason: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerBindingException {
    id: String,
    trigger: String,
    on_relation: String,
    function: String,
    trigger_definition_sha256: String,
    body_sha256: String,
    reason: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationAuditDebt {
    path: String,
    source_sha256: String,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessException {
    id: String,
    service: String,
    path: String,
    function: String,
    relation: String,
    access: String,
    sql_sha256: String,
    reason: String,
}

fn main() -> Result<()> {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("check") => {
            let root = arguments
                .next()
                .map(PathBuf::from)
                .unwrap_or(env::current_dir().context("resolve current directory")?);
            if arguments.next().is_some() {
                bail!("usage: architecture-check check [repository-root]");
            }
            run_check(&root)
        }
        Some("self-test") => {
            if arguments.next().is_some() {
                bail!("usage: architecture-check self-test");
            }
            rust_source::self_test()?;
            sql::self_test()?;
            println!("architecture-check self-test: ok");
            Ok(())
        }
        _ => bail!("usage: architecture-check <check [repository-root] | self-test>"),
    }
}

fn run_check(root: &Path) -> Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize repository root {}", root.display()))?;
    let policy: Policy = read_json(&root.join("tools/architecture/policy-v1.json"))?;
    let ownership: OwnershipPolicy =
        read_json(&root.join("tools/architecture/table-ownership-v1.json"))?;
    validate_policy(&policy, &ownership)?;

    let runtimes = policy
        .runtimes
        .iter()
        .map(RuntimePolicy::spec)
        .collect::<Vec<_>>();
    let workspace_helpers = policy
        .runtimes
        .iter()
        .map(|runtime| {
            (
                runtime.package.clone(),
                runtime
                    .allowed_workspace_helpers
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<rust_source::WorkspaceHelperPolicy>();

    let mut diagnostics =
        rust_source::audit_cargo_with_workspace_helpers(&root, &runtimes, &workspace_helpers)?;
    let source_audit = rust_source::audit_sources(&root, &runtimes)?;
    diagnostics.extend(source_audit.diagnostics.clone());
    diagnostics.extend(rust_source::validate_runtime_hooks(
        &source_audit,
        &runtimes,
    ));

    let owner_map = ownership
        .relations
        .iter()
        .map(|relation| (relation.name.as_str(), relation.owner.as_str()))
        .collect::<BTreeMap<_, _>>();
    let routine_owner_map = ownership
        .routines
        .iter()
        .map(|routine| (routine.name.as_str(), routine.owner.as_str()))
        .collect::<BTreeMap<_, _>>();
    let canonical_sql = fs::read_to_string(root.join("infra/postgres/init.sql"))
        .context("read canonical PostgreSQL schema")?;
    let schema = sql::canonical_schema_snapshot(&canonical_sql)
        .context("strictly audit canonical schema")?;
    check_schema_inventory(&root, &schema, &ownership, &owner_map, &mut diagnostics)?;
    check_foreign_key_debt(
        &schema.foreign_keys,
        &ownership,
        &owner_map,
        &mut diagnostics,
    );
    check_schema_dependencies(
        &schema,
        &ownership,
        &owner_map,
        &routine_owner_map,
        &mut diagnostics,
    );
    check_sql_ownership(
        &source_audit.sql_calls,
        &ownership,
        &owner_map,
        &routine_owner_map,
        &schema,
        &mut diagnostics,
    );

    diagnostics.sort();
    diagnostics.dedup();
    for diagnostic in &diagnostics {
        eprintln!(
            "{} {}:{} {}",
            diagnostic.code, diagnostic.path, diagnostic.line, diagnostic.message
        );
    }

    let cross_owner_count = schema
        .foreign_keys
        .iter()
        .filter(|foreign_key| {
            owner_map.get(foreign_key.child.as_str()) != owner_map.get(foreign_key.parent.as_str())
        })
        .count();
    println!(
        "architecture-check: {} Rust files ({} domain, {} application), {} Rust path references analyzed",
        source_audit.rust_files,
        source_audit.domain_files,
        source_audit.application_files,
        source_audit.explicit_references
    );
    println!(
        "architecture-check: {} tables + {} views, {} routines, {} triggers, {} foreign keys; {} acknowledged cross-owner FK debts + {} trigger access debts + {} trigger binding debts + {} migration audit debts",
        schema.tables.len(),
        schema.views.len(),
        schema.routines.len(),
        schema.triggers.len(),
        schema.foreign_keys.len(),
        cross_owner_count,
        ownership.acknowledged_cross_owner_trigger_accesses.len(),
        ownership.acknowledged_cross_owner_trigger_bindings.len(),
        ownership.acknowledged_migration_audit_debts.len()
    );
    println!(
        "architecture-check: {} runtime hook sets under {}",
        policy.runtimes.len(),
        policy.deployment_profile
    );
    println!(
        "architecture-check: static source evidence only; shared DB roles, multi-replica safety, drain completion, collectors, alerts, and runtime behavior are not proven"
    );
    if diagnostics.is_empty() {
        println!("architecture-check: 0 blocking violations");
        Ok(())
    } else {
        bail!(
            "architecture-check found {} blocking violation(s)",
            diagnostics.len()
        )
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

fn validate_policy(policy: &Policy, ownership: &OwnershipPolicy) -> Result<()> {
    if policy.version != 1 || ownership.version != 1 {
        bail!("architecture policy versions must be 1");
    }
    if policy.deployment_profile != "private-single-node-v1" {
        bail!("the checked deployment profile must remain private-single-node-v1");
    }
    if ownership.schema != "public" {
        bail!("only the canonical public application schema is supported");
    }
    let expected = BTreeSet::from([
        "gateway",
        "user-service",
        "novel-service",
        "agent-service",
        "narrative-service",
    ]);
    let actual = policy
        .runtimes
        .iter()
        .map(|runtime| runtime.package.as_str())
        .collect::<BTreeSet<_>>();
    if actual != expected || policy.runtimes.len() != expected.len() {
        bail!("runtime policy must name exactly the five current runtimes");
    }
    let runtime_roots = policy
        .runtimes
        .iter()
        .map(|runtime| runtime.root.as_str())
        .collect::<HashSet<_>>();
    if runtime_roots.len() != policy.runtimes.len() {
        bail!("runtime policy roots must be unique");
    }
    for runtime in &policy.runtimes {
        let config_keys = runtime.required_env.iter().collect::<HashSet<_>>();
        let helpers = runtime
            .allowed_workspace_helpers
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if runtime.root.trim().is_empty()
            || !runtime.liveness_path.starts_with('/')
            || !runtime.readiness_path.starts_with('/')
            || runtime.liveness_path == runtime.readiness_path
            || runtime.required_env.is_empty()
            || config_keys.len() != runtime.required_env.len()
            || helpers.len() != runtime.allowed_workspace_helpers.len()
            || helpers
                .iter()
                .any(|helper| helper.trim().is_empty() || expected.contains(helper))
        {
            bail!("runtime policy for {} is incomplete", runtime.package);
        }
    }
    let relation_names = ownership
        .relations
        .iter()
        .map(|relation| relation.name.as_str())
        .collect::<HashSet<_>>();
    if relation_names.len() != ownership.relations.len() {
        bail!("table ownership policy contains duplicate relations");
    }
    if ownership
        .relations
        .iter()
        .any(|relation| !matches!(relation.kind.as_str(), "table" | "view"))
    {
        bail!("relation kind must be table or view");
    }
    let allowed_owners = expected
        .iter()
        .copied()
        .chain(std::iter::once("platform"))
        .collect::<HashSet<_>>();
    if ownership.relations.iter().any(|relation| {
        relation.name.trim().is_empty() || !allowed_owners.contains(relation.owner.as_str())
    }) {
        bail!("every relation must have a known, non-empty owner");
    }
    let routine_names = ownership
        .routines
        .iter()
        .map(|routine| routine.name.as_str())
        .collect::<HashSet<_>>();
    if routine_names.len() != ownership.routines.len()
        || ownership.routines.iter().any(|routine| {
            routine.name.trim().is_empty() || !allowed_owners.contains(routine.owner.as_str())
        })
    {
        bail!("routine ownership policy contains a duplicate, empty, or unknown owner");
    }
    let acknowledged = ownership
        .acknowledged_cross_owner_foreign_keys
        .iter()
        .collect::<BTreeSet<_>>();
    if acknowledged.len() != ownership.acknowledged_cross_owner_foreign_keys.len() {
        bail!("acknowledged cross-owner foreign keys must be unique");
    }
    let relation_owners = ownership
        .relations
        .iter()
        .map(|relation| (relation.name.as_str(), relation.owner.as_str()))
        .collect::<BTreeMap<_, _>>();
    for foreign_key in &ownership.acknowledged_cross_owner_foreign_keys {
        let Some(child_owner) = relation_owners.get(foreign_key.child.as_str()) else {
            bail!("acknowledged FK child {} has no owner", foreign_key.child);
        };
        let Some(parent_owner) = relation_owners.get(foreign_key.parent.as_str()) else {
            bail!("acknowledged FK parent {} has no owner", foreign_key.parent);
        };
        if child_owner == parent_owner
            || foreign_key.child_columns.is_empty()
            || foreign_key.parent_columns.is_empty()
            || foreign_key.on_delete.trim().is_empty()
        {
            bail!(
                "acknowledged FK {} -> {} is not cross-owner or is incomplete",
                foreign_key.child,
                foreign_key.parent
            );
        }
    }
    let exception_ids = ownership
        .access_exceptions
        .iter()
        .map(|exception| exception.id.as_str())
        .collect::<HashSet<_>>();
    if exception_ids.len() != ownership.access_exceptions.len() {
        bail!("access exception ids must be unique");
    }
    for exception in &ownership.access_exceptions {
        let runtime = policy
            .runtimes
            .iter()
            .find(|runtime| runtime.package == exception.service)
            .ok_or_else(|| {
                anyhow::anyhow!("access exception {} names an unknown service", exception.id)
            })?;
        let expected_path_prefix = format!("{}/src/", runtime.root);
        let relation_owner = relation_owners.get(exception.relation.as_str());
        if !matches!(exception.access.as_str(), "read" | "write" | "metadata")
            || !is_lower_hex_hash(&exception.sql_sha256)
            || exception.reason.trim().is_empty()
            || exception.function.trim().is_empty()
            || !exception.path.starts_with(&expected_path_prefix)
            || exception.path.contains("..")
            || relation_owner.is_none()
            || relation_owner.copied() == Some(exception.service.as_str())
        {
            bail!("access exception {} is incomplete", exception.id);
        }
    }
    let trigger_exception_ids = ownership
        .acknowledged_cross_owner_trigger_accesses
        .iter()
        .map(|exception| exception.id.as_str())
        .collect::<HashSet<_>>();
    if trigger_exception_ids.len() != ownership.acknowledged_cross_owner_trigger_accesses.len() {
        bail!("cross-owner trigger exception ids must be unique");
    }
    for exception in &ownership.acknowledged_cross_owner_trigger_accesses {
        if exception.id.trim().is_empty()
            || exception.trigger.trim().is_empty()
            || !relation_names.contains(exception.on_relation.as_str())
            || !routine_names.contains(exception.function.as_str())
            || !relation_names.contains(exception.relation.as_str())
            || !matches!(exception.access.as_str(), "read" | "write" | "metadata")
            || !is_lower_hex_hash(&exception.trigger_definition_sha256)
            || !is_lower_hex_hash(&exception.body_sha256)
            || exception.reason.trim().is_empty()
        {
            bail!(
                "cross-owner trigger exception {} is incomplete",
                exception.id
            );
        }
    }
    let trigger_binding_ids = ownership
        .acknowledged_cross_owner_trigger_bindings
        .iter()
        .map(|exception| exception.id.as_str())
        .collect::<HashSet<_>>();
    if trigger_binding_ids.len() != ownership.acknowledged_cross_owner_trigger_bindings.len() {
        bail!("cross-owner trigger binding exception ids must be unique");
    }
    let routine_owners = ownership
        .routines
        .iter()
        .map(|routine| (routine.name.as_str(), routine.owner.as_str()))
        .collect::<BTreeMap<_, _>>();
    for exception in &ownership.acknowledged_cross_owner_trigger_bindings {
        if exception.id.trim().is_empty()
            || exception.trigger.trim().is_empty()
            || !relation_names.contains(exception.on_relation.as_str())
            || !routine_names.contains(exception.function.as_str())
            || !is_lower_hex_hash(&exception.trigger_definition_sha256)
            || !is_lower_hex_hash(&exception.body_sha256)
            || exception.reason.trim().is_empty()
            || relation_owners.get(exception.on_relation.as_str())
                == routine_owners.get(exception.function.as_str())
        {
            bail!(
                "cross-owner trigger binding exception {} is incomplete",
                exception.id
            );
        }
    }
    let migration_paths = ownership
        .acknowledged_migration_audit_debts
        .iter()
        .map(|debt| debt.path.as_str())
        .collect::<HashSet<_>>();
    if migration_paths.len() != ownership.acknowledged_migration_audit_debts.len() {
        bail!("migration audit debt paths must be unique");
    }
    for debt in &ownership.acknowledged_migration_audit_debts {
        if !debt.path.starts_with("infra/postgres/migrations/")
            || !debt.path.ends_with(".sql")
            || debt.path.contains("..")
            || debt.path.contains('\\')
            || !is_lower_hex_hash(&debt.source_sha256)
            || debt.reason.trim().is_empty()
        {
            bail!("migration audit debt {} is incomplete", debt.path);
        }
    }
    Ok(())
}

fn is_lower_hex_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn check_schema_inventory(
    root: &Path,
    schema: &sql::SchemaSnapshot,
    ownership: &OwnershipPolicy,
    owner_map: &BTreeMap<&str, &str>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<()> {
    let declared_tables = ownership
        .relations
        .iter()
        .filter(|relation| relation.kind == "table")
        .map(|relation| relation.name.clone())
        .collect::<BTreeSet<_>>();
    let declared_views = ownership
        .relations
        .iter()
        .filter(|relation| relation.kind == "view")
        .map(|relation| relation.name.clone())
        .collect::<BTreeSet<_>>();
    for relation in schema.tables.difference(&declared_tables) {
        diagnostics.push(schema_diagnostic(
            "SCHEMA001",
            format!("table {relation} has no declared owner"),
        ));
    }
    for relation in declared_tables.difference(&schema.tables) {
        diagnostics.push(schema_diagnostic(
            "SCHEMA002",
            format!("declared table {relation} is absent from init.sql"),
        ));
    }
    for relation in schema.views.difference(&declared_views) {
        diagnostics.push(schema_diagnostic(
            "SCHEMA003",
            format!("view {relation} has no declared owner"),
        ));
    }
    for relation in declared_views.difference(&schema.views) {
        diagnostics.push(schema_diagnostic(
            "SCHEMA004",
            format!("declared view {relation} is absent from init.sql"),
        ));
    }

    let declared_routines = ownership
        .routines
        .iter()
        .map(|routine| routine.name.clone())
        .collect::<BTreeSet<_>>();
    let canonical_routines = schema
        .routines
        .iter()
        .map(|routine| routine.name.clone())
        .collect::<BTreeSet<_>>();
    for routine in canonical_routines.difference(&declared_routines) {
        diagnostics.push(schema_diagnostic(
            "SCHEMA006",
            format!("routine {routine} has no declared owner"),
        ));
    }
    for routine in declared_routines.difference(&canonical_routines) {
        diagnostics.push(schema_diagnostic(
            "SCHEMA007",
            format!("declared routine {routine} is absent from init.sql"),
        ));
    }

    let migrations = root.join("infra/postgres/migrations");
    let canonical_routine_definitions = schema
        .routines
        .iter()
        .map(|routine| (routine.name.as_str(), routine))
        .collect::<BTreeMap<_, _>>();
    let canonical_view_definitions = schema
        .view_definitions
        .iter()
        .map(|view| (view.name.as_str(), view))
        .collect::<BTreeMap<_, _>>();
    let routine_owners = ownership
        .routines
        .iter()
        .map(|routine| (routine.name.as_str(), routine.owner.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut used_migration_debts = HashSet::new();
    let mut files = fs::read_dir(&migrations)
        .with_context(|| format!("read {}", migrations.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort();
    for path in files {
        if path.extension().and_then(|extension| extension.to_str()) != Some("sql") {
            continue;
        }
        let source = fs::read_to_string(&path)
            .with_context(|| format!("read migration {}", path.display()))?;
        let relative = rust_source::normalize_path(path.strip_prefix(root).unwrap_or(&path));
        let source_sha256 = sql::fingerprint(&source);
        let exact_debt = ownership
            .acknowledged_migration_audit_debts
            .iter()
            .find(|debt| debt.path == relative && debt.source_sha256 == source_sha256);
        let migration = match sql::migration_snapshot(&source) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if let Some(debt) = exact_debt {
                    used_migration_debts.insert(debt.path.as_str());
                    sql::schema_snapshot(&source).with_context(|| {
                        format!("parse acknowledged migration debt {}", path.display())
                    })?
                } else {
                    diagnostics.push(Diagnostic {
                        code: "MIG001",
                        path: relative.clone(),
                        line: 1,
                        message: format!("migration cannot be audited safely: {error}"),
                    });
                    continue;
                }
            }
        };
        for relation in migration.tables.iter().chain(&migration.views) {
            if !owner_map.contains_key(relation.as_str()) {
                diagnostics.push(Diagnostic {
                    code: "SCHEMA005",
                    path: relative.clone(),
                    line: 1,
                    message: format!("migration creates unowned relation {relation}"),
                });
            }
        }
        let mut drift = migration_drop_drift(&migration, schema);
        for view in &migration.view_definitions {
            if canonical_view_definitions.get(view.name.as_str()).copied() != Some(view) {
                drift.push(format!(
                    "view {} definition (relation accesses/function calls) differs from canonical init.sql",
                    view.name
                ));
            }
            if let Some(view_owner) = owner_map.get(view.name.as_str()).copied() {
                for access in &view.direct_accesses {
                    if owner_map.get(access.relation.as_str()).copied() != Some(view_owner) {
                        drift.push(format!(
                            "{}-owned migration view {} {} accesses differently owned or unowned relation {}",
                            view_owner,
                            view.name,
                            access.kind.as_str(),
                            access.relation
                        ));
                    }
                }
                for called in &view.called_functions {
                    if let Some(called_owner) = routine_owners.get(called.as_str()).copied() {
                        if called_owner != view_owner {
                            drift.push(format!(
                                "{}-owned migration view {} calls {}-owned routine {}",
                                view_owner, view.name, called_owner, called
                            ));
                        }
                    }
                }
            }
        }
        for foreign_key in &migration.foreign_keys {
            if !schema.foreign_keys.contains(foreign_key) {
                drift.push(format!(
                    "foreign key {}({}) -> {}({}) ON DELETE {} differs from canonical init.sql",
                    foreign_key.child,
                    foreign_key.child_columns.join(","),
                    foreign_key.parent,
                    foreign_key.parent_columns.join(","),
                    foreign_key.on_delete
                ));
            }
        }
        for routine in &migration.routines {
            if canonical_routine_definitions
                .get(routine.name.as_str())
                .copied()
                != Some(routine)
            {
                drift.push(format!(
                    "routine {} definition (language/body/access/calls) differs from canonical init.sql",
                    routine.name
                ));
            }
        }
        for trigger in &migration.triggers {
            if !schema.triggers.contains(trigger) {
                drift.push(format!(
                    "trigger {} ON {} EXECUTE {} differs from canonical init.sql",
                    trigger.name, trigger.on_relation, trigger.function
                ));
            }
        }
        if !drift.is_empty() {
            if let Some(debt) = exact_debt {
                used_migration_debts.insert(debt.path.as_str());
            } else {
                diagnostics.extend(drift.into_iter().map(|message| Diagnostic {
                    code: "MIG002",
                    path: relative.clone(),
                    line: 1,
                    message,
                }));
            }
        }
    }
    for debt in &ownership.acknowledged_migration_audit_debts {
        if !used_migration_debts.contains(debt.path.as_str()) {
            diagnostics.push(Diagnostic {
                code: "MIG003",
                path: debt.path.clone(),
                line: 1,
                message:
                    "migration audit debt is stale, missing, or its full-source SHA-256 changed"
                        .into(),
            });
        }
    }
    Ok(())
}

fn migration_drop_drift(
    migration: &sql::SchemaSnapshot,
    canonical: &sql::SchemaSnapshot,
) -> Vec<String> {
    let canonical_views = canonical
        .view_definitions
        .iter()
        .map(|view| (view.name.as_str(), view))
        .collect::<BTreeMap<_, _>>();
    let canonical_triggers = canonical
        .triggers
        .iter()
        .map(|trigger| {
            (
                (trigger.name.as_str(), trigger.on_relation.as_str()),
                trigger,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut drift = Vec::new();

    for dropped in &migration.dropped_views {
        let recreated_exactly = migration.recreated_dropped_views.contains(dropped)
            && canonical_views
                .get(dropped.as_str())
                .is_some_and(|canonical| migration.view_definitions.contains(*canonical));
        if !recreated_exactly {
            drift.push(format!(
                "dropped view {dropped} is not subsequently recreated with its exact canonical definition"
            ));
        }
    }
    for dropped in &migration.dropped_triggers {
        let recreated_exactly = migration.recreated_dropped_triggers.contains(dropped)
            && canonical_triggers
                .get(&(dropped.name.as_str(), dropped.on_relation.as_str()))
                .is_some_and(|canonical| migration.triggers.contains(*canonical));
        if !recreated_exactly {
            drift.push(format!(
                "dropped trigger {} ON {} is not subsequently recreated with its exact canonical binding",
                dropped.name, dropped.on_relation
            ));
        }
    }
    drift
}

fn check_foreign_key_debt(
    foreign_keys: &BTreeSet<ForeignKey>,
    ownership: &OwnershipPolicy,
    owner_map: &BTreeMap<&str, &str>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let actual = foreign_keys
        .iter()
        .filter(|foreign_key| {
            owner_map.get(foreign_key.child.as_str()) != owner_map.get(foreign_key.parent.as_str())
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let acknowledged = ownership
        .acknowledged_cross_owner_foreign_keys
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    for foreign_key in actual.difference(&acknowledged) {
        diagnostics.push(schema_diagnostic(
            "FK001",
            format!(
                "new cross-owner FK {}({}) -> {}({}) ON DELETE {}",
                foreign_key.child,
                foreign_key.child_columns.join(","),
                foreign_key.parent,
                foreign_key.parent_columns.join(","),
                foreign_key.on_delete
            ),
        ));
    }
    for foreign_key in acknowledged.difference(&actual) {
        diagnostics.push(schema_diagnostic(
            "FK002",
            format!(
                "stale cross-owner FK debt {}({}) -> {}({}) ON DELETE {}",
                foreign_key.child,
                foreign_key.child_columns.join(","),
                foreign_key.parent,
                foreign_key.parent_columns.join(","),
                foreign_key.on_delete
            ),
        ));
    }
}

fn check_schema_dependencies(
    schema: &sql::SchemaSnapshot,
    ownership: &OwnershipPolicy,
    owner_map: &BTreeMap<&str, &str>,
    routine_owner_map: &BTreeMap<&str, &str>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let routine_names = schema
        .routines
        .iter()
        .map(|routine| routine.name.clone())
        .collect::<BTreeSet<_>>();
    let routines = schema
        .routines
        .iter()
        .map(|routine| (routine.name.as_str(), routine))
        .collect::<BTreeMap<_, _>>();
    let mut effective_accesses = schema
        .routines
        .iter()
        .map(|routine| (routine.name.clone(), routine.direct_accesses.clone()))
        .collect::<BTreeMap<_, _>>();

    loop {
        let previous = effective_accesses.clone();
        let mut changed = false;
        for routine in &schema.routines {
            let accesses = effective_accesses.entry(routine.name.clone()).or_default();
            for called in routine
                .called_functions
                .intersection(&routine_names)
                .filter_map(|called| previous.get(called))
            {
                let before = accesses.len();
                accesses.extend(called.iter().cloned());
                changed |= accesses.len() != before;
            }
        }
        if !changed {
            break;
        }
    }

    for routine in &schema.routines {
        let Some(routine_owner) = routine_owner_map.get(routine.name.as_str()).copied() else {
            continue;
        };
        for called in routine.called_functions.intersection(&routine_names) {
            if routine_owner_map.get(called.as_str()).copied() != Some(routine_owner) {
                diagnostics.push(schema_diagnostic(
                    "DBFUNC001",
                    format!(
                        "{}-owned routine {} calls differently owned routine {}",
                        routine_owner, routine.name, called
                    ),
                ));
            }
        }
        for access in effective_accesses.get(&routine.name).into_iter().flatten() {
            match owner_map.get(access.relation.as_str()).copied() {
                None => diagnostics.push(schema_diagnostic(
                    "DBFUNC002",
                    format!(
                        "routine {} accesses unowned relation {} ({})",
                        routine.name,
                        access.relation,
                        access.kind.as_str()
                    ),
                )),
                Some(relation_owner) if relation_owner != routine_owner => {
                    diagnostics.push(schema_diagnostic(
                        "DBFUNC003",
                        format!(
                            "{}-owned routine {} {} accesses {}-owned relation {}",
                            routine_owner,
                            routine.name,
                            access.kind.as_str(),
                            relation_owner,
                            access.relation
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
    }

    for view in &schema.view_definitions {
        let Some(view_owner) = owner_map.get(view.name.as_str()).copied() else {
            continue;
        };
        let mut accesses = view.direct_accesses.clone();
        for called in view.called_functions.intersection(&routine_names) {
            if routine_owner_map.get(called.as_str()).copied() != Some(view_owner) {
                diagnostics.push(schema_diagnostic(
                    "DBVIEW001",
                    format!(
                        "{}-owned view {} calls differently owned routine {}",
                        view_owner, view.name, called
                    ),
                ));
            }
            if let Some(called_accesses) = effective_accesses.get(called) {
                accesses.extend(called_accesses.iter().cloned());
            }
        }
        for access in accesses {
            match owner_map.get(access.relation.as_str()).copied() {
                None => diagnostics.push(schema_diagnostic(
                    "DBVIEW002",
                    format!(
                        "view {} accesses unowned relation {} ({})",
                        view.name,
                        access.relation,
                        access.kind.as_str()
                    ),
                )),
                Some(relation_owner) if relation_owner != view_owner => {
                    diagnostics.push(schema_diagnostic(
                        "DBVIEW003",
                        format!(
                            "{}-owned view {} {} accesses {}-owned relation {}",
                            view_owner,
                            view.name,
                            access.kind.as_str(),
                            relation_owner,
                            access.relation
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
    }

    let mut used_exceptions = HashSet::new();
    let mut used_binding_exceptions = HashSet::new();
    for trigger in &schema.triggers {
        let Some(on_owner) = owner_map.get(trigger.on_relation.as_str()).copied() else {
            diagnostics.push(schema_diagnostic(
                "DBTRIGGER001",
                format!(
                    "trigger {} is attached to unowned relation {}",
                    trigger.name, trigger.on_relation
                ),
            ));
            continue;
        };
        let Some(routine) = routines.get(trigger.function.as_str()).copied() else {
            diagnostics.push(schema_diagnostic(
                "DBTRIGGER002",
                format!(
                    "trigger {} references missing routine {}",
                    trigger.name, trigger.function
                ),
            ));
            continue;
        };
        let routine_owner = routine_owner_map
            .get(trigger.function.as_str())
            .copied()
            .unwrap_or("<unowned>");
        if routine_owner != on_owner {
            let matching = exact_trigger_binding_exception(
                &ownership.acknowledged_cross_owner_trigger_bindings,
                trigger,
                &routine.body_sha256,
            );
            if let Some(exception) = matching {
                used_binding_exceptions.insert(exception.id.as_str());
            } else {
                diagnostics.push(schema_diagnostic(
                    "DBTRIGGER005",
                    format!(
                        "{}-owned trigger {} on {} attaches differently owned routine {} ({}) without an exact binding debt (trigger_definition_sha256 {}, body_sha256 {})",
                        on_owner,
                        trigger.name,
                        trigger.on_relation,
                        trigger.function,
                        routine_owner,
                        trigger.definition_sha256,
                        routine.body_sha256
                    ),
                ));
            }
        }
        for access in effective_accesses.get(&routine.name).into_iter().flatten() {
            let Some(relation_owner) = owner_map.get(access.relation.as_str()).copied() else {
                continue;
            };
            if relation_owner == on_owner {
                continue;
            }
            let matching = ownership
                .acknowledged_cross_owner_trigger_accesses
                .iter()
                .find(|exception| {
                    exception.trigger == trigger.name
                        && exception.on_relation == trigger.on_relation
                        && exception.function == trigger.function
                        && exception.relation == access.relation
                        && exception.access == access.kind.as_str()
                        && exception.trigger_definition_sha256 == trigger.definition_sha256
                        && exception.body_sha256 == routine.body_sha256
                });
            if let Some(exception) = matching {
                used_exceptions.insert(exception.id.as_str());
            } else {
                diagnostics.push(schema_diagnostic(
                    "DBTRIGGER003",
                    format!(
                        "{}-owned trigger {} on {} {} accesses {}-owned relation {} through {} (trigger_definition_sha256 {}, body_sha256 {}) without an exact debt entry",
                        on_owner,
                        trigger.name,
                        trigger.on_relation,
                        access.kind.as_str(),
                        relation_owner,
                        access.relation,
                        trigger.function,
                        trigger.definition_sha256,
                        routine.body_sha256
                    ),
                ));
            }
        }
    }
    for exception in &ownership.acknowledged_cross_owner_trigger_accesses {
        if !used_exceptions.contains(exception.id.as_str()) {
            diagnostics.push(schema_diagnostic(
                "DBTRIGGER004",
                format!(
                    "cross-owner trigger exception {} is stale or no longer exact",
                    exception.id
                ),
            ));
        }
    }
    for exception in &ownership.acknowledged_cross_owner_trigger_bindings {
        if !used_binding_exceptions.contains(exception.id.as_str()) {
            diagnostics.push(schema_diagnostic(
                "DBTRIGGER006",
                format!(
                    "cross-owner trigger binding exception {} is stale or no longer exact",
                    exception.id
                ),
            ));
        }
    }
}

fn exact_trigger_binding_exception<'a>(
    exceptions: &'a [TriggerBindingException],
    trigger: &sql::TriggerBinding,
    body_sha256: &str,
) -> Option<&'a TriggerBindingException> {
    exceptions.iter().find(|exception| {
        exception.trigger == trigger.name
            && exception.on_relation == trigger.on_relation
            && exception.function == trigger.function
            && exception.trigger_definition_sha256 == trigger.definition_sha256
            && exception.body_sha256 == body_sha256
    })
}

fn check_sql_ownership(
    calls: &[rust_source::SqlCall],
    ownership: &OwnershipPolicy,
    owner_map: &BTreeMap<&str, &str>,
    routine_owner_map: &BTreeMap<&str, &str>,
    schema: &sql::SchemaSnapshot,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let known_routines = schema
        .routines
        .iter()
        .map(|routine| routine.name.clone())
        .collect::<BTreeSet<_>>();
    let mut used_exceptions = HashSet::new();
    for call in calls {
        let fingerprint = sql::fingerprint(&call.sql);
        match sql::routine_calls(&call.sql, &known_routines) {
            Ok(routines) => {
                for routine in routines {
                    if routine_owner_map.get(routine.as_str()).copied()
                        != Some(call.service.as_str())
                    {
                        diagnostics.push(Diagnostic {
                            code: "OWN004",
                            path: call.path.clone(),
                            line: call.line,
                            message: format!(
                                "{} SQL calls differently owned routine {} in function {}",
                                call.service, routine, call.function
                            ),
                        });
                    }
                }
            }
            Err(error) => diagnostics.push(Diagnostic {
                code: "SQL004",
                path: call.path.clone(),
                line: call.line,
                message: format!("cannot inspect SQL routine calls: {error}"),
            }),
        }
        let accesses = match sql::relation_accesses(&call.sql) {
            Ok(accesses) => accesses,
            Err(error) => {
                diagnostics.push(Diagnostic {
                    code: "SQL003",
                    path: call.path.clone(),
                    line: call.line,
                    message: format!("cannot inspect SQL literal: {error}"),
                });
                continue;
            }
        };
        for access in accesses {
            let Some(owner) = owner_map.get(access.relation.as_str()) else {
                diagnostics.push(Diagnostic {
                    code: "OWN001",
                    path: call.path.clone(),
                    line: call.line,
                    message: format!(
                        "SQL references unowned relation {} ({})",
                        access.relation,
                        access.kind.as_str()
                    ),
                });
                continue;
            };
            if *owner == call.service {
                continue;
            }
            let matching = ownership.access_exceptions.iter().find(|exception| {
                exception.service == call.service
                    && exception.path == call.path
                    && exception.function == call.function
                    && exception.relation == access.relation
                    && exception.access == access.kind.as_str()
                    && exception.sql_sha256 == fingerprint
            });
            if let Some(exception) = matching {
                used_exceptions.insert(exception.id.as_str());
            } else {
                diagnostics.push(Diagnostic {
                    code: "OWN002",
                    path: call.path.clone(),
                    line: call.line,
                    message: format!(
                        "{} {} access to {}-owned relation {} has no exact exception (function {}, sql_sha256 {})",
                        call.service,
                        access.kind.as_str(),
                        owner,
                        access.relation,
                        call.function,
                        fingerprint
                    ),
                });
            }
        }
    }
    for exception in &ownership.access_exceptions {
        if !used_exceptions.contains(exception.id.as_str()) {
            diagnostics.push(Diagnostic {
                code: "OWN003",
                path: exception.path.clone(),
                line: 1,
                message: format!(
                    "access exception {} is stale or no longer exact",
                    exception.id
                ),
            });
        }
    }
}

fn schema_diagnostic(code: &'static str, message: String) -> Diagnostic {
    Diagnostic {
        code,
        path: "infra/postgres/init.sql".into(),
        line: 1,
        message,
    }
}

#[allow(dead_code)]
fn _assert_access_kind_is_exhaustive(kind: AccessKind) -> &'static str {
    kind.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_owner_trigger_binding_requires_exact_definition_and_body() {
        let definition_sha256 = "a".repeat(64);
        let body_sha256 = "b".repeat(64);
        let exception = TriggerBindingException {
            id: "exact-binding".into(),
            trigger: "touch_user".into(),
            on_relation: "users".into(),
            function: "platform_touch".into(),
            trigger_definition_sha256: definition_sha256.clone(),
            body_sha256: body_sha256.clone(),
            reason: "exact cross-owner binding".into(),
        };
        let mut trigger = sql::TriggerBinding {
            name: "touch_user".into(),
            on_relation: "users".into(),
            function: "platform_touch".into(),
            timing: "before".into(),
            events: BTreeSet::from(["update".into()]),
            for_each: "row".into(),
            when_sha256: None,
            argument_sha256: Vec::new(),
            definition_sha256,
        };
        assert!(exact_trigger_binding_exception(
            std::slice::from_ref(&exception),
            &trigger,
            &body_sha256
        )
        .is_some());

        trigger.definition_sha256 = "c".repeat(64);
        assert!(exact_trigger_binding_exception(
            std::slice::from_ref(&exception),
            &trigger,
            &body_sha256
        )
        .is_none());
        trigger.definition_sha256 = exception.trigger_definition_sha256.clone();
        assert!(exact_trigger_binding_exception(&[exception], &trigger, &"d".repeat(64)).is_none());
    }

    #[test]
    fn dropped_views_and_triggers_require_later_exact_recreation() -> Result<()> {
        let canonical = sql::canonical_schema_snapshot(
            r#"
            CREATE TABLE public.users (id UUID PRIMARY KEY);
            CREATE VIEW public.user_shelf AS SELECT id FROM public.users;
            CREATE FUNCTION public.touch_user()
            RETURNS TRIGGER LANGUAGE plpgsql AS $$
            BEGIN
              RETURN NEW;
            END
            $$;
            CREATE TRIGGER touch_user
              BEFORE UPDATE ON public.users
              FOR EACH ROW EXECUTE FUNCTION public.touch_user();
            "#,
        )?;
        let drop_only = sql::migration_snapshot(
            "DROP VIEW IF EXISTS public.user_shelf; \
             DROP TRIGGER IF EXISTS touch_user ON public.users;",
        )?;
        assert_eq!(
            migration_drop_drift(&drop_only, &canonical).len(),
            2,
            "drop-only migrations must fail closed"
        );

        let recreated = sql::migration_snapshot(
            r#"
            DROP VIEW IF EXISTS public.user_shelf;
            CREATE VIEW public.user_shelf AS SELECT id FROM public.users;
            DROP TRIGGER IF EXISTS touch_user ON public.users;
            CREATE TRIGGER touch_user
              BEFORE UPDATE ON public.users
              FOR EACH ROW EXECUTE FUNCTION public.touch_user();
            "#,
        )?;
        assert!(migration_drop_drift(&recreated, &canonical).is_empty());

        let recreated_before_drop = sql::migration_snapshot(
            "CREATE VIEW public.user_shelf AS SELECT id FROM public.users; \
             DROP VIEW IF EXISTS public.user_shelf;",
        )?;
        assert_eq!(
            migration_drop_drift(&recreated_before_drop, &canonical).len(),
            1,
            "recreation must occur after the drop"
        );
        Ok(())
    }
}
