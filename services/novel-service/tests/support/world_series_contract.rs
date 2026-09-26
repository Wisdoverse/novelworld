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
            game_rule_template::{
                basic_attribute, GameActionKind, GameActionRule, GameAttribute, GameRuleTemplate,
                BASIC_ACTION_DESCRIPTION,
            },
            world_series::WorldSeries,
        },
        ports::{
            series_matcher::{SeriesBookMetadata, SeriesMatchCandidate, SeriesMatcherPort},
            AccountExportPort,
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
#[async_trait::async_trait]
impl SeriesMatcherPort for MatcherSpy {
    async fn suggest(
        &self,
        _: &SeriesBookMetadata,
        candidates: &[SeriesMatchCandidate],
    ) -> anyhow::Result<Option<usize>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        assert_eq!(candidates.len(), 8);
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
    };

    // Missing source rules do not create a series, member or generation claim.
    assert!(matches!(
        handler.create(user, command(source, "Missing rules")).await,
        Err(WorldSeriesApplicationError::SourceUnavailable)
    ));
    let template = source_template(source);
    sqlx::query("INSERT INTO novel_game_rule_templates (novel_id,canon_model_version,schema_version,prompt_version,status,attempt,content,completed_at) VALUES ($1,1,1,'novel-game-rules-v2','ready',1,$2,NOW())")
        .bind(source).bind(serde_json::to_value(&template).unwrap()).execute(pool).await.unwrap();
    let baseline = sqlx::query_scalar::<_, serde_json::Value>("SELECT jsonb_agg(to_jsonb(t) ORDER BY prompt_version) FROM novel_game_rule_templates t WHERE novel_id = $1")
        .bind(source).fetch_one(pool).await.unwrap();
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
    assert!(repository.list(user).await.unwrap().is_empty());
    let mut series = handler
        .create(user, command(source, "Confirmed series"))
        .await
        .unwrap();
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
    assert_eq!(series.source_template, template);
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
    assert_eq!(repository.list(user).await.unwrap().len(), 1);
    for (outcome, expected) in [(-1, "uncertain"), (99, "uncertain"), (-2, "unavailable")] {
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

    // Forged content cannot create a definition or partial source association.
    handler.associate(user, source, None).await.unwrap();
    let mut forged = WorldSeries {
        id: Uuid::new_v4(),
        name: "Forged source".into(),
        background: series.background.clone(),
        revision: 1,
        source_template: template.clone(),
        created_at: Utc::now(),
    };
    forged.source_template.action_rules[0].difficulty_class += 1;
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
