use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Token {
    Word(String),
    QuotedWord(String),
    String(String),
    DollarBody(String),
    Symbol(char),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CteBinding {
    name: String,
    visible_from: usize,
    visible_until: usize,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AccessKind {
    Read,
    Write,
    Metadata,
}

impl AccessKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Metadata => "metadata",
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RelationAccess {
    pub relation: String,
    pub kind: AccessKind,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(deny_unknown_fields)]
pub struct ForeignKey {
    pub child: String,
    pub child_columns: Vec<String>,
    pub parent: String,
    pub parent_columns: Vec<String>,
    pub on_delete: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ViewDefinition {
    pub name: String,
    pub direct_accesses: BTreeSet<RelationAccess>,
    pub called_functions: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RoutineDefinition {
    pub name: String,
    pub language: String,
    pub body_sha256: String,
    pub direct_accesses: BTreeSet<RelationAccess>,
    pub called_functions: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TriggerBinding {
    pub name: String,
    pub on_relation: String,
    pub function: String,
    pub timing: String,
    pub events: BTreeSet<String>,
    pub for_each: String,
    pub when_sha256: Option<String>,
    pub argument_sha256: Vec<String>,
    pub definition_sha256: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct DroppedTrigger {
    pub name: String,
    pub on_relation: String,
}

#[derive(Debug, Default)]
pub struct SchemaSnapshot {
    pub tables: BTreeSet<String>,
    pub views: BTreeSet<String>,
    pub view_definitions: BTreeSet<ViewDefinition>,
    pub routines: BTreeSet<RoutineDefinition>,
    pub triggers: BTreeSet<TriggerBinding>,
    pub foreign_keys: BTreeSet<ForeignKey>,
    pub dropped_views: BTreeSet<String>,
    pub recreated_dropped_views: BTreeSet<String>,
    pub dropped_triggers: BTreeSet<DroppedTrigger>,
    pub recreated_dropped_triggers: BTreeSet<DroppedTrigger>,
}

pub fn fingerprint(sql: &str) -> String {
    // Exception keys bind the exact SQL source. The only normalization makes
    // hashes portable between Windows and Unix checkouts; all other bytes,
    // including whitespace inside E-strings and executable bodies, are exact.
    let portable = sql.replace("\r\n", "\n");
    digest_hex(Sha256::digest(portable.as_bytes()))
}

fn token_fingerprint(tokens: &[Token]) -> String {
    let mut hasher = Sha256::new();
    for token in tokens {
        let (tag, value) = match token {
            Token::Word(value) => (b'w', value.as_bytes()),
            Token::QuotedWord(value) => (b'q', value.as_bytes()),
            Token::String(value) => (b's', value.as_bytes()),
            Token::DollarBody(value) => (b'd', value.as_bytes()),
            Token::Symbol(symbol) => {
                hasher.update(*b"y");
                hasher.update((*symbol as u32).to_be_bytes());
                continue;
            }
        };
        hasher.update([tag]);
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    digest_hex(hasher.finalize())
}

fn digest_hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn relation_accesses(sql: &str) -> Result<BTreeSet<RelationAccess>> {
    let tokens = lex(sql)?;
    let mut accesses = BTreeSet::new();
    let mut statements = 0;
    for statement in split_statements(&tokens) {
        if statement.is_empty() {
            continue;
        }
        statements += 1;
        validate_runtime_statement(statement)?;
        accesses.extend(relation_accesses_from_tokens(statement)?);
    }
    if statements == 0 {
        bail!("runtime SQL contains no statement");
    }
    Ok(accesses)
}

pub fn routine_calls(sql: &str, known: &BTreeSet<String>) -> Result<BTreeSet<String>> {
    let tokens = lex(sql)?;
    let known = known
        .iter()
        .map(|name| normalize_relation(name))
        .collect::<BTreeSet<_>>();
    Ok(function_call_candidates(&tokens)
        .intersection(&known)
        .cloned()
        .collect())
}

fn function_call_candidates(tokens: &[Token]) -> BTreeSet<String> {
    let mut calls = BTreeSet::new();
    for index in 0..tokens.len() {
        if matches!(tokens.get(index.wrapping_sub(1)), Some(Token::Symbol('.')))
            || matches!(
                word_at(tokens, index.wrapping_sub(1)),
                Some(
                    "as" | "function"
                        | "foreign"
                        | "into"
                        | "only"
                        | "procedure"
                        | "references"
                        | "table"
                        | "trigger"
                        | "update"
                        | "with"
                )
            )
        {
            continue;
        }
        let Some((candidate, next)) = relation_at(tokens, index) else {
            continue;
        };
        if matches!(tokens.get(next), Some(Token::Symbol('('))) {
            calls.insert(candidate);
        }
    }
    calls
}

fn relation_accesses_from_tokens(tokens: &[Token]) -> Result<BTreeSet<RelationAccess>> {
    let ctes = collect_cte_bindings(tokens)?;
    let mut accesses = BTreeSet::new();

    for index in 0..tokens.len() {
        let Some(word) = word_at(tokens, index) else {
            continue;
        };
        let (kind, relation_index) = match word {
            "insert" if word_at(tokens, index + 1) == Some("into") => {
                (AccessKind::Write, index + 2)
            }
            "merge" if word_at(tokens, index + 1) == Some("into") => (AccessKind::Write, index + 2),
            "update" if index == 0 || !matches!(word_at(tokens, index - 1), Some("do" | "for")) => {
                (AccessKind::Write, index + 1)
            }
            "delete" if word_at(tokens, index + 1) == Some("from") => {
                (AccessKind::Write, index + 2)
            }
            "truncate" => {
                let offset = usize::from(word_at(tokens, index + 1) == Some("table"));
                (AccessKind::Write, index + 1 + offset)
            }
            "lock" => {
                let offset = usize::from(word_at(tokens, index + 1) == Some("table"));
                (AccessKind::Write, index + 1 + offset)
            }
            "using"
                if is_delete_or_merge_using(tokens, index)
                    && !matches!(tokens.get(index + 1), Some(Token::Symbol('('))) =>
            {
                (AccessKind::Read, index + 1)
            }
            "table" if is_table_shorthand(tokens, index) => (AccessKind::Read, index + 1),
            "from" | "join" => {
                if word == "from" && index > 0 && word_at(tokens, index - 1) == Some("delete") {
                    continue;
                }
                if word == "from" && index > 0 && word_at(tokens, index - 1) == Some("distinct") {
                    continue;
                }
                if word == "from"
                    && enclosing_function(tokens, index).is_some_and(|function| {
                        matches!(function, "extract" | "substring" | "trim" | "overlay")
                    })
                {
                    continue;
                }
                (AccessKind::Read, index + 1)
            }
            _ => continue,
        };

        let mut relation_indexes = vec![relation_index];
        if matches!(word, "from" | "using" | "lock" | "truncate") {
            relation_indexes.extend(comma_relation_targets(
                tokens,
                relation_index,
                word == "using",
            ));
        }
        for relation_index in relation_indexes {
            let relation_index = skip_relation_modifiers(tokens, relation_index);
            let Some((relation, next)) = relation_at(tokens, relation_index) else {
                if matches!(tokens.get(relation_index), Some(Token::Symbol('('))) {
                    continue;
                }
                bail!("cannot resolve literal relation target after {word}");
            };
            if relation.starts_with('$')
                || (kind == AccessKind::Read
                    && matches!(tokens.get(next), Some(Token::Symbol('('))))
                || is_cte_reference(tokens, relation_index, &relation, &ctes)
                || is_system_relation(&relation)
                || is_relation_keyword(&relation)
            {
                continue;
            }
            accesses.insert(RelationAccess { relation, kind });
        }
    }

    for index in 0..tokens.len().saturating_sub(3) {
        let Token::String(value) = &tokens[index] else {
            continue;
        };
        if !matches!(tokens.get(index + 1), Some(Token::Symbol(':')))
            || !matches!(tokens.get(index + 2), Some(Token::Symbol(':')))
        {
            continue;
        }
        let Some((cast, _)) = relation_at(tokens, index + 3) else {
            continue;
        };
        if !matches!(cast.as_str(), "regclass" | "pg_catalog.regclass") {
            continue;
        }
        let relation = normalize_relation(value);
        if !is_system_relation(&relation) && !relation.starts_with("idx_") {
            accesses.insert(RelationAccess {
                relation,
                kind: AccessKind::Metadata,
            });
        }
    }

    Ok(accesses)
}

fn split_statements(tokens: &[Token]) -> Vec<&[Token]> {
    let mut statements = Vec::new();
    let mut start = 0;
    for end in 0..=tokens.len() {
        if end == tokens.len() || matches!(tokens[end], Token::Symbol(';')) {
            statements.push(&tokens[start..end]);
            start = end + 1;
        }
    }
    statements
}

fn validate_runtime_statement(tokens: &[Token]) -> Result<()> {
    let (command, command_index) = statement_command(tokens)?;
    if !matches!(
        command,
        "select"
            | "insert"
            | "update"
            | "delete"
            | "merge"
            | "truncate"
            | "lock"
            | "table"
            | "values"
    ) {
        bail!("unsupported runtime SQL statement {command}");
    }
    if command == "select" && has_top_level_select_into(tokens, command_index) {
        bail!("runtime SELECT INTO creates an unanalyzable relation");
    }

    let accesses = relation_accesses_from_tokens(tokens)?;
    let required = match command {
        "insert" | "update" | "delete" | "merge" | "truncate" | "lock" => Some(AccessKind::Write),
        "table" => Some(AccessKind::Read),
        _ => None,
    };
    if required.is_some_and(|kind| !accesses.iter().any(|access| access.kind == kind)) {
        bail!("cannot resolve relation for runtime SQL statement {command}");
    }
    Ok(())
}

fn statement_command(tokens: &[Token]) -> Result<(&str, usize)> {
    let first = word_at(tokens, 0)
        .ok_or_else(|| anyhow::anyhow!("runtime SQL statement has no leading command"))?;
    if first != "with" {
        return Ok((first, 0));
    }
    let Some((_, command_index)) = parse_cte_clause(tokens, 0)? else {
        bail!("malformed WITH clause in runtime SQL");
    };
    let command = word_at(tokens, command_index)
        .ok_or_else(|| anyhow::anyhow!("WITH clause has no main statement"))?;
    Ok((command, command_index))
}

fn has_top_level_select_into(tokens: &[Token], command_index: usize) -> bool {
    let mut depth = 0_u32;
    for token in &tokens[command_index + 1..] {
        match token {
            Token::Symbol('(') => depth += 1,
            Token::Symbol(')') => depth = depth.saturating_sub(1),
            Token::Word(word) if depth == 0 && word == "into" => return true,
            Token::Word(word) if depth == 0 && word == "from" => return false,
            _ => {}
        }
    }
    false
}

pub fn schema_snapshot(sql: &str) -> Result<SchemaSnapshot> {
    let tokens = lex(sql)?;
    let mut snapshot = SchemaSnapshot::default();
    let mut start = 0;
    for end in 0..=tokens.len() {
        if end == tokens.len() || matches!(tokens[end], Token::Symbol(';')) {
            parse_statement(&tokens[start..end], &mut snapshot)?;
            start = end + 1;
        }
    }
    Ok(snapshot)
}

pub fn migration_snapshot(sql: &str) -> Result<SchemaSnapshot> {
    let tokens = lex(sql)?;
    let mut snapshot = SchemaSnapshot::default();
    let mut statements = 0;
    for statement in split_statements(&tokens) {
        if statement.is_empty() {
            continue;
        }
        statements += 1;
        audit_migration_statement(statement, &mut snapshot)?;
    }
    if statements == 0 {
        bail!("migration contains no statement");
    }
    Ok(snapshot)
}

pub fn canonical_schema_snapshot(sql: &str) -> Result<SchemaSnapshot> {
    let tokens = lex(sql)?;
    let mut snapshot = SchemaSnapshot::default();
    let mut data_accesses = BTreeSet::new();
    let mut statements = 0;
    for statement in split_statements(&tokens) {
        if statement.is_empty() {
            continue;
        }
        statements += 1;
        let command = word_at(statement, 0)
            .ok_or_else(|| anyhow::anyhow!("canonical schema statement has no leading command"))?;
        match command {
            "select" | "insert" | "update" | "delete" | "merge" | "truncate" | "lock" | "table"
            | "values" | "with" => {
                validate_runtime_statement(statement)?;
                data_accesses.extend(relation_accesses_from_tokens(statement)?);
            }
            "create" => audit_create_statement(statement, &mut snapshot)?,
            "alter" => audit_alter_table_statement(statement, &mut snapshot)?,
            _ => bail!("unsupported canonical schema statement {command}"),
        }
    }
    if statements == 0 {
        bail!("canonical schema contains no statement");
    }
    for access in data_accesses {
        if !snapshot.tables.contains(&access.relation) && !snapshot.views.contains(&access.relation)
        {
            bail!(
                "canonical schema data statement references undeclared relation {} ({})",
                access.relation,
                access.kind.as_str()
            );
        }
    }
    Ok(snapshot)
}

fn audit_migration_statement(tokens: &[Token], snapshot: &mut SchemaSnapshot) -> Result<()> {
    let command = word_at(tokens, 0)
        .ok_or_else(|| anyhow::anyhow!("migration statement has no leading command"))?;
    match command {
        "begin" | "commit" => {
            if tokens.len() != 1 {
                bail!("unsupported migration transaction statement");
            }
        }
        "select" | "insert" | "update" | "delete" | "merge" | "truncate" | "lock" | "table"
        | "values" | "with" => validate_runtime_statement(tokens)?,
        "create" => audit_create_statement(tokens, snapshot)?,
        "alter" => audit_alter_table_statement(tokens, snapshot)?,
        "drop" => parse_drop_statement(tokens, snapshot)?,
        "do" => audit_do_statement(tokens, snapshot)?,
        _ => bail!("unsupported migration statement {command}"),
    }
    Ok(())
}

fn audit_create_statement(tokens: &[Token], snapshot: &mut SchemaSnapshot) -> Result<()> {
    let mut modifier_index = 1;
    if word_at(tokens, modifier_index) == Some("or")
        && word_at(tokens, modifier_index + 1) == Some("replace")
    {
        modifier_index += 2;
    }
    if matches!(
        word_at(tokens, modifier_index),
        Some("temp" | "temporary" | "unlogged")
    ) {
        bail!("temporary or unlogged owned schema objects are unsupported");
    }
    let (kind, kind_index) = create_kind(tokens)?;
    match kind {
        "table" | "function" | "procedure" | "trigger" => {
            parse_statement(tokens, snapshot)?;
        }
        "view" => {
            let as_index = tokens
                .iter()
                .position(|token| is_word(token, "as"))
                .ok_or_else(|| anyhow::anyhow!("CREATE VIEW is missing AS"))?;
            validate_runtime_statement(&tokens[as_index + 1..])?;
            parse_statement(tokens, snapshot)?;
        }
        "index" => validate_create_index(tokens, kind_index)?,
        "extension" => validate_named_create(tokens, kind_index, "extension")?,
        "type" => {
            validate_named_create(tokens, kind_index, "type")?;
            if !tokens.iter().any(|token| is_word(token, "as"))
                || !tokens.iter().any(|token| is_word(token, "enum"))
            {
                bail!("only CREATE TYPE ... AS ENUM is supported in migrations");
            }
        }
        _ => bail!("unsupported migration CREATE {kind}"),
    }
    Ok(())
}

fn create_kind(tokens: &[Token]) -> Result<(&str, usize)> {
    let mut index = 1;
    if word_at(tokens, index) == Some("or") && word_at(tokens, index + 1) == Some("replace") {
        index += 2;
    }
    if word_at(tokens, index) == Some("unique") {
        index += 1;
    }
    if matches!(
        word_at(tokens, index),
        Some("unlogged" | "temp" | "temporary")
    ) {
        index += 1;
    }
    word_at(tokens, index)
        .map(|kind| (kind, index))
        .ok_or_else(|| anyhow::anyhow!("CREATE is missing an object kind"))
}

fn validate_named_create(tokens: &[Token], kind_index: usize, kind: &str) -> Result<()> {
    let mut index = kind_index + 1;
    if word_at(tokens, index) == Some("if")
        && word_at(tokens, index + 1) == Some("not")
        && word_at(tokens, index + 2) == Some("exists")
    {
        index += 3;
    }
    if relation_at(tokens, index).is_none() {
        bail!("CREATE {kind} is missing a literal name");
    }
    Ok(())
}

fn validate_create_index(tokens: &[Token], kind_index: usize) -> Result<()> {
    let mut index = kind_index + 1;
    if word_at(tokens, index) == Some("concurrently") {
        index += 1;
    }
    if word_at(tokens, index) == Some("if")
        && word_at(tokens, index + 1) == Some("not")
        && word_at(tokens, index + 2) == Some("exists")
    {
        index += 3;
    }
    let Some((_, after_name)) = relation_at(tokens, index) else {
        bail!("CREATE INDEX is missing a literal index name");
    };
    let on = tokens[after_name..]
        .iter()
        .position(|token| is_word(token, "on"))
        .map(|offset| after_name + offset)
        .ok_or_else(|| anyhow::anyhow!("CREATE INDEX is missing ON"))?;
    let relation_index = skip_relation_modifiers(tokens, on + 1);
    if relation_at(tokens, relation_index).is_none() {
        bail!("CREATE INDEX is missing a literal relation name");
    }
    Ok(())
}

fn audit_alter_table_statement(tokens: &[Token], snapshot: &mut SchemaSnapshot) -> Result<()> {
    if word_at(tokens, 1) != Some("table") {
        bail!("only ALTER TABLE is supported in migrations");
    }
    let mut index = 2;
    if word_at(tokens, index) == Some("if") && word_at(tokens, index + 1) == Some("exists") {
        index += 2;
    }
    index = skip_relation_modifiers(tokens, index);
    let Some((_, actions)) = relation_at(tokens, index) else {
        bail!("ALTER TABLE is missing a literal relation name");
    };
    for action in split_clauses(&tokens[actions..]) {
        if !matches!(word_at(action, 0), Some("add" | "alter" | "drop")) {
            bail!("unsupported ALTER TABLE action");
        }
    }
    parse_alter_table(tokens, snapshot)
}

fn parse_drop_statement(tokens: &[Token], snapshot: &mut SchemaSnapshot) -> Result<()> {
    let kind =
        word_at(tokens, 1).ok_or_else(|| anyhow::anyhow!("DROP is missing an object kind"))?;
    if !matches!(kind, "index" | "trigger" | "view") {
        bail!("unsupported migration DROP {kind}");
    }
    let mut index = 2;
    if word_at(tokens, index) == Some("concurrently") {
        index += 1;
    }
    if word_at(tokens, index) == Some("if") && word_at(tokens, index + 1) == Some("exists") {
        index += 2;
    }
    let Some((name, after_name)) = relation_at(tokens, index) else {
        bail!("DROP {kind} is missing a literal name");
    };
    match kind {
        "view" => {
            validate_drop_tail(tokens, after_name, "VIEW")?;
            snapshot.recreated_dropped_views.remove(&name);
            snapshot.dropped_views.insert(name);
        }
        "trigger" => {
            if word_at(tokens, after_name) != Some("on") {
                bail!("DROP TRIGGER is missing a literal ON relation");
            }
            let Some((on_relation, after_relation)) = relation_at(tokens, after_name + 1) else {
                bail!("DROP TRIGGER is missing a literal ON relation");
            };
            validate_drop_tail(tokens, after_relation, "TRIGGER")?;
            let dropped = DroppedTrigger { name, on_relation };
            snapshot.recreated_dropped_triggers.remove(&dropped);
            snapshot.dropped_triggers.insert(dropped);
        }
        "index" => validate_drop_tail(tokens, after_name, "INDEX")?,
        _ => unreachable!("DROP kind was validated above"),
    }
    Ok(())
}

fn validate_drop_tail(tokens: &[Token], index: usize, kind: &str) -> Result<()> {
    if index == tokens.len()
        || (index + 1 == tokens.len()
            && matches!(word_at(tokens, index), Some("cascade" | "restrict")))
    {
        return Ok(());
    }
    bail!("DROP {kind} has unsupported trailing syntax")
}

fn audit_do_statement(tokens: &[Token], snapshot: &mut SchemaSnapshot) -> Result<()> {
    let bodies = tokens
        .iter()
        .filter_map(|token| match token {
            Token::DollarBody(body) => Some(body.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if bodies.len() != 1 {
        bail!("DO migration statement must contain exactly one dollar-quoted body");
    }
    if let Some(language) = tokens.iter().position(|token| is_word(token, "language")) {
        if word_at(tokens, language + 1) != Some("plpgsql") {
            bail!("DO migration statement uses an unsupported language");
        }
    }
    audit_do_body(bodies[0], snapshot)
}

fn audit_do_body(body: &str, snapshot: &mut SchemaSnapshot) -> Result<()> {
    let tokens = lex(body)?;
    for forbidden in [
        "call", "cluster", "copy", "execute", "grant", "lock", "prepare", "refresh", "reindex",
        "revoke", "truncate", "vacuum",
    ] {
        if tokens.iter().any(|token| is_word(token, forbidden)) {
            bail!("DO migration body uses unsupported {forbidden}");
        }
    }
    for executable in [
        "call", "insert", "merge", "perform", "select", "truncate", "update",
    ] {
        if tokens.iter().any(|token| is_word(token, executable)) {
            bail!("DO migration body contains executable {executable}");
        }
    }
    if (0..tokens.len()).any(|index| {
        word_at(&tokens, index) == Some("delete") && word_at(&tokens, index + 1) == Some("from")
    }) {
        bail!("DO migration body contains executable delete");
    }
    let unsafe_calls = function_call_candidates(&tokens)
        .into_iter()
        .filter(|function| function != "exists")
        .collect::<Vec<_>>();
    if !unsafe_calls.is_empty() {
        bail!(
            "DO migration body contains executable function call(s): {}",
            unsafe_calls.join(", ")
        );
    }

    for segment in split_statements(&tokens) {
        for index in embedded_ddl_commands(segment) {
            match word_at(segment, index).expect("command index") {
                "create" => audit_create_statement(&segment[index..], snapshot)?,
                "alter" => audit_alter_table_statement(&segment[index..], snapshot)?,
                "drop" => parse_drop_statement(&segment[index..], snapshot)?,
                _ => unreachable!(),
            }
        }
    }
    Ok(())
}

fn embedded_ddl_commands(tokens: &[Token]) -> Vec<usize> {
    let mut commands = Vec::new();
    for index in 0..tokens.len() {
        if !matches!(word_at(tokens, index), Some("create" | "alter" | "drop")) {
            continue;
        }
        let previous = tokens[..index].iter().rev().find_map(|token| match token {
            Token::Word(word) => Some(word.as_str()),
            _ => None,
        });
        if previous.is_none() || matches!(previous, Some("begin" | "else" | "loop" | "then")) {
            commands.push(index);
        }
    }
    commands
}

fn parse_statement(tokens: &[Token], snapshot: &mut SchemaSnapshot) -> Result<()> {
    if word_at(tokens, 0) == Some("alter") {
        return parse_alter_table(tokens, snapshot);
    }
    if word_at(tokens, 0) == Some("drop") {
        return parse_drop_statement(tokens, snapshot);
    }
    let Some(create) = tokens.iter().position(|token| is_word(token, "create")) else {
        return Ok(());
    };
    let mut index = create + 1;
    if word_at(tokens, index) == Some("or") && word_at(tokens, index + 1) == Some("replace") {
        index += 2;
    }
    if matches!(
        word_at(tokens, index),
        Some("unlogged" | "temp" | "temporary")
    ) {
        index += 1;
    }
    let Some(kind) = word_at(tokens, index) else {
        return Ok(());
    };
    index += 1;
    match kind {
        "function" | "procedure" => return parse_routine(tokens, index, snapshot),
        "trigger" => return parse_trigger(tokens, index, snapshot),
        "table" | "view" => {}
        _ => return Ok(()),
    }
    if word_at(tokens, index) == Some("if")
        && word_at(tokens, index + 1) == Some("not")
        && word_at(tokens, index + 2) == Some("exists")
    {
        index += 3;
    }
    let Some((relation, next)) = relation_at(tokens, index) else {
        bail!("CREATE {kind} is missing a literal relation name");
    };
    if kind == "view" {
        let Some(as_index) = tokens[next..]
            .iter()
            .position(|token| is_word(token, "as"))
            .map(|offset| next + offset)
        else {
            bail!("CREATE VIEW {relation} is missing AS");
        };
        snapshot.view_definitions.insert(ViewDefinition {
            name: relation.clone(),
            direct_accesses: relation_accesses_from_tokens(&tokens[as_index + 1..])?,
            called_functions: function_call_candidates(&tokens[as_index + 1..]),
        });
        if snapshot.dropped_views.contains(&relation) {
            snapshot.recreated_dropped_views.insert(relation.clone());
        }
        snapshot.views.insert(relation);
        return Ok(());
    }
    snapshot.tables.insert(relation.clone());

    let Some(open) = tokens[next..]
        .iter()
        .position(|token| matches!(token, Token::Symbol('(')))
        .map(|offset| next + offset)
    else {
        bail!("CREATE TABLE {relation} is missing its column list");
    };
    let close = matching_close(tokens, open)?;
    for clause in split_clauses(&tokens[open + 1..close]) {
        if let Some(foreign_key) = parse_foreign_key(&relation, clause)? {
            snapshot.foreign_keys.insert(foreign_key);
        }
    }
    Ok(())
}

fn parse_routine(tokens: &[Token], name_index: usize, snapshot: &mut SchemaSnapshot) -> Result<()> {
    let Some((name, _)) = relation_at(tokens, name_index) else {
        bail!("CREATE routine is missing a literal name");
    };
    let language_index = tokens
        .iter()
        .position(|token| is_word(token, "language"))
        .ok_or_else(|| anyhow::anyhow!("CREATE routine {name} is missing LANGUAGE"))?;
    let language = word_at(tokens, language_index + 1)
        .ok_or_else(|| anyhow::anyhow!("CREATE routine {name} has no literal language"))?
        .to_string();
    let body = tokens
        .iter()
        .find_map(|token| match token {
            Token::DollarBody(body) => Some(body),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("CREATE routine {name} has no dollar-quoted body"))?;
    let (direct_accesses, called_functions) = analyze_routine_body(&name, &language, body)?;
    snapshot.routines.insert(RoutineDefinition {
        name,
        language,
        body_sha256: fingerprint(body),
        direct_accesses,
        called_functions,
    });
    Ok(())
}

fn analyze_routine_body(
    name: &str,
    language: &str,
    body: &str,
) -> Result<(BTreeSet<RelationAccess>, BTreeSet<String>)> {
    let tokens = lex(body)?;
    let called_functions = function_call_candidates(&tokens);
    let direct_accesses = match language {
        "sql" => {
            let mut accesses = BTreeSet::new();
            let mut statements = 0;
            for statement in split_statements(&tokens) {
                if statement.is_empty() {
                    continue;
                }
                statements += 1;
                if word_at(statement, 0) == Some("call") {
                    validate_static_call(statement)?;
                } else {
                    validate_runtime_statement(statement)?;
                    accesses.extend(relation_accesses_from_tokens(statement)?);
                }
            }
            if statements == 0 {
                bail!("SQL routine {name} has no executable statement");
            }
            accesses
        }
        "plpgsql" => {
            validate_plpgsql_body(name, &tokens)?;
            relation_accesses_from_tokens(&tokens)?
        }
        _ => bail!("routine {name} uses unsupported executable language {language}"),
    };
    Ok((direct_accesses, called_functions))
}

fn validate_static_call(tokens: &[Token]) -> Result<()> {
    let Some((_, next)) = relation_at(tokens, 1) else {
        bail!("CALL is missing a literal routine name");
    };
    if !matches!(tokens.get(next), Some(Token::Symbol('('))) {
        bail!("CALL is missing a literal argument list");
    }
    Ok(())
}

fn validate_plpgsql_body(name: &str, tokens: &[Token]) -> Result<()> {
    for forbidden in [
        "alter", "cluster", "copy", "create", "declare", "drop", "execute", "fetch", "foreach",
        "grant", "lock", "move", "open", "prepare", "refresh", "reindex", "revoke", "truncate",
        "vacuum", "while",
    ] {
        if tokens.iter().any(|token| is_word(token, forbidden)) {
            bail!("PL/pgSQL routine {name} uses unsupported {forbidden}");
        }
    }

    let mut executable_segments = 0;
    for statement in split_statements(tokens) {
        if statement.is_empty() {
            continue;
        }
        executable_segments += 1;
        validate_plpgsql_segment(name, statement)?;
    }
    if executable_segments == 0 {
        bail!("PL/pgSQL routine {name} has no analyzable statement");
    }
    Ok(())
}

fn validate_plpgsql_segment(name: &str, tokens: &[Token]) -> Result<()> {
    let mut starts = vec![0];
    for (index, token) in tokens.iter().enumerate() {
        if matches!(token, Token::Word(word) if matches!(word.as_str(), "begin" | "then" | "else"))
        {
            starts.push(index + 1);
        }
    }
    for start in starts {
        let Some((offset, command)) =
            tokens[start..]
                .iter()
                .enumerate()
                .find_map(|(offset, token)| match token {
                    Token::Word(word) => Some((offset, word.as_str())),
                    Token::Symbol(';') => None,
                    _ => None,
                })
        else {
            continue;
        };
        let command_index = start + offset;
        if matches!(
            command,
            "begin"
                | "call"
                | "delete"
                | "else"
                | "elsif"
                | "end"
                | "if"
                | "insert"
                | "merge"
                | "null"
                | "perform"
                | "raise"
                | "return"
                | "select"
                | "update"
                | "values"
                | "with"
        ) || contains_assignment(&tokens[command_index..])
        {
            if command == "call" {
                validate_static_call(&tokens[command_index..])?;
            }
            continue;
        }
        bail!("PL/pgSQL routine {name} has unanalyzable statement starting with {command}");
    }
    Ok(())
}

fn contains_assignment(tokens: &[Token]) -> bool {
    if !matches!(tokens.first(), Some(Token::Word(_))) {
        return false;
    }
    let mut index = 1;
    while matches!(tokens.get(index), Some(Token::Symbol('.')))
        && matches!(tokens.get(index + 1), Some(Token::Word(_)))
    {
        index += 2;
    }
    matches!(tokens.get(index), Some(Token::Symbol('=')))
        || matches!(
            tokens.get(index..index + 2),
            Some([Token::Symbol(':'), Token::Symbol('=')])
        )
}

fn parse_trigger(tokens: &[Token], name_index: usize, snapshot: &mut SchemaSnapshot) -> Result<()> {
    let Some((name, after_name)) = relation_at(tokens, name_index) else {
        bail!("CREATE TRIGGER is missing a literal name");
    };
    let (timing, events_start) = match word_at(tokens, after_name) {
        Some("before") => ("before".to_string(), after_name + 1),
        Some("after") => ("after".to_string(), after_name + 1),
        Some("instead") if word_at(tokens, after_name + 1) == Some("of") => {
            ("instead_of".to_string(), after_name + 2)
        }
        _ => bail!("CREATE TRIGGER {name} has unsupported or missing timing"),
    };
    let on_index = tokens[after_name..]
        .iter()
        .position(|token| is_word(token, "on"))
        .map(|offset| after_name + offset)
        .ok_or_else(|| anyhow::anyhow!("CREATE TRIGGER {name} is missing ON"))?;
    let events = parse_trigger_events(&name, &tokens[events_start..on_index])?;
    let Some((on_relation, after_relation)) = relation_at(tokens, on_index + 1) else {
        bail!("CREATE TRIGGER {name} is missing its ON relation");
    };
    let execute_index = tokens[on_index + 1..]
        .iter()
        .position(|token| is_word(token, "execute"))
        .map(|offset| on_index + 1 + offset)
        .ok_or_else(|| anyhow::anyhow!("CREATE TRIGGER {name} is missing EXECUTE"))?;
    let mut cursor = after_relation;
    let mut for_each = "statement".to_string();
    if word_at(tokens, cursor) == Some("for") {
        cursor += 1;
        if word_at(tokens, cursor) == Some("each") {
            cursor += 1;
        }
        for_each = match word_at(tokens, cursor) {
            Some("row") => "row".to_string(),
            Some("statement") => "statement".to_string(),
            _ => bail!("CREATE TRIGGER {name} has invalid FOR EACH target"),
        };
        cursor += 1;
    }
    let mut when_sha256 = None;
    if word_at(tokens, cursor) == Some("when") {
        cursor += 1;
        if !matches!(tokens.get(cursor), Some(Token::Symbol('('))) {
            bail!("CREATE TRIGGER {name} WHEN is missing a condition");
        }
        let close = matching_close(tokens, cursor)?;
        if close >= execute_index {
            bail!("CREATE TRIGGER {name} has malformed WHEN condition");
        }
        when_sha256 = Some(token_fingerprint(&tokens[cursor + 1..close]));
        cursor = close + 1;
    }
    if cursor != execute_index {
        bail!("CREATE TRIGGER {name} contains unsupported trigger clauses");
    }
    if !matches!(
        word_at(tokens, execute_index + 1),
        Some("function" | "procedure")
    ) {
        bail!("CREATE TRIGGER {name} EXECUTE is missing FUNCTION");
    }
    let Some((function, arguments_open)) = relation_at(tokens, execute_index + 2) else {
        bail!("CREATE TRIGGER {name} is missing its function");
    };
    if !matches!(tokens.get(arguments_open), Some(Token::Symbol('('))) {
        bail!("CREATE TRIGGER {name} function is missing its argument list");
    }
    let arguments_close = matching_close(tokens, arguments_open)?;
    if arguments_close + 1 != tokens.len() {
        bail!("CREATE TRIGGER {name} has trailing unsupported syntax");
    }
    let argument_sha256 = split_clauses(&tokens[arguments_open + 1..arguments_close])
        .into_iter()
        .filter(|argument| !argument.is_empty())
        .map(token_fingerprint)
        .collect::<Vec<_>>();
    let definition_source = format!(
        "timing={timing}\nevents={}\non={on_relation}\nfor_each={for_each}\nwhen={}\nfunction={function}\narguments={}",
        events.iter().cloned().collect::<Vec<_>>().join(","),
        when_sha256.as_deref().unwrap_or("none"),
        argument_sha256.join(",")
    );
    let dropped = DroppedTrigger {
        name: name.clone(),
        on_relation: on_relation.clone(),
    };
    if snapshot.dropped_triggers.contains(&dropped) {
        snapshot.recreated_dropped_triggers.insert(dropped);
    }
    snapshot.triggers.insert(TriggerBinding {
        name,
        on_relation,
        function,
        timing,
        events,
        for_each,
        when_sha256,
        argument_sha256,
        definition_sha256: fingerprint(&definition_source),
    });
    Ok(())
}

fn parse_trigger_events(name: &str, tokens: &[Token]) -> Result<BTreeSet<String>> {
    let mut events = BTreeSet::new();
    let mut start = 0;
    for end in 0..=tokens.len() {
        if end != tokens.len() && word_at(tokens, end) != Some("or") {
            continue;
        }
        let clause = &tokens[start..end];
        let Some(event) = word_at(clause, 0) else {
            bail!("CREATE TRIGGER {name} has an empty event");
        };
        let normalized = match event {
            "insert" | "delete" | "truncate" if clause.len() == 1 => event.to_string(),
            "update" if clause.len() == 1 => "update".to_string(),
            "update" if word_at(clause, 1) == Some("of") => {
                let mut columns = BTreeSet::new();
                let mut expect_column = true;
                for token in &clause[2..] {
                    match (expect_column, token) {
                        (true, token @ (Token::Word(_) | Token::QuotedWord(_))) => {
                            let column = identifier_component(token).expect("identifier token");
                            if !columns.insert(column) {
                                bail!("CREATE TRIGGER {name} repeats an UPDATE OF column");
                            }
                            expect_column = false;
                        }
                        (false, Token::Symbol(',')) => expect_column = true,
                        _ => bail!("CREATE TRIGGER {name} has invalid UPDATE OF columns"),
                    }
                }
                if columns.is_empty() || expect_column {
                    bail!("CREATE TRIGGER {name} has invalid UPDATE OF columns");
                }
                format!(
                    "update_of:{}",
                    columns.into_iter().collect::<Vec<_>>().join(",")
                )
            }
            _ => bail!("CREATE TRIGGER {name} has unsupported event syntax"),
        };
        if !events.insert(normalized) {
            bail!("CREATE TRIGGER {name} repeats an event");
        }
        start = end + 1;
    }
    if events.is_empty() {
        bail!("CREATE TRIGGER {name} has no event");
    }
    Ok(events)
}

fn parse_alter_table(tokens: &[Token], snapshot: &mut SchemaSnapshot) -> Result<()> {
    if word_at(tokens, 1) != Some("table") {
        return Ok(());
    }
    let mut index = 2;
    if word_at(tokens, index) == Some("if") && word_at(tokens, index + 1) == Some("exists") {
        index += 2;
    }
    index = skip_relation_modifiers(tokens, index);
    let Some((child, next)) = relation_at(tokens, index) else {
        bail!("ALTER TABLE is missing a literal relation name");
    };

    for action in split_clauses(&tokens[next..]) {
        if word_at(action, 0) != Some("add") {
            continue;
        }
        let definition = &action[1..];
        let inline_child = if !definition.iter().any(|token| is_word(token, "foreign"))
            && definition.iter().any(|token| is_word(token, "references"))
        {
            Some(alter_inline_child_column(&child, definition)?)
        } else {
            None
        };
        if let Some(foreign_key) = parse_foreign_key_with_child(&child, definition, inline_child)? {
            snapshot.foreign_keys.insert(foreign_key);
        }
    }
    Ok(())
}

fn parse_foreign_key(child: &str, clause: &[Token]) -> Result<Option<ForeignKey>> {
    parse_foreign_key_with_child(child, clause, None)
}

fn parse_foreign_key_with_child(
    child: &str,
    clause: &[Token],
    inline_child: Option<String>,
) -> Result<Option<ForeignKey>> {
    let Some(reference_index) = clause.iter().position(|token| is_word(token, "references")) else {
        return Ok(None);
    };
    let Some((parent, after_parent)) = relation_at(clause, reference_index + 1) else {
        bail!("REFERENCES in {child} is missing a parent relation");
    };
    let parent_columns = column_list(clause, after_parent)?;

    let child_columns = if let Some(inline_child) = inline_child {
        vec![inline_child]
    } else if let Some(foreign_index) = clause.iter().position(|token| is_word(token, "foreign")) {
        let key_index = foreign_index + 1;
        if word_at(clause, key_index) != Some("key") {
            bail!("FOREIGN in {child} is not followed by KEY");
        }
        column_list(clause, key_index + 1)?
    } else {
        clause
            .iter()
            .find_map(|token| match token {
                Token::Word(word)
                    if !matches!(word.as_str(), "constraint" | "primary" | "unique") =>
                {
                    Some(vec![word.to_ascii_lowercase()])
                }
                Token::QuotedWord(_) => identifier_component(token).map(|column| vec![column]),
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("inline REFERENCES in {child} has no child column"))?
    };

    let mut on_delete = "no_action".to_string();
    for index in reference_index..clause.len().saturating_sub(2) {
        if word_at(clause, index) == Some("on") && word_at(clause, index + 1) == Some("delete") {
            on_delete = match word_at(clause, index + 2) {
                Some("cascade" | "restrict") => word_at(clause, index + 2)
                    .expect("matched literal")
                    .to_string(),
                Some("no") if word_at(clause, index + 3) == Some("action") => {
                    "no_action".to_string()
                }
                Some("set") if word_at(clause, index + 3) == Some("null") => "set_null".to_string(),
                Some("set") if word_at(clause, index + 3) == Some("default") => {
                    "set_default".to_string()
                }
                _ => bail!("REFERENCES in {child} has unsupported ON DELETE action"),
            };
            break;
        }
    }
    Ok(Some(ForeignKey {
        child: child.to_string(),
        child_columns,
        parent,
        parent_columns,
        on_delete,
    }))
}

fn alter_inline_child_column(child: &str, clause: &[Token]) -> Result<String> {
    let mut index = usize::from(word_at(clause, 0) == Some("column"));
    if word_at(clause, index) == Some("if")
        && word_at(clause, index + 1) == Some("not")
        && word_at(clause, index + 2) == Some("exists")
    {
        index += 3;
    }
    let Some(column) = clause.get(index).and_then(identifier_component) else {
        bail!("ALTER TABLE {child} ADD COLUMN REFERENCES has no literal child column");
    };
    Ok(column)
}

fn column_list(tokens: &[Token], from: usize) -> Result<Vec<String>> {
    let Some(open) = tokens[from..]
        .iter()
        .position(|token| matches!(token, Token::Symbol('(')))
        .map(|offset| from + offset)
    else {
        bail!("expected a parenthesized column list");
    };
    let close = matching_close(tokens, open)?;
    let columns = tokens[open + 1..close]
        .iter()
        .filter_map(identifier_component)
        .collect::<Vec<_>>();
    if columns.is_empty() {
        bail!("column list cannot be empty");
    }
    Ok(columns)
}

fn split_clauses(tokens: &[Token]) -> Vec<&[Token]> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0_i32;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Symbol('(') => depth += 1,
            Token::Symbol(')') => depth -= 1,
            Token::Symbol(',') if depth == 0 => {
                result.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    result.push(&tokens[start..]);
    result
}

fn matching_close(tokens: &[Token], open: usize) -> Result<usize> {
    let mut depth = 0_i32;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token {
            Token::Symbol('(') => depth += 1,
            Token::Symbol(')') => {
                depth -= 1;
                if depth == 0 {
                    return Ok(index);
                }
            }
            _ => {}
        }
    }
    bail!("unclosed parenthesis in SQL")
}

fn collect_cte_bindings(tokens: &[Token]) -> Result<Vec<CteBinding>> {
    let mut bindings = Vec::new();
    for with_index in 0..tokens.len() {
        if word_at(tokens, with_index) != Some("with") {
            continue;
        }
        if let Some((nested, _)) = parse_cte_clause(tokens, with_index)? {
            bindings.extend(nested);
        }
    }
    Ok(bindings)
}

fn parse_cte_clause(
    tokens: &[Token],
    with_index: usize,
) -> Result<Option<(Vec<CteBinding>, usize)>> {
    let mut index = with_index + 1;
    let recursive = word_at(tokens, index) == Some("recursive");
    if recursive {
        index += 1;
    }
    let scope_end = enclosing_scope_end(tokens, with_index)?;
    let mut bindings = Vec::new();

    loop {
        let Some(alias) = tokens.get(index).and_then(identifier_component) else {
            return Ok(None);
        };
        index += 1;
        if matches!(tokens.get(index), Some(Token::Symbol('('))) {
            index = matching_close(tokens, index)? + 1;
        }
        if word_at(tokens, index) != Some("as") {
            return Ok(None);
        }
        index += 1;
        if word_at(tokens, index) == Some("not") {
            index += 1;
            if word_at(tokens, index) != Some("materialized") {
                bail!("WITH NOT must be followed by MATERIALIZED");
            }
            index += 1;
        } else if word_at(tokens, index) == Some("materialized") {
            index += 1;
        }
        if !matches!(tokens.get(index), Some(Token::Symbol('('))) {
            bail!("CTE {alias} is missing its parenthesized query");
        }
        let body_open = index;
        let body_close = matching_close(tokens, body_open)?;
        bindings.push(CteBinding {
            name: alias,
            visible_from: if recursive {
                body_open + 1
            } else {
                body_close + 1
            },
            visible_until: scope_end,
        });
        index = body_close + 1;
        if !matches!(tokens.get(index), Some(Token::Symbol(','))) {
            break;
        }
        index += 1;
    }
    Ok(Some((bindings, index)))
}

fn enclosing_scope_end(tokens: &[Token], index: usize) -> Result<usize> {
    let mut opens = Vec::new();
    for (cursor, token) in tokens.iter().enumerate().take(index) {
        match token {
            Token::Symbol('(') => opens.push(cursor),
            Token::Symbol(')') => {
                opens.pop();
            }
            _ => {}
        }
    }
    opens
        .last()
        .copied()
        .map_or(Ok(tokens.len()), |open| matching_close(tokens, open))
}

fn is_cte_reference(
    tokens: &[Token],
    relation_index: usize,
    relation: &str,
    bindings: &[CteBinding],
) -> bool {
    if matches!(tokens.get(relation_index + 1), Some(Token::Symbol('.'))) {
        return false;
    }
    bindings.iter().any(|binding| {
        binding.name == relation
            && relation_index >= binding.visible_from
            && relation_index < binding.visible_until
    })
}

fn skip_relation_modifiers(tokens: &[Token], mut index: usize) -> usize {
    while matches!(word_at(tokens, index), Some("lateral" | "only")) {
        index += 1;
    }
    index
}

fn comma_relation_targets(tokens: &[Token], from: usize, stop_on_on: bool) -> Vec<usize> {
    let mut targets = Vec::new();
    let mut depth = 0_u32;
    let mut index = from;
    while index < tokens.len() {
        match tokens.get(index) {
            Some(Token::Symbol('(')) => depth += 1,
            Some(Token::Symbol(')')) if depth == 0 => break,
            Some(Token::Symbol(')')) => depth -= 1,
            Some(Token::Symbol(';')) if depth == 0 => break,
            Some(Token::Symbol(',')) if depth == 0 => targets.push(index + 1),
            Some(Token::Word(word))
                if depth == 0 && (ends_from_clause(word) || (stop_on_on && word == "on")) =>
            {
                break
            }
            _ => {}
        }
        index += 1;
    }
    targets
}

fn is_table_shorthand(tokens: &[Token], index: usize) -> bool {
    index == 0
        || matches!(tokens.get(index.wrapping_sub(1)), Some(Token::Symbol(';')))
        || matches!(
            word_at(tokens, index.wrapping_sub(1)),
            Some("union" | "intersect" | "except")
        )
}

fn is_delete_or_merge_using(tokens: &[Token], index: usize) -> bool {
    let start = tokens[..index]
        .iter()
        .rposition(|token| matches!(token, Token::Symbol(';')))
        .map_or(0, |semicolon| semicolon + 1);
    let mut depth = 0_u32;
    for token in &tokens[start..index] {
        match token {
            Token::Symbol('(') => depth += 1,
            Token::Symbol(')') => depth = depth.saturating_sub(1),
            Token::Word(word) if depth == 0 && matches!(word.as_str(), "delete" | "merge") => {
                return true
            }
            _ => {}
        }
    }
    false
}

fn ends_from_clause(word: &str) -> bool {
    matches!(
        word,
        "where"
            | "group"
            | "having"
            | "window"
            | "union"
            | "intersect"
            | "except"
            | "order"
            | "limit"
            | "offset"
            | "fetch"
            | "for"
            | "returning"
    )
}

fn relation_at(tokens: &[Token], index: usize) -> Option<(String, usize)> {
    let first = identifier_component(tokens.get(index)?)?;
    if matches!(tokens.get(index + 1), Some(Token::Symbol('.'))) {
        let second = identifier_component(tokens.get(index + 2)?)?;
        let relation = if first == "public" {
            second
        } else {
            format!("{first}.{second}")
        };
        return Some((relation, index + 3));
    }
    Some((first, index + 1))
}

fn identifier_component(token: &Token) -> Option<String> {
    match token {
        Token::Word(value) => Some(value.to_ascii_lowercase()),
        Token::QuotedWord(value) if quoted_is_unquoted_equivalent(value) => Some(value.clone()),
        Token::QuotedWord(value) => Some(format!("\"{}\"", value.replace('"', "\"\""))),
        _ => None,
    }
}

fn quoted_is_unquoted_equivalent(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && !is_reserved_identifier(value)
}

fn is_reserved_identifier(value: &str) -> bool {
    matches!(
        value,
        "alter"
            | "and"
            | "as"
            | "before"
            | "create"
            | "delete"
            | "drop"
            | "from"
            | "grant"
            | "insert"
            | "into"
            | "join"
            | "lock"
            | "merge"
            | "on"
            | "or"
            | "order"
            | "references"
            | "revoke"
            | "select"
            | "table"
            | "trigger"
            | "truncate"
            | "update"
            | "using"
            | "values"
            | "where"
            | "with"
    )
}

fn normalize_relation(value: &str) -> String {
    if let Ok(tokens) = lex(value) {
        if let Some((relation, next)) = relation_at(&tokens, 0) {
            if next == tokens.len() {
                return relation;
            }
        }
    }
    value.to_ascii_lowercase()
}

fn is_system_relation(relation: &str) -> bool {
    relation.starts_with("pg_catalog.") || relation.starts_with("information_schema.")
}

fn is_relation_keyword(relation: &str) -> bool {
    matches!(
        relation,
        "select" | "values" | "only" | "table" | "unnest" | "jsonb_array_elements" | "set"
    )
}

fn enclosing_function(tokens: &[Token], index: usize) -> Option<&str> {
    let mut depth = 0_u32;
    for cursor in (0..index).rev() {
        match tokens.get(cursor) {
            Some(Token::Symbol(')')) => depth += 1,
            Some(Token::Symbol('(')) if depth > 0 => depth -= 1,
            Some(Token::Symbol('(')) => {
                return cursor.checked_sub(1).and_then(|at| word_at(tokens, at))
            }
            _ => {}
        }
    }
    None
}

fn word_at(tokens: &[Token], index: usize) -> Option<&str> {
    match tokens.get(index) {
        Some(Token::Word(word)) => Some(word),
        _ => None,
    }
}

fn is_word(token: &Token, expected: &str) -> bool {
    matches!(token, Token::Word(word) if word == expected)
}

fn lex(input: &str) -> Result<Vec<Token>> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            byte if byte.is_ascii_whitespace() => index += 1,
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                let mut depth = 1_u32;
                while index < bytes.len() && depth > 0 {
                    if bytes.get(index..index + 2) == Some(b"/*") {
                        depth += 1;
                        index += 2;
                    } else if bytes.get(index..index + 2) == Some(b"*/") {
                        depth -= 1;
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
                if depth != 0 {
                    bail!("unclosed block comment in SQL");
                }
            }
            b'\'' => {
                index += 1;
                let mut value = String::new();
                loop {
                    if index >= bytes.len() {
                        bail!("unclosed SQL string literal");
                    }
                    if bytes[index] == b'\'' {
                        if bytes.get(index + 1) == Some(&b'\'') {
                            value.push('\'');
                            index += 2;
                        } else {
                            index += 1;
                            break;
                        }
                    } else {
                        value.push(bytes[index] as char);
                        index += 1;
                    }
                }
                tokens.push(Token::String(value));
            }
            b'"' => {
                index += 1;
                let mut value = Vec::new();
                loop {
                    if index >= bytes.len() {
                        bail!("unclosed quoted SQL identifier");
                    }
                    if bytes[index] == b'"' {
                        if bytes.get(index + 1) == Some(&b'"') {
                            value.push(b'"');
                            index += 2;
                        } else {
                            index += 1;
                            break;
                        }
                    } else {
                        value.push(bytes[index]);
                        index += 1;
                    }
                }
                tokens.push(Token::QuotedWord(
                    String::from_utf8(value).context("quoted SQL identifier is not UTF-8")?,
                ));
            }
            b'$' if dollar_delimiter(bytes, index).is_some() => {
                let delimiter = dollar_delimiter(bytes, index).expect("checked above");
                index += delimiter.len();
                let body_start = index;
                let Some(end) = find_bytes(bytes, index, delimiter) else {
                    bail!("unclosed dollar-quoted SQL body");
                };
                tokens.push(Token::DollarBody(input[body_start..end].to_string()));
                index = end + delimiter.len();
            }
            byte if is_word_byte(byte) => {
                let start = index;
                index += 1;
                while index < bytes.len() && is_word_byte(bytes[index]) {
                    index += 1;
                }
                tokens.push(Token::Word(input[start..index].to_ascii_lowercase()));
            }
            byte => {
                tokens.push(Token::Symbol(byte as char));
                index += 1;
            }
        }
    }
    Ok(tokens)
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

fn dollar_delimiter(bytes: &[u8], start: usize) -> Option<&[u8]> {
    if bytes.get(start) != Some(&b'$') {
        return None;
    }
    let mut end = start + 1;
    while let Some(byte) = bytes.get(end) {
        if *byte == b'$' {
            return Some(&bytes[start..=end]);
        }
        if !byte.is_ascii_alphanumeric() && *byte != b'_' {
            return None;
        }
        end += 1;
    }
    None
}

fn find_bytes(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

pub fn self_test() -> Result<()> {
    let accesses = relation_accesses(
        r#"
        WITH selected AS (SELECT id FROM public.users)
        INSERT INTO chat_messages(id)
        SELECT id FROM selected JOIN public.novels ON true;
        SELECT 'public.world_states'::pg_catalog.regclass;
        "#,
    )?;
    assert!(accesses.contains(&RelationAccess {
        relation: "users".into(),
        kind: AccessKind::Read,
    }));
    assert!(accesses.contains(&RelationAccess {
        relation: "novels".into(),
        kind: AccessKind::Read,
    }));
    assert!(
        accesses.contains(&RelationAccess {
            relation: "chat_messages".into(),
            kind: AccessKind::Write,
        }),
        "{accesses:?}"
    );
    assert!(accesses.contains(&RelationAccess {
        relation: "world_states".into(),
        kind: AccessKind::Metadata,
    }));
    assert!(!accesses.iter().any(|access| access.relation == "selected"));

    let scoped_ctes = relation_accesses(
        r#"
        SELECT id FROM public.shadowed;
        WITH shadowed AS (SELECT id FROM public.shadowed)
        SELECT id FROM shadowed;
        SELECT * FROM (
          WITH shadowed AS (SELECT id FROM public.inner_source)
          SELECT id FROM shadowed
        ) nested
        JOIN shadowed ON true;
        WITH later AS (SELECT id FROM public.later)
        SELECT id FROM later;
        WITH raw_source AS (SELECT id FROM raw_source)
        SELECT id FROM raw_source;
        WITH local_only AS (SELECT id FROM public.local_only)
        SELECT id FROM public.local_only;
        "#,
    )?;
    for relation in [
        "shadowed",
        "inner_source",
        "later",
        "raw_source",
        "local_only",
    ] {
        assert!(
            scoped_ctes.contains(&RelationAccess {
                relation: relation.into(),
                kind: AccessKind::Read,
            }),
            "scoped CTE hid real relation {relation}: {scoped_ctes:?}"
        );
    }
    assert_eq!(
        scoped_ctes
            .iter()
            .filter(|access| access.relation == "shadowed")
            .count(),
        1
    );

    let contextual = relation_accesses(
        "UPDATE chat_turns SET attempt = 2 WHERE id = $1; \
         INSERT INTO chat_turns(id) VALUES ($1) ON CONFLICT (id) DO UPDATE SET attempt = 3; \
         SELECT EXTRACT(EPOCH FROM lease_expires_at), substring(status FROM $8) FROM chat_turns \
         WHERE status IS DISTINCT FROM expected.status; \
         SELECT 'public.idx_chat_turns'::pg_catalog.regclass",
    )?;
    assert!(
        contextual
            .iter()
            .all(|access| { matches!(access.relation.as_str(), "chat_turns") }),
        "{contextual:?}"
    );

    let multi_relation = relation_accesses(
        r#"
        WITH chosen AS (SELECT id FROM "public"."users")
        SELECT *
        FROM chosen c,
             "public"."novels" n,
             LATERAL jsonb_array_elements(n.metadata) item,
             "audit"."events" event
        JOIN public.chapters chapter ON true
        WHERE EXISTS (SELECT 1 FROM public.chapter_chunks);
        "#,
    )?;
    for relation in [
        "users",
        "novels",
        "audit.events",
        "chapters",
        "chapter_chunks",
    ] {
        assert!(
            multi_relation.contains(&RelationAccess {
                relation: relation.into(),
                kind: AccessKind::Read,
            }),
            "missing {relation}: {multi_relation:?}"
        );
    }
    assert!(!multi_relation
        .iter()
        .any(|access| { matches!(access.relation.as_str(), "chosen" | "jsonb_array_elements") }));

    let quoted_relations = relation_accesses(
        r#"SELECT * FROM "Users";
            SELECT * FROM "public"."users";
            SELECT * FROM "select";
            SELECT * FROM "用户";"#,
    )?;
    for relation in ["\"Users\"", "users", "\"select\"", "\"用户\""] {
        assert!(quoted_relations.contains(&RelationAccess {
            relation: relation.into(),
            kind: AccessKind::Read,
        }));
    }
    for unicode_target in [
        "SELECT * FROM 用户",
        "SELECT * FROM public.users JOIN 章节 ON true",
    ] {
        assert!(
            relation_accesses(unicode_target).is_err(),
            "unquoted Unicode target must fail closed: {unicode_target}"
        );
    }

    let only_writes = relation_accesses(
        r#"
        UPDATE ONLY "public"."users" SET name = 'reader';
        DELETE FROM ONLY public.novels WHERE id = $1;
        TRUNCATE TABLE ONLY "public"."chat_turns";
        "#,
    )?;
    assert_eq!(
        only_writes,
        BTreeSet::from([
            RelationAccess {
                relation: "users".into(),
                kind: AccessKind::Write,
            },
            RelationAccess {
                relation: "novels".into(),
                kind: AccessKind::Write,
            },
            RelationAccess {
                relation: "chat_turns".into(),
                kind: AccessKind::Write,
            },
        ])
    );

    let truncate_targets = relation_accesses(
        "TRUNCATE TABLE ONLY public.users, ONLY public.novels, public.world_states RESTART IDENTITY",
    )?;
    assert_eq!(
        truncate_targets,
        BTreeSet::from([
            RelationAccess {
                relation: "users".into(),
                kind: AccessKind::Write,
            },
            RelationAccess {
                relation: "novels".into(),
                kind: AccessKind::Write,
            },
            RelationAccess {
                relation: "world_states".into(),
                kind: AccessKind::Write,
            },
        ])
    );

    let command_accesses = relation_accesses(
        r#"
        DELETE FROM ONLY public.users target
        USING "audit"."events" event, public.novels novel
        WHERE target.id = event.user_id;
        MERGE INTO ONLY public.world_states target
        USING public.player_chapters source
        ON target.user_id = source.user_id
        WHEN MATCHED THEN UPDATE SET state = source.content;
        TABLE "public"."chapters";
        LOCK TABLE public.profiles IN ROW EXCLUSIVE MODE;
        "#,
    )?;
    assert_eq!(
        command_accesses,
        BTreeSet::from([
            RelationAccess {
                relation: "users".into(),
                kind: AccessKind::Write,
            },
            RelationAccess {
                relation: "audit.events".into(),
                kind: AccessKind::Read,
            },
            RelationAccess {
                relation: "novels".into(),
                kind: AccessKind::Read,
            },
            RelationAccess {
                relation: "world_states".into(),
                kind: AccessKind::Write,
            },
            RelationAccess {
                relation: "player_chapters".into(),
                kind: AccessKind::Read,
            },
            RelationAccess {
                relation: "chapters".into(),
                kind: AccessKind::Read,
            },
            RelationAccess {
                relation: "profiles".into(),
                kind: AccessKind::Write,
            },
        ])
    );

    let adversarial = relation_accesses(
        r#"
        SELECT 'FROM hidden.string', $$ JOIN hidden.dollar; FROM hidden.body $$
        FROM public.users
        /* FROM hidden.block /* JOIN hidden.nested */ */
        -- JOIN hidden.line
        ;
        "#,
    )?;
    assert_eq!(
        adversarial,
        BTreeSet::from([RelationAccess {
            relation: "users".into(),
            kind: AccessKind::Read,
        }])
    );

    for unsupported in [
        "DO $$ BEGIN PERFORM id FROM public.users; END $$",
        "CALL refresh_projection()",
        "COPY public.users TO STDOUT",
        "EXECUTE selected_users",
        "PREPARE selected_users AS SELECT id FROM public.users",
        "CREATE FUNCTION hidden() RETURNS void AS $$ BEGIN DELETE FROM public.users; END $$ LANGUAGE plpgsql",
        "SELECT 1; DROP TABLE public.users",
        "ALTER TABLE public.users ADD COLUMN hidden integer",
        "GRANT SELECT ON public.users TO reader",
        "REVOKE SELECT ON public.users FROM reader",
        "LOCK TABLE",
        "REFRESH MATERIALIZED VIEW public.user_shelf",
        "VACUUM public.users",
        "SELECT id INTO copied_users FROM public.users",
        "SELECT id INTO TEMP copied_users FROM public.users",
        "SELECT id INTO TEMPORARY copied_users FROM public.users",
        "SELECT id INTO UNLOGGED copied_users FROM public.users",
        "WITH selected AS (SELECT id FROM public.users) SELECT id INTO copied_users FROM selected",
    ] {
        assert!(
            relation_accesses(unsupported).is_err(),
            "runtime SQL must fail closed: {unsupported}"
        );
    }

    let known_routines = BTreeSet::from([
        "search_memories".to_string(),
        "refresh_projection".to_string(),
    ]);
    assert_eq!(
        routine_calls(
            r#"SELECT * FROM "public"."search_memories"($1); CALL refresh_projection(); SELECT now()"#,
            &known_routines,
        )?,
        known_routines
    );

    assert_ne!(fingerprint("SELECT  1"), fingerprint("SELECT\n1"));
    assert_eq!(fingerprint("SELECT\r\n1"), fingerprint("SELECT\n1"));
    assert_ne!(
        fingerprint("SELECT 'reader action'"),
        fingerprint("SELECT 'reader  action'")
    );
    assert_ne!(
        fingerprint(r"SELECT E'reader\' action'"),
        fingerprint(r"SELECT E'reader\'  action'")
    );
    assert_ne!(
        fingerprint("SELECT $$reader action$$"),
        fingerprint("SELECT $$reader  action$$")
    );
    assert_ne!(
        fingerprint("SELECT 1 /* reader action */"),
        fingerprint("SELECT 1 /* reader  action */")
    );

    let schema = schema_snapshot(
        r#"
        CREATE TABLE users (id UUID PRIMARY KEY);
        CREATE TABLE profiles (id UUID PRIMARY KEY);
        CREATE TABLE messages (
          id UUID PRIMARY KEY,
          user_id UUID REFERENCES public.users(id) ON DELETE CASCADE,
          CONSTRAINT fk_other FOREIGN KEY (other_id) REFERENCES users(id)
        );
        ALTER TABLE ONLY "public"."messages"
          ADD CONSTRAINT fk_profile FOREIGN KEY (profile_id)
          REFERENCES "public"."profiles"(id) ON DELETE CASCADE;
        ALTER TABLE public.messages
          ADD FOREIGN KEY (parent_id) REFERENCES public.messages(id);
        ALTER TABLE ONLY public.messages
          ADD COLUMN IF NOT EXISTS owner_id UUID
          CONSTRAINT messages_owner_fkey REFERENCES public.users(id) ON DELETE SET NULL;
        CREATE OR REPLACE VIEW message_view AS
          SELECT messages.id, public.audit_message(messages.id)
          FROM messages JOIN profiles ON true;
        CREATE FUNCTION audit_message(p_id UUID)
        RETURNS void
        LANGUAGE sql
        AS $routine$
          SELECT 1;
        $routine$;
        CREATE OR REPLACE FUNCTION sync_message()
        RETURNS TRIGGER
        LANGUAGE plpgsql
        AS $routine$
        BEGIN
          INSERT INTO public.messages(id) SELECT id FROM public.users;
          PERFORM public.audit_message(NEW.id);
          RETURN NEW;
        END;
        $routine$;
        CREATE TRIGGER sync_message
          AFTER INSERT ON public.messages
          FOR EACH ROW EXECUTE FUNCTION public.sync_message();
        "#,
    )?;
    assert_eq!(
        schema.tables,
        BTreeSet::from(["messages".into(), "profiles".into(), "users".into()])
    );
    assert_eq!(schema.views, BTreeSet::from(["message_view".into()]));
    assert_eq!(schema.view_definitions.len(), 1);
    assert_eq!(
        schema.view_definitions.iter().next(),
        Some(&ViewDefinition {
            name: "message_view".into(),
            direct_accesses: BTreeSet::from([
                RelationAccess {
                    relation: "messages".into(),
                    kind: AccessKind::Read,
                },
                RelationAccess {
                    relation: "profiles".into(),
                    kind: AccessKind::Read,
                },
            ]),
            called_functions: BTreeSet::from(["audit_message".into()]),
        })
    );
    let sync_routine = schema
        .routines
        .iter()
        .find(|routine| routine.name == "sync_message")
        .expect("sync_message routine");
    assert_eq!(sync_routine.language, "plpgsql");
    assert_eq!(sync_routine.body_sha256.len(), 64);
    assert_eq!(
        sync_routine.called_functions,
        BTreeSet::from(["audit_message".into()])
    );
    assert_eq!(
        sync_routine.direct_accesses,
        BTreeSet::from([
            RelationAccess {
                relation: "messages".into(),
                kind: AccessKind::Write,
            },
            RelationAccess {
                relation: "users".into(),
                kind: AccessKind::Read,
            },
        ])
    );
    assert!(schema
        .routines
        .iter()
        .any(|routine| routine.name == "audit_message"));
    let sync_trigger = schema
        .triggers
        .iter()
        .find(|trigger| trigger.name == "sync_message")
        .expect("sync_message trigger");
    assert_eq!(sync_trigger.on_relation, "messages");
    assert_eq!(sync_trigger.function, "sync_message");
    assert_eq!(sync_trigger.timing, "after");
    assert_eq!(sync_trigger.events, BTreeSet::from(["insert".into()]));
    assert_eq!(sync_trigger.for_each, "row");
    assert_eq!(sync_trigger.when_sha256, None);
    assert!(sync_trigger.argument_sha256.is_empty());
    assert_eq!(sync_trigger.definition_sha256.len(), 64);
    assert_eq!(schema.foreign_keys.len(), 5);
    assert_eq!(
        schema
            .foreign_keys
            .iter()
            .filter(|foreign_key| foreign_key.on_delete == "cascade")
            .count(),
        2
    );
    assert!(schema.foreign_keys.contains(&ForeignKey {
        child: "messages".into(),
        child_columns: vec!["profile_id".into()],
        parent: "profiles".into(),
        parent_columns: vec!["id".into()],
        on_delete: "cascade".into(),
    }));

    for unsupported_routine in [
        r#"CREATE FUNCTION dynamic_sql() RETURNS void LANGUAGE plpgsql AS $$
              BEGIN EXECUTE 'DELETE FROM public.users'; END
           $$"#,
        r#"CREATE FUNCTION unknown_language() RETURNS void LANGUAGE plpython3u AS $$
              return None
           $$"#,
        r#"CREATE FUNCTION hidden_ddl() RETURNS void LANGUAGE plpgsql AS $$
              BEGIN CREATE TABLE hidden(id int); END
           $$"#,
        r#"CREATE FUNCTION unknown_statement() RETURNS void LANGUAGE plpgsql AS $$
              BEGIN MAGIC public.users; END
           $$"#,
        r#"CREATE FUNCTION disguised_statement() RETURNS void LANGUAGE plpgsql AS $$
              BEGIN MAGIC value = 1; END
           $$"#,
    ] {
        assert!(
            schema_snapshot(unsupported_routine).is_err(),
            "routine body must fail closed: {unsupported_routine}"
        );
    }
    assert!(schema.foreign_keys.contains(&ForeignKey {
        child: "messages".into(),
        child_columns: vec!["parent_id".into()],
        parent: "messages".into(),
        parent_columns: vec!["id".into()],
        on_delete: "no_action".into(),
    }));
    assert!(schema.foreign_keys.contains(&ForeignKey {
        child: "messages".into(),
        child_columns: vec!["owner_id".into()],
        parent: "users".into(),
        parent_columns: vec!["id".into()],
        on_delete: "set_null".into(),
    }));

    let migration = migration_snapshot(
        r#"
        BEGIN;
        CREATE TABLE public.parents (id UUID PRIMARY KEY);
        CREATE TABLE public.children (id UUID PRIMARY KEY, parent_id UUID);
        DO $migration$
        BEGIN
          ALTER TABLE ONLY public.children
            ADD CONSTRAINT children_parent_fkey
            FOREIGN KEY (parent_id) REFERENCES public.parents(id) ON DELETE CASCADE;
        END
        $migration$;
        ALTER TABLE ONLY public.children
          ADD COLUMN IF NOT EXISTS fallback_parent_id UUID
          CONSTRAINT children_fallback_parent_fkey
          REFERENCES public.parents(id) ON DELETE SET DEFAULT;
        CREATE OR REPLACE FUNCTION public.touch_child()
        RETURNS TRIGGER LANGUAGE plpgsql AS $$
        BEGIN
          UPDATE public.children SET id = NEW.id WHERE id = OLD.id;
          RETURN NEW;
        END
        $$;
        CREATE TRIGGER touch_child
          BEFORE UPDATE ON public.children
          FOR EACH ROW EXECUTE FUNCTION public.touch_child();
        COMMIT;
        "#,
    )?;
    assert!(migration.foreign_keys.contains(&ForeignKey {
        child: "children".into(),
        child_columns: vec!["parent_id".into()],
        parent: "parents".into(),
        parent_columns: vec!["id".into()],
        on_delete: "cascade".into(),
    }));
    assert!(migration.foreign_keys.contains(&ForeignKey {
        child: "children".into(),
        child_columns: vec!["fallback_parent_id".into()],
        parent: "parents".into(),
        parent_columns: vec!["id".into()],
        on_delete: "set_default".into(),
    }));
    let touch_child = migration
        .routines
        .iter()
        .find(|routine| routine.name == "touch_child")
        .expect("migration routine");
    assert_eq!(touch_child.language, "plpgsql");
    assert!(touch_child.direct_accesses.contains(&RelationAccess {
        relation: "children".into(),
        kind: AccessKind::Write,
    }));
    let touch_trigger = migration
        .triggers
        .iter()
        .find(|trigger| trigger.name == "touch_child")
        .expect("migration trigger");
    assert_eq!(touch_trigger.timing, "before");
    assert_eq!(touch_trigger.events, BTreeSet::from(["update".into()]));
    assert_eq!(touch_trigger.for_each, "row");

    for unsupported_migration in [
        "GRANT SELECT ON public.users TO reader",
        "REVOKE SELECT ON public.users FROM reader",
        "ALTER SEQUENCE public.sequence RESTART",
        "CREATE MATERIALIZED VIEW public.hidden AS SELECT * FROM public.users",
        "CREATE TEMP TABLE public.hidden(id int)",
        "CREATE TEMPORARY VIEW public.hidden AS SELECT 1",
        "CREATE UNLOGGED TABLE public.hidden(id int)",
        "DO $$ BEGIN EXECUTE 'DROP TABLE public.users'; END $$",
        "DO $$ BEGIN GRANT SELECT ON public.users TO reader; END $$",
        "DO LANGUAGE plpython3u $$ return None $$",
        "DO $$ BEGIN DELETE FROM public.users; END $$",
        "DO $$ BEGIN INSERT INTO public.users(id) VALUES (gen_random_uuid()); END $$",
        "DO $$ BEGIN UPDATE public.users SET name = 'changed'; END $$",
        "DO $$ BEGIN SELECT id FROM public.users; END $$",
        "DO $$ BEGIN PERFORM dblink_exec('remote', 'DELETE FROM users'); END $$",
        "DO $$ BEGIN pg_catalog.pg_sleep(1); END $$",
    ] {
        assert!(
            migration_snapshot(unsupported_migration).is_err(),
            "migration must fail closed: {unsupported_migration}"
        );
    }

    let canonical = canonical_schema_snapshot(
        r#"
        CREATE TABLE public.seeded (id UUID PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE public.seed_owners (id UUID PRIMARY KEY);
        ALTER TABLE ONLY public.seeded
          ADD COLUMN owner_id UUID CONSTRAINT seeded_owner_fkey
          REFERENCES public.seed_owners(id) ON DELETE CASCADE;
        INSERT INTO public.seeded (id, value) VALUES ($1, 'ready');
        UPDATE public.seeded SET value = 'checked' WHERE id = $1;
        "#,
    )?;
    assert_eq!(
        canonical.tables,
        BTreeSet::from(["seed_owners".into(), "seeded".into()])
    );
    assert!(canonical.foreign_keys.contains(&ForeignKey {
        child: "seeded".into(),
        child_columns: vec!["owner_id".into()],
        parent: "seed_owners".into(),
        parent_columns: vec!["id".into()],
        on_delete: "cascade".into(),
    }));
    let quoted_inventory = canonical_schema_snapshot(
        r#"CREATE TABLE users(id UUID);
            CREATE TABLE "Users"(id UUID);
            CREATE TABLE "public"."select"(id UUID);"#,
    )?;
    assert_eq!(
        quoted_inventory.tables,
        BTreeSet::from(["users".into(), "\"Users\"".into(), "\"select\"".into()])
    );
    for unsupported_canonical in [
        "DO $$ BEGIN EXECUTE 'CREATE TABLE public.hidden(id int)'; END $$",
        "CREATE MATERIALIZED VIEW public.hidden AS SELECT 1",
        "CREATE FOREIGN TABLE public.hidden(id int) SERVER remote",
        "CREATE TABLE public.valid(id int); DROP VIEW public.stale",
        "CREATE TABLE public.valid(id int); ANALYZE public.valid",
        "INSERT INTO",
        "CREATE TABLE public.valid(id int); INSERT INTO public.missing(id) VALUES (1)",
        "CREATE TEMP TABLE public.transient_state(id int)",
        "CREATE TEMPORARY VIEW public.transient_view AS SELECT 1",
        "CREATE UNLOGGED TABLE public.transient_state(id int)",
    ] {
        assert!(
            canonical_schema_snapshot(unsupported_canonical).is_err(),
            "canonical schema must fail closed: {unsupported_canonical}"
        );
    }

    let canonical_view = schema_snapshot(
        "CREATE VIEW public.reader_view AS SELECT public.audit_message(id) FROM public.messages",
    )?
    .view_definitions
    .into_iter()
    .next()
    .expect("canonical view definition");
    let drifted_view = schema_snapshot(
        "CREATE VIEW public.reader_view AS SELECT public.other_audit(id) FROM public.profiles",
    )?
    .view_definitions
    .into_iter()
    .next()
    .expect("drifted view definition");
    assert_ne!(canonical_view, drifted_view);

    let formatted_trigger = schema_snapshot(
        r#"
        CREATE TRIGGER semantic_trigger
          AFTER INSERT OR UPDATE OF status, name ON public.messages
          FOR EACH ROW
          WHEN (NEW.enabled = TRUE)
          EXECUTE FUNCTION public.sync_message('reader', 2);
        "#,
    )?
    .triggers
    .into_iter()
    .next()
    .expect("formatted trigger");
    let compact_trigger = schema_snapshot(
        r#"CREATE TRIGGER semantic_trigger AFTER INSERT OR UPDATE OF status,name
            ON messages FOR EACH ROW WHEN(NEW.enabled=TRUE)
            EXECUTE FUNCTION sync_message('reader',2);"#,
    )?
    .triggers
    .into_iter()
    .next()
    .expect("compact trigger");
    assert_eq!(formatted_trigger, compact_trigger);
    assert_eq!(
        formatted_trigger.events,
        BTreeSet::from(["insert".into(), "update_of:name,status".into()])
    );
    assert!(formatted_trigger.when_sha256.is_some());
    assert_eq!(formatted_trigger.argument_sha256.len(), 2);

    for drifted in [
        "CREATE TRIGGER semantic_trigger BEFORE INSERT OR UPDATE OF status, name ON messages FOR EACH ROW WHEN (NEW.enabled = TRUE) EXECUTE FUNCTION sync_message('reader', 2)",
        "CREATE TRIGGER semantic_trigger AFTER DELETE ON messages FOR EACH ROW WHEN (NEW.enabled = TRUE) EXECUTE FUNCTION sync_message('reader', 2)",
        "CREATE TRIGGER semantic_trigger AFTER INSERT OR UPDATE OF status, name ON messages FOR EACH ROW WHEN (NEW.enabled = FALSE) EXECUTE FUNCTION sync_message('reader', 2)",
        "CREATE TRIGGER semantic_trigger AFTER INSERT OR UPDATE OF status, name ON messages FOR EACH ROW WHEN (NEW.enabled = TRUE) EXECUTE FUNCTION sync_message('different', 2)",
    ] {
        let drifted = schema_snapshot(drifted)?
            .triggers
            .into_iter()
            .next()
            .expect("drifted trigger");
        assert_ne!(
            formatted_trigger.definition_sha256, drifted.definition_sha256,
            "trigger semantic drift must change identity"
        );
    }

    let repository_schema = schema_snapshot(include_str!("../../../infra/postgres/init.sql"))?;
    let shelf = repository_schema
        .view_definitions
        .iter()
        .find(|view| view.name == "user_shelf")
        .expect("repository user_shelf view");
    for relation in ["user_novels", "novels", "reading_progress"] {
        assert!(shelf.direct_accesses.contains(&RelationAccess {
            relation: relation.into(),
            kind: AccessKind::Read,
        }));
    }
    let search_memories = repository_schema
        .routines
        .iter()
        .find(|routine| routine.name == "search_memories")
        .expect("repository search_memories routine");
    assert_eq!(search_memories.language, "plpgsql");
    assert!(search_memories.direct_accesses.contains(&RelationAccess {
        relation: "character_memories".into(),
        kind: AccessKind::Read,
    }));
    let record_user_trigger = repository_schema
        .triggers
        .iter()
        .find(|trigger| trigger.name == "record_user_erasure")
        .expect("record_user_erasure trigger");
    assert_eq!(record_user_trigger.on_relation, "users");
    assert_eq!(record_user_trigger.function, "record_user_erasure");
    assert_eq!(record_user_trigger.timing, "after");
    assert_eq!(
        record_user_trigger.events,
        BTreeSet::from(["delete".into()])
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn sql_contracts() {
        super::self_test().unwrap();
    }
}
