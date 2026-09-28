//! Real PostgreSQL contract helper, called by the integration package's fixture.
//! The caller supplies distinct existing users and a source with Canon v1.
//! This helper writes only Novel-owned relations and never invokes a provider.
use chrono::{Timelike, Utc};
use futures::TryStreamExt;
use novel_service::{
    application::world_series::{
        CreateWorldSeries, SeriesSuggestionStatus, WorldSeriesApplicationError, WorldSeriesHandler,
    },
    domain::{
        entities::{
            canon_story_model::{
                CanonEndingSnapshot, CanonEvent, CanonLocation, CanonStoryContent, CanonStoryModel,
                SourceCitation, SourceEvidence, StoryArc,
            },
            game_rule_template::{
                basic_attribute, GameActionKind, GameActionRule, GameAttribute, GameRuleTemplate,
                BASIC_ACTION_DESCRIPTION,
            },
            world_series::WorldSeries,
        },
        ports::{
            series_matcher::{
                BeginSeriesMatch, SeriesBookMetadata, SeriesMatchCandidate, SeriesMatchMethod,
                SeriesMatchReason, SeriesMatcherPort, SeriesSuggestion,
            },
            AccountExportPort, LlmPort, NovelLlmTask, SeriesCompletionPort,
        },
        repositories::{CanonStoryModelRepository, CreateWorldSeriesResult, WorldSeriesRepository},
    },
    infrastructure::persistence::{
        account_export::PgAccountExport, canon_story_model_pg_repo::PgCanonStoryModelRepository,
        novel_pg_repo::NovelPgRepository, world_series_pg_repo::PgWorldSeriesRepository,
    },
};
use sqlx::PgPool;
use std::sync::{
    atomic::{AtomicI32, AtomicUsize, Ordering},
    Arc,
};
use uuid::Uuid;

struct MatcherSpy(AtomicUsize, AtomicI32);

struct CompletionSpy {
    calls: Arc<AtomicUsize>,
    identity: std::sync::Mutex<String>,
    fails: Arc<std::sync::atomic::AtomicBool>,
}
struct PreparedSpy {
    calls: Arc<AtomicUsize>,
    identity: String,
    fails: Arc<std::sync::atomic::AtomicBool>,
}
#[async_trait::async_trait]
impl LlmPort for CompletionSpy {
    async fn chat_json(&self, _: Uuid, _: NovelLlmTask, _: &str) -> anyhow::Result<String> {
        anyhow::bail!("unexpected ordinary chat")
    }
    async fn prepare_series_match(&self, _: Uuid) -> anyhow::Result<Box<dyn SeriesCompletionPort>> {
        Ok(Box::new(PreparedSpy {
            calls: self.calls.clone(),
            identity: self.identity.lock().unwrap().clone(),
            fails: self.fails.clone(),
        }))
    }
}
#[async_trait::async_trait]
impl SeriesCompletionPort for PreparedSpy {
    fn identity(&self) -> &str {
        &self.identity
    }
    async fn complete(&self, prompt: &str) -> anyhow::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(!prompt.contains("source_novel_id"));
        assert!(!prompt.contains("series_id"));
        assert!(!prompt.contains("secret ending"));
        assert!(!prompt.contains("Hidden Tower"));
        assert!(!prompt.contains("Moon Guild"));
        if self.fails.load(Ordering::SeqCst) {
            anyhow::bail!("synthetic unknown provider outcome")
        }
        Ok(r#"{"choice":0,"same_world":true,"basis":"explicit_series"}"#.into())
    }
}
#[async_trait::async_trait]
impl SeriesMatcherPort for MatcherSpy {
    async fn suggest(
        &self,
        target: &SeriesBookMetadata,
        candidates: &[SeriesMatchCandidate],
    ) -> anyhow::Result<Option<usize>> {
        let calls = self.0.fetch_add(1, Ordering::SeqCst);
        if calls == 0 {
            assert_eq!(candidates.len(), 8);
        } else {
            assert!(!candidates.is_empty() && candidates.len() <= 8);
        }
        assert!(!target.world_entities.contains(&"Hidden Tower".into()));
        assert!(candidates.iter().all(|candidate| {
            !candidate
                .book
                .world_entities
                .contains(&"Hidden Tower".into())
        }));
        match self.1.load(Ordering::SeqCst) {
            -2 => Err(anyhow::anyhow!("fixture classifier unavailable")),
            -1 => Ok(None),
            index => Ok(Some(usize::try_from(index).unwrap())),
        }
    }
}

