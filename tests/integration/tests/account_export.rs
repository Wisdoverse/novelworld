use agent_service::{
    domain::ports::AccountExportPort as AgentAccountExportPort,
    infrastructure::persistence::account_export::PgAccountExport as AgentAccountExport,
};
use futures::StreamExt;
use narrative_service::{
    domain::ports::AccountExportPort as NarrativeAccountExportPort,
    infrastructure::persistence::account_export::PgAccountExport as NarrativeAccountExport,
};
use novel_service::{
    domain::ports::AccountExportPort as NovelAccountExportPort,
    infrastructure::persistence::account_export::PgAccountExport as NovelAccountExport,
};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use std::collections::BTreeSet;
use uuid::Uuid;

fn db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://test:test@localhost:25432/novelworld_test".into())
}

#[tokio::test]
async fn production_account_exports_are_complete_scoped_deterministic_and_secret_free() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    let other_user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let other_novel_id = Uuid::new_v4();
    let chapter_id = Uuid::new_v4();
    let character_id = Uuid::new_v4();
    let second_character_id = Uuid::new_v4();
    let canonical_node_id = Uuid::new_v4();
    let player_node_id = Uuid::new_v4();
    let other_private_node_id = Uuid::new_v4();

    for (id, marker) in [(user_id, "owner"), (other_user_id, "other-user-marker")] {
        sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(format!("account-export-{marker}-{id}@test.invalid"))
            .bind(format!("SENTINEL_PASSWORD_{marker}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, cover_url, original_file_key, status) \
         VALUES ($1, $2, 'Portable novel', 'https://assets.invalid/cover', \
                 'SENTINEL_OBJECT_KEY', 'ready'), \
                ($3, $4, 'other-user-marker novel', NULL, NULL, 'ready')",
    )
    .bind(novel_id)
    .bind(user_id)
    .bind(other_novel_id)
    .bind(other_user_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2), ($3, $4)")
        .bind(user_id)
        .bind(novel_id)
        .bind(other_user_id)
        .bind(other_novel_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO chapters (id, novel_id, chapter_number, title, content) \
         VALUES ($1, $2, 1, 'Beginning', 'Portable source chapter'), \
                (uuid_generate_v4(), $3, 1, NULL, 'other-user-marker chapter')",
    )
    .bind(chapter_id)
    .bind(novel_id)
    .bind(other_novel_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO characters (id, novel_id, name, avatar_url, personality) \
         VALUES ($1, $3, 'Aster', 'https://assets.invalid/avatar', 'patient'), \
                ($2, $3, 'Bryn', NULL, 'bold'), \
                (uuid_generate_v4(), $4, 'other-user-marker character', NULL, NULL)",
    )
    .bind(character_id)
    .bind(second_character_id)
    .bind(novel_id)
    .bind(other_novel_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO character_relationships \
         (novel_id, from_character_id, to_character_id, relationship_type, description) \
         VALUES ($1, $2, $3, 'ally', 'Trusted ally')",
    )
    .bind(novel_id)
    .bind(character_id)
    .bind(second_character_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO canon_story_models \
         (novel_id, model_version, schema_version, prompt_version, content) \
         VALUES ($1, 1, 1, 'account-export-test', $2), \
                ($1, 2, 1, 'account-export-test', $3), \
                ($1, 3, 1, 'account-export-test', $4), \
                ($1, 4, 1, 'account-export-test', $5)",
    )
    .bind(novel_id)
    .bind(json!({"world": "portable canon"}))
    .bind(json!({"events": [{"evidence": {"confidence": 1.0}}]}))
    .bind(json!({"events": [{"evidence": {"confidence": 0.7}}]}))
    .bind(json!({"events": [
        {"evidence": {"confidence": 1.0}},
        {"evidence": {"confidence": 0.7}},
    ]}))
    .execute(&pool)
    .await
    .unwrap();
    // H4 identity boundary: one progress row adopts character identity so the
    // export must prove identity data is portable (and stays out of canon).
    let identity_character_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO characters (id, novel_id, name, first_appearance_chapter) \
         VALUES ($1, $2, 'Portable identity', 1)",
    )
    .bind(identity_character_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO reading_progress (user_id, novel_id, current_chapter, reader_identity_type, \
         reader_identity, reader_character_id) \
         VALUES ($1, $2, 1, 'character', 'Portable identity', $4), ($1, $3, 1, 'self', NULL, NULL)",
    )
    .bind(user_id)
    .bind(novel_id)
    .bind(other_novel_id)
    .bind(identity_character_id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO chat_messages \
         (character_id, user_id, novel_id, role, content, chapter_context) \
         VALUES ($1, $2, $3, 'user', 'Portable conversation', 1)",
    )
    .bind(character_id)
    .bind(user_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    let embedding = format!("[0.3141592653589793,{}]", vec!["0"; 1535].join(","));
    sqlx::query(
        "INSERT INTO character_memories \
         (character_id, user_id, novel_id, layer, content, importance, chapter_number, \
          embedding, access_count, last_accessed) \
         VALUES ($1, $2, $3, 'long', 'Portable memory', 9, 1, $4::vector, 777777, NOW())",
    )
    .bind(character_id)
    .bind(user_id)
    .bind(novel_id)
    .bind(embedding)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO chat_turns \
         (id, user_id, character_id, novel_id, request_fingerprint, chapter_context, \
          reader_identity_type, deviation_mode, status, failure_code) \
         VALUES ($1, $2, $3, $4, $5, 1, 'self', 'canon', 'failed', \
                 'SENTINEL_CHAT_FAILURE')",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(character_id)
    .bind(novel_id)
    .bind(vec![0x5a_u8; 32])
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO narrative_nodes \
         (id, novel_id, chapter_number, description, choices) \
         VALUES ($1, $2, 1, 'Portable canonical branch', '[]'), \
                (uuid_generate_v4(), $3, 1, 'other-user-marker branch', '[]')",
    )
    .bind(canonical_node_id)
    .bind(novel_id)
    .bind(other_novel_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO narrative_nodes \
         (id, user_id, novel_id, chapter_number, description, choices) \
         VALUES ($1, $2, $3, 2, 'Portable player branch', '[]'), \
                ($4, $5, $6, 2, 'other-user-marker player branch', '[]')",
    )
    .bind(player_node_id)
    .bind(user_id)
    .bind(novel_id)
    .bind(other_private_node_id)
    .bind(other_user_id)
    .bind(other_novel_id)
    .execute(&pool)
    .await
    .unwrap();
    let transition = json!({
        "schema_version": 1,
        "prompt_version": "account-export-test",
        "canon_model_version": 1,
        "canonical_checkpoint_chapter": 1,
        "rendered_narrative": "Portable consequence",
        "events": [],
        "relationship_changes": [],
        "location_changes": [],
        "thread_changes": []
    });
    // Simulate legacy/dirty data that points at another reader's private node.
    // The choice belongs to this user, but the private node content must not.
    sqlx::query(
        "INSERT INTO user_choices \
         (user_id, novel_id, node_id, chapter_number, choice_index, choice_text, \
          consequence, transition) \
         VALUES ($1, $2, $3, 2, 0, 'Legacy scoped choice', 'Portable consequence', $4)",
    )
    .bind(user_id)
    .bind(other_novel_id)
    .bind(other_private_node_id)
    .bind(&transition)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_choices \
         (user_id, novel_id, node_id, chapter_number, choice_index, choice_text, \
          consequence, transition) \
         VALUES ($1, $2, $3, 1, 0, 'Portable choice', 'Portable consequence', $4)",
    )
    .bind(user_id)
    .bind(novel_id)
    .bind(canonical_node_id)
    .bind(&transition)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO world_states (user_id, novel_id, state) VALUES ($1, $2, $3), ($4, $5, $6)",
    )
    .bind(user_id)
    .bind(novel_id)
    .bind(json!({"world_events": ["portable"]}))
    .bind(other_user_id)
    .bind(other_novel_id)
    .bind(json!({"world_events": ["other-user-marker"]}))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO world_turns \
         (id, user_id, novel_id, request_fingerprint, action, expected_turn_number, \
          status, failure_code) \
         VALUES ($1, $2, $3, $4, $5, 0, 'failed', 'test_failure'), \
                ($6, $7, $8, $9, $10, 0, 'failed', 'test_failure')",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(novel_id)
    .bind(vec![0x11_u8; 32])
    .bind(json!({"kind": "investigate", "target_id": "thread", "intent": "Portable world action"}))
    .bind(Uuid::new_v4())
    .bind(other_user_id)
    .bind(other_novel_id)
    .bind(vec![0x12_u8; 32])
    .bind(json!({"kind": "investigate", "target_id": "thread", "intent": "other-user-marker world action"}))
    .execute(&pool)
    .await
    .unwrap();
    // A completed turn carries generated prose: source must be 'mixed'.
    sqlx::query(
        "INSERT INTO world_turns \
         (id, user_id, novel_id, request_fingerprint, action, expected_turn_number, \
          status, transition, result, completed_at) \
         VALUES ($1, $2, $3, $4, $5, 1, 'completed', $6, $7, NOW())",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(novel_id)
    .bind(vec![0x13_u8; 32])
    .bind(json!({"kind": "investigate", "target_id": "thread", "intent": "Portable completed action"}))
    .bind(json!({"rendered_narrative": "Portable generated consequence", "events": []}))
    .bind(json!({"turn_id": Uuid::new_v4()}))
    .execute(&pool)
    .await
    .unwrap();
    // An open-world state combines reader actions with generated prose:
    // source must be 'mixed'. world_states is unique per (user, novel), so
    // the open-world state lives under a second novel of the same user.
    let open_world_novel_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, total_chapters, status)          VALUES ($1, $2, 'Open world novel', 1, 'ready')",
    )
    .bind(open_world_novel_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(open_world_novel_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO world_states (user_id, novel_id, state) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(open_world_novel_id)
        .bind(json!({"open_world": {"turn_number": 1}}))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO player_chapters (user_id, novel_id, chapter_number, content, origin) \
         VALUES ($1, $2, 2, 'Portable generated chapter', 'continuation'), \
                ($1, $2, 3, 'Portable choice chapter', 'choice'), \
                ($3, $4, 2, 'other-user-marker generated chapter', 'continuation')",
    )
    .bind(user_id)
    .bind(novel_id)
    .bind(other_user_id)
    .bind(other_novel_id)
    .execute(&pool)
    .await
    .unwrap();

    let novel_export = NovelAccountExport::new(pool.clone());
    let mut novel_stream = NovelAccountExportPort::export_user(&novel_export, user_id);
    let mut novel_records = Vec::new();
    while let Some(record) = novel_stream.next().await {
        let record = record.unwrap();
        novel_records.push(json!({"kind": record.kind, "data": record.data}));
    }
    let mut second_novel_stream = NovelAccountExportPort::export_user(&novel_export, user_id);
    let mut second_novel_records = Vec::new();
    while let Some(record) = second_novel_stream.next().await {
        let record = record.unwrap();
        second_novel_records.push(json!({"kind": record.kind, "data": record.data}));
    }
    assert_eq!(novel_records, second_novel_records);
    assert_eq!(
        novel_records
            .iter()
            .filter(|record| record["kind"] == "reading_progress")
            .count(),
        2
    );
    // H4 identity boundary: character identity (type, name, character id) is
    // portable account data carried by the export; it never becomes canon.
    let identity_progress: Vec<_> = novel_records
        .iter()
        .filter(|record| {
            record["kind"] == "reading_progress"
                && record["data"]["reader_identity_type"] == "character"
        })
        .collect();
    assert_eq!(identity_progress.len(), 1);
    assert_eq!(
        identity_progress[0]["data"]["reader_identity"],
        "Portable identity"
    );
    assert_eq!(
        identity_progress[0]["data"]["reader_character_id"],
        identity_character_id.to_string()
    );

    let agent_export = AgentAccountExport::new(pool.clone());
    let mut agent_stream = AgentAccountExportPort::export_user(&agent_export, user_id);
    let mut agent_records = Vec::new();
    while let Some(record) = agent_stream.next().await {
        let record = record.unwrap();
        agent_records.push(json!({"kind": record.kind, "data": record.data}));
    }
    let mut second_agent_stream = AgentAccountExportPort::export_user(&agent_export, user_id);
    let mut second_agent_records = Vec::new();
    while let Some(record) = second_agent_stream.next().await {
        let record = record.unwrap();
        second_agent_records.push(json!({"kind": record.kind, "data": record.data}));
    }
    assert_eq!(agent_records, second_agent_records);

    let narrative_export = NarrativeAccountExport::new(pool.clone());
    let mut narrative_stream = NarrativeAccountExportPort::export_user(&narrative_export, user_id);
    let mut narrative_records = Vec::new();
    while let Some(record) = narrative_stream.next().await {
        let record = record.unwrap();
        narrative_records.push(json!({"kind": record.kind, "data": record.data}));
    }
    let mut second_narrative_stream =
        NarrativeAccountExportPort::export_user(&narrative_export, user_id);
    let mut second_narrative_records = Vec::new();
    while let Some(record) = second_narrative_stream.next().await {
        let record = record.unwrap();
        second_narrative_records.push(json!({"kind": record.kind, "data": record.data}));
    }
    assert_eq!(narrative_records, second_narrative_records);

    assert_eq!(
        kinds(&novel_records),
        BTreeSet::from([
            "canon_story_model",
            "chapter",
            "character",
            "character_relationship",
            "novel",
            "reading_progress",
        ])
    );
    // H4: the canon-story-model records distinguish confident extraction
    // (all facts confidence 1.0 -> canon) from uncertain extraction (missing
    // or below-1.0 confidence -> uncertain). The roadmap forbids silently
    // promoting uncertainty to canon, so confidence-less content is
    // uncertain, not canon.
    for record in &novel_records {
        if record["kind"] != "canon_story_model" {
            continue;
        }
        let source = record["data"]["source"].as_str().unwrap();
        match record["data"]["model_version"].as_i64().unwrap() {
            1 => assert_eq!(
                source, "uncertain",
                "confidence-less content must stay uncertain"
            ),
            4 => assert_eq!(
                source, "uncertain",
                "any sub-1.0 fact makes a mixed-confidence model uncertain"
            ),
            2 => assert_eq!(source, "canon", "all-confident facts are canon"),
            3 => assert_eq!(source, "uncertain", "a sub-1.0 fact is uncertain"),
            version => panic!("unexpected canon model version {version}"),
        }
    }
    assert_eq!(
        kinds(&agent_records),
        BTreeSet::from(["character_memory", "chat_message"])
    );
    assert_eq!(
        kinds(&narrative_records),
        BTreeSet::from([
            "narrative_node",
            "player_chapter",
            "user_choice",
            "world_turn",
            "world_state",
        ])
    );

    // H4: every narrative record carries a uniform source label so canonical
    // history, reader-created history, and generated prose are programmatically
    // separable in the export (H4 exit evidence: export preserves the
    // canon/player distinction).
    let record_sources: BTreeSet<String> = narrative_records
        .iter()
        .map(|record| {
            record["data"]["source"]
                .as_str()
                .expect("every narrative record must carry a source")
                .to_string()
        })
        .collect();
    assert_eq!(
        record_sources,
        BTreeSet::from([
            "canon".to_string(),
            "generated".to_string(),
            "reader".to_string(),
            "mixed".to_string(),
        ]),
        "the export must distinguish canon, reader-created, generated, and mixed records"
    );
    for record in &narrative_records {
        let kind = record["kind"].as_str().unwrap();
        let source = record["data"]["source"].as_str().unwrap();
        let description = record["data"]["description"]
            .as_str()
            .or_else(|| record["data"]["content"].as_str())
            .or_else(|| record["data"]["choice_text"].as_str())
            .or_else(|| record["data"]["action"]["intent"].as_str())
            .unwrap_or("");
        match (kind, source) {
            // Canon nodes are source-anchored; player-owned nodes are
            // LLM-generated branches.
            ("narrative_node", "canon") => assert_eq!(description, "Portable canonical branch"),
            ("narrative_node", "generated") => {
                assert_eq!(description, "Portable player branch")
            }
            ("user_choice", "reader") => {
                assert!(
                    matches!(description, "Portable choice" | "Legacy scoped choice"),
                    "reader choices must carry their text"
                )
            }
            // The seeded failed turns have no transition: reader-only.
            ("world_turn", "reader") => assert!(description.contains("Portable world action")),
            // The seeded completed turn carries generated prose: mixed.
            ("world_turn", "mixed") => {
                assert_eq!(description, "Portable completed action");
                assert_eq!(
                    record["data"]["transition"]["rendered_narrative"]
                        .as_str()
                        .unwrap(),
                    "Portable generated consequence"
                );
            }
            ("player_chapter", "generated") => {
                assert_eq!(description, "Portable generated chapter")
            }
            ("player_chapter", "reader") => assert_eq!(description, "Portable choice chapter"),
            // The seeded plain world state never opened a world: reader-only.
            ("world_state", "reader") => {}
            // The seeded open-world state combines reader actions with
            // generated prose: mixed.
            ("world_state", "mixed") => {
                assert!(record["data"]["state"]["open_world"].is_object())
            }
            (kind, source) => panic!("unexpected kind/source pair {kind}/{source}"),
        }
    }

    let serialized = serde_json::to_string(&json!({
        "novel": novel_records,
        "agent": agent_records,
        "narrative": narrative_records,
    }))
    .unwrap();
    for portable in [
        "Portable source chapter",
        "Portable conversation",
        "Portable memory",
        "Portable canonical branch",
        "Portable player branch",
        "Portable choice",
        "Portable generated chapter",
        "Portable choice chapter",
        "Portable world action",
        "Portable completed action",
        "Portable generated consequence",
        "https://assets.invalid/cover",
        "https://assets.invalid/avatar",
    ] {
        assert!(serialized.contains(portable), "missing {portable}");
    }
    for excluded in [
        "other-user-marker",
        "SENTINEL_PASSWORD",
        "SENTINEL_OBJECT_KEY",
        "SENTINEL_CHAT_FAILURE",
        "0.3141592653589793",
        "password_hash",
        "original_file_key",
        "request_fingerprint",
        "failure_code",
        "embedding",
        "access_count",
        "last_accessed",
    ] {
        assert!(!serialized.contains(excluded), "leaked {excluded}");
    }

    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![user_id, other_user_id])
        .execute(&pool)
        .await
        .unwrap();
}

fn kinds(records: &[Value]) -> BTreeSet<&str> {
    records
        .iter()
        .map(|record| record["kind"].as_str().unwrap())
        .collect()
}
