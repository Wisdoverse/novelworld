//! Live Novel-owned community-consent and privacy contracts; no provider calls.
use futures::TryStreamExt;
use novel_service::{
    application::world_series::{
        CreateWorldSeries, WorldSeriesApplicationError, WorldSeriesHandler,
    },
    domain::{
        ports::{
            series_matcher::{SeriesMatchMethod, SeriesMatchReason, SeriesSuggestionStatus},
            AccountExportPort,
        },
        repositories::WorldSeriesRepository,
    },
    infrastructure::persistence::{
        account_export::PgAccountExport, canon_story_model_pg_repo::PgCanonStoryModelRepository,
        character_pg_repo::CharacterPgRepository, novel_pg_repo::NovelPgRepository,
        world_series_pg_repo::PgWorldSeriesRepository,
    },
};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

pub async fn run(pool: &PgPool, recipient: Uuid, donors: &[Uuid], source: Uuid) -> Uuid {
    assert_eq!(donors.len(), 4);
    sqlx::query("INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2)")
        .bind(recipient)
        .bind(source)
        .execute(pool)
        .await
        .unwrap();
    let target = Uuid::new_v4();
    let extra = Uuid::new_v4();
    for (id, title) in [(target, "Recipient target"), (extra, "Recipient sequel")] {
        sqlx::query("INSERT INTO novels (id, user_id, title, total_chapters, status) VALUES ($1, $2, $3, 1, 'ready'::novel_status)")
            .bind(id).bind(recipient).bind(title).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2)")
            .bind(recipient)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }
    let repo = Arc::new(PgWorldSeriesRepository::new(pool.clone()));
    let handler = WorldSeriesHandler {
        series_repo: repo.clone(),
        novel_repo: Arc::new(NovelPgRepository::new(pool.clone())),
        canon_repo: Arc::new(PgCanonStoryModelRepository::new(pool.clone())),
        character_repo: Arc::new(CharacterPgRepository::new(pool.clone())),
        matcher: None,
        llm: None,
    };
    let mut donor_series = Vec::new();
    for donor in donors {
        for id in [source, target, extra] {
            sqlx::query("INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2)")
                .bind(donor)
                .bind(id)
                .execute(pool)
                .await
                .unwrap();
        }
        let series = handler
            .create(
                *donor,
                CreateWorldSeries {
                    name: "PRIVATE DONOR NAME".into(),
                    background: Some("PRIVATE DONOR BACKGROUND".into()),
                    source_novel_id: source,
                    canon_model_version: None,
                },
            )
            .await
            .unwrap();
        assert!(!handler.contribution(*donor, series.id).await.unwrap());
        assert!(matches!(
            handler.contribution(recipient, series.id).await,
            Err(WorldSeriesApplicationError::NotFound)
        ));
        assert!(matches!(
            handler.set_contribution(recipient, series.id, true).await,
            Err(WorldSeriesApplicationError::NotFound)
        ));
        assert!(repo
            .associate(*donor, target, Some(series.id))
            .await
            .unwrap());
        donor_series.push(series.id);
    }
    let uncertain = || SeriesSuggestionStatus::Uncertain;
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        uncertain()
    );
    for index in 0..2 {
        handler
            .set_contribution(donors[index], donor_series[index], true)
            .await
            .unwrap();
        // Repeated opt-ins cannot fabricate independent support.
        handler
            .set_contribution(donors[index], donor_series[index], true)
            .await
            .unwrap();
    }
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        uncertain()
    );
    handler
        .set_contribution(donors[2], donor_series[2], true)
        .await
        .unwrap();
    let result = handler.suggest_community(recipient, target).await.unwrap();
    assert_eq!(result.status, SeriesSuggestionStatus::Suggested);
    assert_eq!(result.method, SeriesMatchMethod::Community);
    assert_eq!(result.reason, SeriesMatchReason::CommunityConsensus);
    assert!(!result.cached);
    assert_eq!(result.suggestion.as_ref().unwrap().source_novel_id, source);
    assert_eq!(result.suggestion.as_ref().unwrap().series_id, None);
    let serialized = serde_json::to_string(&result).unwrap();
    for private in donor_series.iter().chain(donors.iter()) {
        assert!(!serialized.contains(&private.to_string()));
    }
    assert!(!serialized.contains("PRIVATE DONOR"));
    assert!(!serialized.contains("background"));
    assert!(!serialized.contains("count"));
    assert!(matches!(
        handler.suggest_community(Uuid::new_v4(), target).await,
        Err(WorldSeriesApplicationError::NotFound)
    ));

    let exporter = PgAccountExport::new(pool.clone());
    let exported = exporter
        .export_user(donors[2])
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert!(exported
        .iter()
        .any(|r| r.kind == "world_series_contribution" && r.data["enabled"] == true));
    handler
        .set_contribution(donors[2], donor_series[2], false)
        .await
        .unwrap();
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        uncertain()
    );
    assert!(!exporter
        .export_user(donors[2])
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .iter()
        .any(|r| r.kind == "world_series_contribution"));
    handler
        .set_contribution(donors[2], donor_series[2], true)
        .await
        .unwrap();

    // Detachment and shelf removal change live evidence without a cache flush.
    assert!(repo.associate(donors[2], target, None).await.unwrap());
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        uncertain()
    );
    repo.associate(donors[2], target, Some(donor_series[2]))
        .await
        .unwrap();
    sqlx::query("DELETE FROM user_novels WHERE user_id = $1 AND novel_id = $2")
        .bind(donors[2])
        .bind(target)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        uncertain()
    );
    sqlx::query("INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2)")
        .bind(donors[2])
        .bind(target)
        .execute(pool)
        .await
        .unwrap();
    repo.associate(donors[2], target, Some(donor_series[2]))
        .await
        .unwrap();

    // One opted-in explicit split vetoes consensus. Opt-out splits are ignored.
    repo.associate(donors[3], target, None).await.unwrap();
    let split = Uuid::new_v4();
    sqlx::query("INSERT INTO user_world_series (id, user_id, name, revision, source_novel_id, created_at) VALUES ($1, $2, 'PRIVATE SPLIT', 1, $3, NOW())")
        .bind(split).bind(donors[3]).bind(target).execute(pool).await.unwrap();
    repo.associate(donors[3], target, Some(split))
        .await
        .unwrap();
    handler
        .set_contribution(donors[3], donor_series[3], true)
        .await
        .unwrap();
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        SeriesSuggestionStatus::Suggested
    );
    handler
        .set_contribution(donors[3], split, true)
        .await
        .unwrap();
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        uncertain()
    );
    handler
        .set_contribution(donors[3], split, false)
        .await
        .unwrap();

    // Consent authorization and series deletion serialize on the owning row.
    let mut deletion = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM user_world_series WHERE id = $1 FOR UPDATE")
        .bind(split)
        .execute(&mut *deletion)
        .await
        .unwrap();
    let racing_repo = repo.clone();
    let split_owner = donors[3];
    let enable =
        tokio::spawn(async move { racing_repo.set_contribution(split_owner, split, true).await });
    sqlx::query("DELETE FROM user_world_series WHERE id = $1")
        .bind(split)
        .execute(&mut *deletion)
        .await
        .unwrap();
    deletion.commit().await.unwrap();
    assert!(!enable.await.unwrap().unwrap());
    assert_eq!(repo.contribution(split_owner, split).await.unwrap(), None);

    // The Ready shelf ceiling is checked before choosing a partial candidate set.
    let mut overflow = Vec::new();
    for index in 0..127 {
        let id = Uuid::new_v4();
        // Identical titles never create cross-upload canonical identity.
        sqlx::query("INSERT INTO novels (id, user_id, title, total_chapters, status) SELECT $1, $2, title, 1, 'ready'::novel_status FROM novels WHERE id = $3")
            .bind(id).bind(recipient).bind(source).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2)")
            .bind(recipient)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        overflow.push(id);
        if index == 0 {
            let exact = handler.suggest_community(recipient, target).await.unwrap();
            assert_eq!(exact.suggestion.unwrap().source_novel_id, source);
        }
    }
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        uncertain()
    );
    sqlx::query("DELETE FROM novels WHERE id = ANY($1)")
        .bind(&overflow)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        SeriesSuggestionStatus::Suggested
    );

    // Existing local series requires direct support for every current member.
    let own = handler
        .create(
            recipient,
            CreateWorldSeries {
                name: "MY SERIES".into(),
                background: None,
                source_novel_id: source,
                canon_model_version: None,
            },
        )
        .await
        .unwrap();
    repo.associate(recipient, extra, Some(own.id))
        .await
        .unwrap();
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        uncertain()
    );
    for index in 0..3 {
        repo.associate(donors[index], extra, Some(donor_series[index]))
            .await
            .unwrap();
    }
    let grouped = handler.suggest_community(recipient, target).await.unwrap();
    assert_eq!(grouped.suggestion.unwrap().series_id, Some(own.id));
    repo.associate(recipient, extra, None).await.unwrap();
    // Two separate local candidates are ambiguous, regardless of ordering.
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        uncertain()
    );
    repo.associate(recipient, extra, Some(own.id))
        .await
        .unwrap();
    repo.associate(recipient, target, Some(own.id))
        .await
        .unwrap();
    handler
        .set_contribution(recipient, own.id, true)
        .await
        .unwrap();
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        uncertain()
    );
    repo.associate(recipient, target, None).await.unwrap();
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        SeriesSuggestionStatus::Suggested
    );
    sqlx::query("UPDATE novels SET status = 'parsing'::novel_status WHERE id = $1")
        .bind(extra)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        uncertain()
    );
    sqlx::query("UPDATE novels SET status = 'ready'::novel_status WHERE id = $1")
        .bind(extra)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        handler
            .suggest_community(recipient, target)
            .await
            .unwrap()
            .status,
        SeriesSuggestionStatus::Suggested
    );
    target
}
