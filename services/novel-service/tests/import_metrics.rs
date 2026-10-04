//! Current import-state observations over disposable PostgreSQL and actual HTTP.
use novel_service::{
    domain::entities::{chapter::Chapter, novel::Novel},
    interface::http::router,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{str::FromStr, time::Duration};
use uuid::Uuid;

#[path = "support/http_state.rs"]
mod http_state;

async fn scrape(client: &reqwest::Client, url: &str) -> String {
    let response = client.get(url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; version=0.0.4"
    );
    let text = response.text().await.unwrap();
    assert!(text.contains("novelworld_llm_observability_info"));
    text
}

fn observed(text: &str, counts: [i64; 4]) {
    assert!(text
        .lines()
        .any(|line| line == "novelworld_import_jobs_observation_success 1"));
    let samples = text
        .lines()
        .filter(|line| line.starts_with("novelworld_import_jobs{"))
        .collect::<Vec<_>>();
    assert_eq!(samples.len(), 4);
    for (status, count) in ["pending", "in_progress", "failed", "completed"]
        .into_iter()
        .zip(counts)
    {
        assert!(samples
            .contains(&format!("novelworld_import_jobs{{status=\"{status}\"}} {count}").as_str()));
    }
}

fn unknown(text: &str) {
    assert!(text
        .lines()
        .any(|line| line == "novelworld_import_jobs_observation_success 0"));
    assert!(!text.contains("novelworld_import_jobs{"));
}

async fn next_observation() {
    tokio::time::sleep(Duration::from_millis(5050)).await;
}

async fn wait_for_locked_observation(pool: &sqlx::PgPool, database: &str) {
    tokio::time::timeout(Duration::from_millis(900), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = $1 \
                 AND wait_event_type = 'Lock' AND query LIKE '%COUNT(*) FILTER%')",
            )
            .bind(database)
            .fetch_one(pool)
            .await
            .unwrap();
            if blocked {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("observation must reach the actual PostgreSQL lock");
}

#[tokio::test]
#[ignore = "requires explicitly disposable TEST_DATABASE_URL; run with --ignored"]
async fn import_metrics_observe_retained_states_and_fail_boundedly_without_stale_data() {
    let url = std::env::var("TEST_DATABASE_URL").expect("provide explicitly disposable PostgreSQL");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let database = format!("novelworld_import_metrics_{}", Uuid::new_v4().simple());
    // The identifier is generated here, never supplied by a caller. Do not replace another database.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE DATABASE \"{database}\""
    )))
    .execute(&admin)
    .await
    .unwrap();
    let options = PgConnectOptions::from_str(&url)
        .unwrap()
        .database(&database);
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options.clone())
        .await
        .unwrap();
    let control = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../../../infra/postgres/init.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let state = http_state::state_with_pool(pool.clone());
    let repo = state.novel_repo.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/metrics", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap();

    observed(&scrape(&client, &endpoint).await, [0, 0, 0, 0]);
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id,email,password_hash) VALUES ($1,$2,'synthetic')")
        .bind(user)
        .bind(format!("import-metrics-{user}@test.invalid"))
        .execute(&pool)
        .await
        .unwrap();
    let novels = (0..4)
        .map(|_| Novel::create(user, "PRIVATE IMPORT TITLE".into(), None))
        .collect::<Vec<_>>();
    for novel in &novels {
        repo.create_import(
            novel,
            &[Chapter::new(
                novel.id,
                1,
                None,
                "PRIVATE SOURCE CONTENT".repeat(4),
            )],
        )
        .await
        .unwrap();
    }
    repo.claim_import(novels[1].id, user)
        .await
        .unwrap()
        .unwrap();
    let failed = repo
        .claim_import(novels[2].id, user)
        .await
        .unwrap()
        .unwrap();
    assert!(repo
        .fail_import(
            failed.novel_id,
            failed.attempt,
            "PRIVATE_FAILURE",
            "PRIVATE FAILURE TEXT"
        )
        .await
        .unwrap());
    // A completed-job fixture does not qualify readiness; these gauges count job states alone.
    sqlx::query(
        "UPDATE novel_import_jobs SET status='completed', stage='completed' WHERE novel_id=$1",
    )
    .bind(novels[3].id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE novels SET status='ready'::novel_status, original_file_key='source-files/PRIVATE_KEY' WHERE id=$1")
        .bind(novels[3].id).execute(&pool).await.unwrap();
    next_observation().await;
    let before: serde_json::Value = sqlx::query_scalar(
        "SELECT jsonb_agg(to_jsonb(job) ORDER BY novel_id) FROM novel_import_jobs AS job",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let text = scrape(&client, &endpoint).await;
    observed(&text, [1, 1, 1, 1]);
    for private in [
        user.to_string(),
        "PRIVATE IMPORT TITLE".into(),
        "PRIVATE SOURCE CONTENT".into(),
        "PRIVATE_FAILURE".into(),
        "PRIVATE FAILURE TEXT".into(),
        "PRIVATE_KEY".into(),
    ]
    .into_iter()
    .chain(novels.iter().map(|n| n.id.to_string()))
    {
        assert!(!text.contains(&private));
    }
    let after: serde_json::Value = sqlx::query_scalar(
        "SELECT jsonb_agg(to_jsonb(job) ORDER BY novel_id) FROM novel_import_jobs AS job",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        before, after,
        "observing must not change jobs or timestamps"
    );
    for _ in 0..3 {
        unknown(&scrape(&client, &endpoint).await);
    }

    for attempt in 2..=3 {
        assert_eq!(
            repo.claim_import(novels[2].id, user)
                .await
                .unwrap()
                .unwrap()
                .attempt,
            attempt
        );
        assert!(repo
            .fail_import(
                novels[2].id,
                attempt,
                "PRIVATE_FAILURE",
                "PRIVATE FAILURE TEXT"
            )
            .await
            .unwrap());
    }
    assert!(repo
        .claim_import(novels[2].id, user)
        .await
        .unwrap()
        .is_none());
    let code: String =
        sqlx::query_scalar("SELECT failure_code FROM novel_import_jobs WHERE novel_id=$1")
            .bind(novels[2].id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(code, "budget_exhausted");
    sqlx::query(
        "UPDATE novel_import_jobs SET lease_expires_at=NOW()-INTERVAL '1 second' WHERE novel_id=$1",
    )
    .bind(novels[1].id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        repo.claim_import(novels[1].id, user)
            .await
            .unwrap()
            .unwrap()
            .attempt,
        2
    );
    next_observation().await;
    observed(&scrape(&client, &endpoint).await, [1, 1, 1, 1]);

    next_observation().await;
    let mut lock = control.begin().await.unwrap();
    sqlx::query("LOCK TABLE novel_import_jobs IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let first_client = client.clone();
    let first_endpoint = endpoint.clone();
    let started = std::time::Instant::now();
    let first = tokio::spawn(async move { scrape(&first_client, &first_endpoint).await });
    wait_for_locked_observation(&control, &database).await;
    unknown(&scrape(&client, &endpoint).await);
    unknown(&first.await.unwrap());
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "server-side lock/scan timeout must bound the response"
    );
    lock.rollback().await.unwrap();

    next_observation().await;
    let mut lock = control.begin().await.unwrap();
    sqlx::query("LOCK TABLE novel_import_jobs IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let cancelled_repo = repo.clone();
    let cancelled = tokio::spawn(async move { cancelled_repo.observe_import_jobs().await });
    wait_for_locked_observation(&control, &database).await;
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_millis(750), async {
        for _ in 0..3 {
            unknown(&scrape(&client, &endpoint).await);
        }
    })
    .await
    .expect("cancelled work must retain admission: a burst cannot wait on more database scans");
    lock.rollback().await.unwrap();
    next_observation().await;
    observed(&scrape(&client, &endpoint).await, [1, 1, 1, 1]);
    let statement_timeout: String = sqlx::query_scalar("SHOW statement_timeout")
        .fetch_one(&pool)
        .await
        .unwrap();
    let read_only: String = sqlx::query_scalar("SHOW transaction_read_only")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        statement_timeout, "0",
        "transaction-local limits must not leak to business work"
    );
    assert_eq!(read_only, "off");

    next_observation().await;
    let held = pool.acquire().await.unwrap();
    let started = std::time::Instant::now();
    unknown(&scrape(&client, &endpoint).await);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "pool acquisition must use the outer deadline"
    );
    drop(held);
    next_observation().await;
    sqlx::query("DELETE FROM novels WHERE user_id=$1")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    observed(&scrape(&client, &endpoint).await, [0, 0, 0, 0]);
    next_observation().await;
    pool.close().await;
    unknown(&scrape(&client, &endpoint).await);

    drop(client);
    server.abort();
    let _ = server.await;
    control.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE \"{database}\"")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