fn command(source: Uuid, name: &str) -> CreateWorldSeries {
    CreateWorldSeries {
        name: name.into(),
        background: "用户确认的共同世界设定".into(),
        source_novel_id: source,
        canon_model_version: None,
    }
}

fn source_template(source: Uuid) -> GameRuleTemplate {
    let attributes = ["vigor", "insight", "influence"]
        .into_iter()
        .map(|key| {
            let (label, description) = basic_attribute(key).unwrap();
            GameAttribute {
                key: key.into(),
                label: label.into(),
                description: description.into(),
                default_score: 10,
                source_chapters: vec![1],
            }
        })
        .collect();
    let actions = GameActionKind::ALL
        .into_iter()
        .map(|kind| GameActionRule {
            kind,
            attribute_key: "vigor".into(),
            difficulty_class: 12,
            description: BASIC_ACTION_DESCRIPTION.into(),
            source_chapters: vec![1],
        })
        .collect();
    GameRuleTemplate::new_basic(source, 1, attributes, actions).unwrap()
}

pub async fn run(pool: &PgPool, user: Uuid, other_user: Uuid, source: Uuid) {
    assert_ne!(user, other_user);
    sqlx::query(
        "INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(user)
    .bind(source)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE novels SET status = 'ready'::novel_status WHERE id = $1")
        .bind(source)
        .execute(pool)
        .await
        .unwrap();
    let target = Uuid::new_v4();
    sqlx::query("INSERT INTO novels (id, user_id, title, total_chapters, status) VALUES ($1, $2, 'Series target book', 1, 'ready'::novel_status)")
        .bind(target).bind(user).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2)")
        .bind(user)
        .bind(target)
        .execute(pool)
        .await
        .unwrap();
    let mut extra_books = Vec::new();
    for index in 0..10 {
        let book = Uuid::new_v4();
        sqlx::query("INSERT INTO novels (id, user_id, title, total_chapters, status) VALUES ($1, $2, $3, 1, 'ready'::novel_status)")
            .bind(book).bind(user).bind(format!("Candidate book {index}" )).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2)")
            .bind(user)
            .bind(book)
            .execute(pool)
            .await
            .unwrap();
        extra_books.push(book);
    }
    let repository = Arc::new(PgWorldSeriesRepository::new(pool.clone()));
    let canon = Arc::new(PgCanonStoryModelRepository::new(pool.clone()));
    let matcher = Arc::new(MatcherSpy(AtomicUsize::new(0), AtomicI32::new(0)));
    let handler = WorldSeriesHandler {
        series_repo: repository.clone(),
        novel_repo: Arc::new(NovelPgRepository::new(pool.clone())),
        canon_repo: canon.clone(),
        matcher: Some(matcher.clone()),
        llm: None,
    };

    // Missing source rules create a pending series and source association only.
    let mut pending = handler
        .create(user, command(source, "Confirmed series"))
        .await
        .unwrap();
    pending.created_at = pending
        .created_at
        .with_nanosecond(pending.created_at.nanosecond() / 1_000 * 1_000)
        .unwrap();
    assert_eq!(pending.source_novel_id, source);
    assert!(pending.source_template.is_none());
    assert!(pending.rules_for(target).is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM novel_game_rule_templates WHERE novel_id = $1"
        )
        .bind(source)
        .fetch_one(pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        repository.find_for_novel(user, source).await.unwrap(),
        Some(pending.clone())
    );
    let pending_export = PgAccountExport::new(pool.clone())
        .export_user(user)
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    let exported_series = pending_export
        .iter()
        .find(|row| row.kind == "world_series")
        .unwrap();
    assert_eq!(exported_series.data["source_novel_id"], source.to_string());
    assert!(exported_series.data["source_template"].is_null());
    assert!(handler
        .bind_ready_source(user, pending.id)
        .await
        .unwrap()
        .source_template
        .is_none());
    let template = source_template(source);
    sqlx::query("INSERT INTO novel_game_rule_templates (novel_id,canon_model_version,schema_version,prompt_version,status,attempt,content,completed_at) VALUES ($1,1,1,'novel-game-rules-v2','ready',1,$2,NOW())")
        .bind(source).bind(serde_json::to_value(&template).unwrap()).execute(pool).await.unwrap();
    let baseline = sqlx::query_scalar::<_, serde_json::Value>("SELECT jsonb_agg(to_jsonb(t) ORDER BY prompt_version) FROM novel_game_rule_templates t WHERE novel_id = $1")
        .bind(source).fetch_one(pool).await.unwrap();
    handler.associate(user, source, None).await.unwrap();
    assert!(handler
        .bind_ready_source(user, pending.id)
        .await
        .unwrap()
        .source_template
        .is_none());
    handler
        .associate(user, source, Some(pending.id))
        .await
        .unwrap();
    assert!(matches!(
        handler
            .create(other_user, command(source, "Foreign source"))
            .await,
        Err(WorldSeriesApplicationError::NotFound)
    ));
    let mut wrong = command(source, "Wrong version");
    wrong.canon_model_version = Some(2);
    assert!(matches!(
        handler.create(user, wrong).await,
        Err(WorldSeriesApplicationError::SourceUnavailable)
    ));
    let mut series = handler.bind_ready_source(user, pending.id).await.unwrap();
    // PostgreSQL timestamptz stores microseconds; compare the complete frozen
    // definition at the database's precision without changing runtime values.
    series.created_at = series
        .created_at
        .with_nanosecond(series.created_at.nanosecond() / 1_000 * 1_000)
        .unwrap();
    assert_eq!(
        repository.find_for_novel(user, source).await.unwrap(),
        Some(series.clone())
    );
    assert_eq!(series.source_template, Some(template.clone()));
    assert_eq!(
        handler.bind_ready_source(user, pending.id).await.unwrap(),
        series
    );
    assert!(
        sqlx::query("UPDATE user_world_series SET source_template = NULL WHERE id = $1")
            .bind(series.id)
            .execute(pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE user_world_series SET name = 'rewritten' WHERE id = $1")
            .bind(series.id)
            .execute(pool)
            .await
            .is_err()
    );
    assert!(repository
        .find(other_user, series.id)
        .await
        .unwrap()
        .is_none());
    assert!(!repository
        .associate(other_user, target, Some(series.id))
        .await
        .unwrap());
    assert!(matches!(
        handler
            .create(user, command(source, "Do not replace"))
            .await,
        Err(WorldSeriesApplicationError::SourceAlreadyAssociated)
    ));
    assert_eq!(repository.list(user).await.unwrap().len(), 1);

    let frozen = handler
        .frozen_rules(user, target, series.id, 1, 1, false)
        .await
        .unwrap();
    assert!(handler
        .frozen_rules(user, target, series.id, 1, 1, true)
        .await
        .is_err());
    assert!(handler
        .frozen_rules(user, target, series.id, 2, 1, false)
        .await
        .is_err());
    assert!(handler
        .frozen_rules(user, target, series.id, 1, 2, false)
        .await
        .is_err());
    assert!(handler
        .frozen_rules(other_user, target, series.id, 1, 1, false)
        .await
        .is_err());
    handler
        .associate(user, target, Some(series.id))
        .await
        .unwrap();
    assert_eq!(
        handler
            .frozen_rules(user, target, series.id, 1, 1, true)
            .await
            .unwrap(),
        frozen
    );
    assert_eq!(frozen.novel_id, source);
    assert_eq!(frozen.series.as_ref().unwrap().target_novel_id, target);
    assert_eq!(frozen.attributes, template.attributes);
    assert_eq!(frozen.action_rules, template.action_rules);

    let immutable =
        sqlx::query("UPDATE user_world_series SET background = 'changed' WHERE id = $1")
            .bind(series.id)
            .execute(pool)
            .await
            .unwrap_err();
    assert_eq!(
        immutable.as_database_error().unwrap().code().as_deref(),
        Some("55000")
    );
    assert_eq!(
        repository.find(user, series.id).await.unwrap(),
        Some(series.clone())
    );
    let suggestion = handler.suggest(user, target).await.unwrap();
    assert!(matches!(
        suggestion.status,
        SeriesSuggestionStatus::Suggested
    ));
    assert_eq!(suggestion.suggestion.unwrap().series_id, Some(series.id));
    assert_eq!(matcher.0.load(Ordering::SeqCst), 1);
    assert!(handler.suggest(user, target).await.unwrap().cached);
    assert_eq!(matcher.0.load(Ordering::SeqCst), 1);
    assert_eq!(repository.list(user).await.unwrap().len(), 1);
    for (outcome, expected) in [(-1, "uncertain"), (99, "uncertain"), (-2, "unavailable")] {
        sqlx::query("UPDATE novels SET genre = $2 WHERE id = $1")
            .bind(target)
            .bind(format!("scenario {outcome}"))
            .execute(pool)
            .await
            .unwrap();
        matcher.1.store(outcome, Ordering::SeqCst);
        let advisory = handler.suggest(user, target).await.unwrap();
        assert_eq!(serde_json::to_value(&advisory.status).unwrap(), expected);
        assert!(advisory.suggestion.is_none());
        assert_eq!(
            repository.find_for_novel(user, target).await.unwrap(),
            Some(series.clone())
        );
        assert_eq!(repository.list(user).await.unwrap().len(), 1);
    }
    let unconfigured = WorldSeriesHandler {
        series_repo: repository.clone(),
        novel_repo: handler.novel_repo.clone(),
        canon_repo: canon.clone(),
        matcher: None,
        llm: None,
    };
    let advisory = unconfigured.suggest(user, target).await.unwrap();
    assert!(matches!(
        advisory.status,
        SeriesSuggestionStatus::Unconfigured
    ));
    assert!(advisory.suggestion.is_none());
    assert_eq!(matcher.0.load(Ordering::SeqCst), 4);

    handler.associate(user, target, None).await.unwrap();
    assert!(handler
        .frozen_rules(user, target, series.id, 1, 1, true)
        .await
        .is_err());
    assert_eq!(
        handler
            .frozen_rules(user, target, series.id, 1, 1, false)
            .await
            .unwrap(),
        frozen
    );
    assert_eq!(
        repository.find_for_novel(user, source).await.unwrap(),
        Some(series.clone())
    );

    // User-triggered paid-path semantics without making any provider request.
    let completion = Arc::new(CompletionSpy {
        calls: Arc::new(AtomicUsize::new(0)),
        identity: std::sync::Mutex::new("deepseek/test-a".into()),
        fails: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    let deepseek_handler = WorldSeriesHandler {
        series_repo: repository.clone(),
        novel_repo: handler.novel_repo.clone(),
        canon_repo: canon.clone(),
        matcher: Some(matcher.clone()),
        llm: Some(completion.clone()),
    };
    let decisions = || {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM series_match_decisions WHERE user_id = $1 AND novel_id = $2",
        )
        .bind(user)
        .bind(target)
        .fetch_one(pool)
    };
    let before_query = decisions().await.unwrap();
    assert!(matches!(
        deepseek_handler
            .check_deepseek(user, target)
            .await
            .unwrap()
            .reason,
        SeriesMatchReason::UnknownOutcome
    ));
    assert_eq!(decisions().await.unwrap(), before_query);
    assert_eq!(completion.calls.load(Ordering::SeqCst), 0);
    let (first, second) = tokio::join!(
        deepseek_handler.suggest_deepseek(user, target),
        deepseek_handler.suggest_deepseek(user, target)
    );
    let results = [first.unwrap(), second.unwrap()];
    assert!(results
        .iter()
        .any(|result| matches!(result.status, SeriesSuggestionStatus::Suggested)));
    assert_eq!(completion.calls.load(Ordering::SeqCst), 1);
    assert!(
        deepseek_handler
            .check_deepseek(user, target)
            .await
            .unwrap()
            .cached
    );
    assert!(
        deepseek_handler
            .suggest_deepseek(user, target)
            .await
            .unwrap()
            .cached
    );
    assert_eq!(completion.calls.load(Ordering::SeqCst), 1);
    *completion.identity.lock().unwrap() = "deepseek/test-b".into();
    let before_query = decisions().await.unwrap();
    assert!(matches!(
        deepseek_handler
            .check_deepseek(user, target)
            .await
            .unwrap()
            .reason,
        SeriesMatchReason::UnknownOutcome
    ));
    assert_eq!(decisions().await.unwrap(), before_query);
    assert_eq!(completion.calls.load(Ordering::SeqCst), 1);
    assert!(
        !deepseek_handler
            .suggest_deepseek(user, target)
            .await
            .unwrap()
            .cached
    );
    assert_eq!(completion.calls.load(Ordering::SeqCst), 2);
    sqlx::query("UPDATE novels SET genre = 'query-only changed evidence' WHERE id = $1")
        .bind(target)
        .execute(pool)
        .await
        .unwrap();
    let before_query = decisions().await.unwrap();
    assert!(matches!(
        deepseek_handler
            .check_deepseek(user, target)
            .await
            .unwrap()
            .reason,
        SeriesMatchReason::UnknownOutcome
    ));
    assert_eq!(decisions().await.unwrap(), before_query);
    assert_eq!(completion.calls.load(Ordering::SeqCst), 2);
    completion.fails.store(true, Ordering::SeqCst);
    *completion.identity.lock().unwrap() = "deepseek/test-unknown".into();
    assert!(matches!(
        deepseek_handler
            .suggest_deepseek(user, target)
            .await
            .unwrap()
            .reason,
        SeriesMatchReason::UnknownOutcome
    ));
    assert!(
        deepseek_handler
            .suggest_deepseek(user, target)
            .await
            .unwrap()
            .cached
    );
    assert_eq!(completion.calls.load(Ordering::SeqCst), 3);
    assert!(matches!(
        unconfigured
            .suggest_deepseek(user, target)
            .await
            .unwrap()
            .status,
        SeriesSuggestionStatus::Unconfigured
    ));
    assert!(deepseek_handler
        .suggest_deepseek(other_user, target)
        .await
        .is_err());
    assert_eq!(completion.calls.load(Ordering::SeqCst), 3);
    assert!(repository
        .find_for_novel(user, target)
        .await
        .unwrap()
        .is_none());
    assert_eq!(repository.list(user).await.unwrap().len(), 1);

    let claim_key = "a".repeat(64);
    let (first, second) = tokio::join!(
        repository.begin_match(user, target, SeriesMatchMethod::Deepseek, &claim_key, false),
        repository.begin_match(user, target, SeriesMatchMethod::Deepseek, &claim_key, false)
    );
    let claims = [first.unwrap(), second.unwrap()];
    let token = claims
        .iter()
        .find_map(|claim| match claim {
            BeginSeriesMatch::Acquired { token } => Some(*token),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        claims
            .iter()
            .filter(|claim| matches!(claim, BeginSeriesMatch::InProgress))
            .count(),
        1
    );
    let outcome = SeriesSuggestion {
        status: SeriesSuggestionStatus::Unavailable,
        suggestion: None,
        method: SeriesMatchMethod::Deepseek,
        reason: SeriesMatchReason::UnknownOutcome,
        cached: false,
    };
    assert!(!repository
        .complete_match(
            user,
            target,
            SeriesMatchMethod::Deepseek,
            &claim_key,
            Uuid::new_v4(),
            &outcome
        )
        .await
        .unwrap());
    sqlx::query("UPDATE series_match_decisions SET claimed_at = NOW() - INTERVAL '61 seconds' WHERE user_id = $1 AND novel_id = $2 AND evidence_key = $3")
        .bind(user).bind(target).bind(&claim_key).execute(pool).await.unwrap();
    assert!(matches!(
        repository
            .begin_match(user, target, SeriesMatchMethod::Deepseek, &claim_key, false)
            .await
            .unwrap(),
        BeginSeriesMatch::UnknownOutcome
    ));
    // A late response may finish only the original token; no retry acquires it.
    assert!(repository
        .complete_match(
            user,
            target,
            SeriesMatchMethod::Deepseek,
            &claim_key,
            token,
            &outcome
        )
        .await
        .unwrap());
    assert!(matches!(
        repository
            .begin_match(user, target, SeriesMatchMethod::Deepseek, &claim_key, false)
            .await
            .unwrap(),
        BeginSeriesMatch::Cached(_)
    ));
    assert!(matches!(
        repository
            .begin_match(user, target, SeriesMatchMethod::Laya, &claim_key, false)
            .await
            .unwrap(),
        BeginSeriesMatch::Acquired { .. }
    ));

    // Forged content cannot create a definition or partial source association.
    handler.associate(user, source, None).await.unwrap();
    let mut forged = WorldSeries {
        id: Uuid::new_v4(),
        name: "Forged source".into(),
        background: series.background.clone(),
        revision: 1,
        source_novel_id: source,
        source_template: Some(template.clone()),
        created_at: Utc::now(),
    };
    forged.source_template.as_mut().unwrap().action_rules[0].difficulty_class += 1;
    assert_eq!(
        repository.create(user, &forged).await.unwrap(),
        CreateWorldSeriesResult::SourceUnavailable
    );
    assert_eq!(repository.list(user).await.unwrap().len(), 1);
    assert!(repository
        .find_for_novel(user, source)
        .await
        .unwrap()
        .is_none());

    // Same source shelf lock serializes confirmation without overwriting.
    let (first, second) = tokio::join!(
        handler.create(user, command(source, "Race A")),
        handler.create(user, command(source, "Race B"))
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let loser = if first.is_ok() { second } else { first };
    assert!(matches!(
        loser,
        Err(WorldSeriesApplicationError::SourceAlreadyAssociated)
    ));
    assert_eq!(repository.list(user).await.unwrap().len(), 2);
    let after = sqlx::query_scalar::<_, serde_json::Value>("SELECT jsonb_agg(to_jsonb(t) ORDER BY prompt_version) FROM novel_game_rule_templates t WHERE novel_id = $1")
        .bind(source).fetch_one(pool).await.unwrap();
    assert_eq!(after, baseline);
    assert!(canon
        .begin_game_rule_generation(source, 1, "series-game-rules-v1")
        .await
        .is_err());

    let exported = PgAccountExport::new(pool.clone())
        .export_user(user)
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert!(exported
        .iter()
        .any(|row| row.kind == "series_match_decision"));
    assert!(!serde_json::to_string(
        &exported
            .iter()
            .filter(|row| row.kind == "series_match_decision")
            .map(|row| &row.data)
            .collect::<Vec<_>>()
    )
    .unwrap()
    .contains("synthetic unknown provider outcome"));
    assert_eq!(
        exported
            .iter()
            .filter(|row| row.kind == "world_series")
            .count(),
        2
    );
    assert_eq!(
        exported
            .iter()
            .filter(|row| row.kind == "novel_world_series")
            .count(),
        1
    );
    assert!(!PgAccountExport::new(pool.clone())
        .export_user(other_user)
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .iter()
        .any(|row| row.kind == "world_series"));

    // Removing the source shelf and Canon never destroys safe frozen snapshots.
    handler
        .associate(user, extra_books[0], Some(series.id))
        .await
        .unwrap();
    sqlx::query("DELETE FROM user_novels WHERE user_id = $1 AND novel_id = $2")
        .bind(user)
        .bind(source)
        .execute(pool)
        .await
        .unwrap();
    assert!(repository
        .find_for_novel(user, source)
        .await
        .unwrap()
        .is_none());
    matcher.1.store(0, Ordering::SeqCst);
    let advisory = handler.suggest(user, target).await.unwrap();
    let representative = advisory.suggestion.unwrap();
    assert_eq!(representative.series_id, Some(series.id));
    assert_eq!(representative.source_novel_id, extra_books[0]);
    assert_eq!(
        handler
            .frozen_rules(user, target, series.id, 1, 1, false)
            .await
            .unwrap(),
        frozen
    );
    sqlx::query("DELETE FROM novels WHERE id = $1")
        .bind(source)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        repository.find(user, series.id).await.unwrap(),
        Some(series)
    );
    assert_eq!(
        handler
            .frozen_rules(
                user,
                target,
                frozen.series.as_ref().unwrap().binding.series_id,
                1,
                1,
                false
            )
            .await
            .unwrap(),
        frozen
    );
    // Filenames are arbitrary. Two names first attested in chapter 2 are
    // private ranking evidence, never Laya/DeepSeek input or a public spoiler.
    for book in extra_books.iter().filter(|book| **book != extra_books[1]) {
        sqlx::query("DELETE FROM user_novels WHERE user_id = $1 AND novel_id = $2")
            .bind(user)
            .bind(*book)
            .execute(pool)
            .await
            .unwrap();
    }
    let mut candidate_model = None;
    for (book, title) in [
        (target, "upload-93f1-random.txt"),
        (extra_books[1], "another-unrelated-filename.epub"),
    ] {
        sqlx::query("UPDATE novels SET title = $2, total_chapters = 2 WHERE id = $1")
            .bind(book)
            .bind(title)
            .execute(pool)
            .await
            .unwrap();
        for (number, text) in [
            (1, "A hero begins the journey."),
            (2, "The hero reaches Hidden Tower and Moon Guild."),
        ] {
            sqlx::query(
                "INSERT INTO chapters (novel_id, chapter_number, content) VALUES ($1, $2, $3)",
            )
            .bind(book)
            .bind(number)
            .bind(text)
            .execute(pool)
            .await
            .unwrap();
        }
        let character = Uuid::new_v4();
        sqlx::query("INSERT INTO characters (id, novel_id, name) VALUES ($1, $2, 'Hero')")
            .bind(character)
            .bind(book)
            .execute(pool)
            .await
            .unwrap();
        let chapter_one = SourceEvidence {
            confidence: 1.0,
            provenance: vec![SourceCitation {
                chapter_number: 1,
                excerpt: "A hero begins the journey.".into(),
            }],
        };
        let later = SourceEvidence {
            confidence: 1.0,
            provenance: vec![SourceCitation {
                chapter_number: 2,
                excerpt: "The hero reaches Hidden Tower and Moon Guild.".into(),
            }],
        };
        let model = CanonStoryModel {
            id: Uuid::new_v4(),
            novel_id: book,
            model_version: 1,
            schema_version: 1,
            prompt_version: "test-series-evidence-v1".into(),
            content: CanonStoryContent {
                arcs: vec![StoryArc {
                    id: "arc".into(),
                    title: "Journey".into(),
                    summary: "A hero begins the journey.".into(),
                    event_ids: vec!["event".into()],
                    evidence: chapter_one.clone(),
                }],
                events: vec![CanonEvent {
                    id: "event".into(),
                    sequence: 1,
                    summary: "A hero begins the journey.".into(),
                    caused_by: vec![],
                    location_ids: vec![],
                    character_ids: vec![character],
                    faction_ids: vec![],
                    evidence: chapter_one.clone(),
                }],
                locations: ["Hidden Tower", "Moon Guild"]
                    .into_iter()
                    .map(|name| CanonLocation {
                        id: name.into(),
                        name: name.into(),
                        description: "A named place in the source.".into(),
                        evidence: later.clone(),
                    })
                    .collect(),
                factions: vec![],
                world_rules: vec![],
                character_goals: vec![],
                relationships: vec![],
                deaths: vec![],
                unresolved_threads: vec![],
                ending: CanonEndingSnapshot {
                    summary: "The journey reaches its ending.".into(),
                    character_states: Default::default(),
                    faction_states: Default::default(),
                    location_states: Default::default(),
                    unresolved_thread_ids: vec![],
                    evidence: later,
                },
            },
            created_at: Utc::now(),
        };
        model
            .validate(
                &std::collections::BTreeMap::from([
                    (1, "A hero begins the journey.".into()),
                    (2, "The hero reaches Hidden Tower and Moon Guild.".into()),
                ]),
                &std::collections::HashSet::from([character]),
            )
            .unwrap();
        sqlx::query("INSERT INTO canon_story_models (id, novel_id, model_version, schema_version, prompt_version, content, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(model.id).bind(model.novel_id).bind(model.model_version)
            .bind(model.schema_version).bind(&model.prompt_version)
            .bind(serde_json::to_value(&model.content).unwrap())
            .bind(model.created_at).execute(pool).await.unwrap();
        if book == extra_books[1] {
            candidate_model = Some((model, character));
        }
    }
    matcher.1.store(-1, Ordering::SeqCst);
    let advisory = handler.suggest(user, target).await.unwrap();
    assert!(matches!(advisory.status, SeriesSuggestionStatus::Suggested));
    assert!(matches!(advisory.reason, SeriesMatchReason::LocalEvidence));
    assert!(!serde_json::to_string(&advisory)
        .unwrap()
        .contains("Hidden Tower"));
    assert_eq!(advisory.suggestion.unwrap().source_novel_id, extra_books[1]);
    let before_check = decisions().await.unwrap();
    let before_calls = completion.calls.load(Ordering::SeqCst);
    assert!(matches!(
        deepseek_handler
            .check_deepseek(user, target)
            .await
            .unwrap()
            .reason,
        SeriesMatchReason::UnknownOutcome
    ));
    assert_eq!(decisions().await.unwrap(), before_check);
    assert_eq!(completion.calls.load(Ordering::SeqCst), before_calls);
    completion.fails.store(false, Ordering::SeqCst);
    *completion.identity.lock().unwrap() = "deepseek/test-private".into();
    assert!(matches!(
        deepseek_handler
            .suggest_deepseek(user, target)
            .await
            .unwrap()
            .status,
        SeriesSuggestionStatus::Suggested
    ));
    assert_eq!(completion.calls.load(Ordering::SeqCst), before_calls + 1);
    assert!(repository
        .find_for_novel(user, target)
        .await
        .unwrap()
        .is_none());
    assert!(handler.suggest(user, target).await.unwrap().cached);
    let (mut changed, character) = candidate_model.unwrap();
    changed.id = Uuid::new_v4();
    changed.model_version = 2;
    changed.content.locations.clear();
    changed
        .validate(
            &std::collections::BTreeMap::from([
                (1, "A hero begins the journey.".into()),
                (2, "The hero reaches Hidden Tower and Moon Guild.".into()),
            ]),
            &std::collections::HashSet::from([character]),
        )
        .unwrap();
    sqlx::query("INSERT INTO canon_story_models (id, novel_id, model_version, schema_version, prompt_version, content, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(changed.id).bind(changed.novel_id).bind(changed.model_version)
        .bind(changed.schema_version).bind(&changed.prompt_version)
        .bind(serde_json::to_value(&changed.content).unwrap())
        .bind(changed.created_at).execute(pool).await.unwrap();
    let changed_advisory = handler.suggest(user, target).await.unwrap();
    assert!(!changed_advisory.cached);
    assert!(matches!(
        changed_advisory.status,
        SeriesSuggestionStatus::Uncertain
    ));
    assert!(matches!(
        changed_advisory.reason,
        SeriesMatchReason::LowConfidence
    ));
    let before_check = decisions().await.unwrap();
    let before_calls = completion.calls.load(Ordering::SeqCst);
    let paid_check = deepseek_handler.check_deepseek(user, target).await.unwrap();
    assert!(paid_check.cached);
    assert!(matches!(
        paid_check.status,
        SeriesSuggestionStatus::Suggested
    ));
    assert_eq!(decisions().await.unwrap(), before_check);
    assert_eq!(completion.calls.load(Ordering::SeqCst), before_calls);
    let before_overflow = matcher.0.load(Ordering::SeqCst);
    sqlx::query("WITH added AS (INSERT INTO novels (id, user_id, title, total_chapters, status) SELECT public.uuid_generate_v4(), $1, 'arbitrary-' || number, 1, 'ready'::novel_status FROM generate_series(1, 129) AS number RETURNING id) INSERT INTO user_novels (user_id, novel_id) SELECT $1, id FROM added")
        .bind(user).execute(pool).await.unwrap();
    let overflow = handler.suggest(user, target).await.unwrap();
    assert!(matches!(overflow.status, SeriesSuggestionStatus::Uncertain));
    assert!(matches!(overflow.reason, SeriesMatchReason::TooManyBooks));
    assert_eq!(matcher.0.load(Ordering::SeqCst), before_overflow);
    // The caller deletes fixture users to verify account cascades and cleanup.
    sqlx::query("DELETE FROM novels WHERE id = $1")
        .bind(target)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM novels WHERE id = ANY($1)")
        .bind(&extra_books)
        .execute(pool)
        .await
        .unwrap();
}
