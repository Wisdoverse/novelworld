use chrono::{Duration, Utc};
use futures::future::join_all;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::sync::Arc;
use user_service::{
    domain::{
        entities::diagnostic_budget::{
            Amount, Attempt, BudgetError, Profile, Registration, Settlement,
        },
        repositories::diagnostic_budget::DiagnosticBudgetRepository,
    },
    infrastructure::persistence::pg_diagnostic_budget::PgDiagnosticBudgetRepository,
};
use uuid::Uuid;

async fn pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(16)
        .connect(
            &std::env::var("TEST_DATABASE_URL")
                .unwrap_or_else(|_| "postgres://test:test@localhost:25432/novelworld_test".into()),
        )
        .await
        .expect("dedicated migrated integration database")
}

fn registration() -> Registration {
    let profile = Profile::compiled();
    Registration {
        budget_id: Uuid::new_v4(),
        contract: profile.contract,
        profile: profile.profile,
        profile_sha256: user_service::infrastructure::diagnostic_budget::profile_sha256(),
        limits: profile.max_limits,
        expires_at: chrono::DateTime::from_timestamp(
            (Utc::now() + Duration::hours(1)).timestamp(),
            0,
        )
        .unwrap(),
    }
}

fn attempt() -> Attempt {
    Attempt {
        attempt_id: Uuid::new_v4(),
        operation: "setup_connection".into(),
        output_limit: 8,
    }
}

fn usage() -> Settlement {
    Settlement {
        model: Profile::compiled().model,
        input_tokens: 10,
        output_tokens: 2,
        cached_input_tokens: None,
    }
}

#[derive(sqlx::FromRow)]
struct ReceiptTotals {
    count: i64,
    ordinals: i64,
}

#[tokio::test]
async fn parallel_reservations_are_atomic_and_receipts_are_bounded() {
    let pool = pool().await;
    let repo = PgDiagnosticBudgetRepository::new(pool.clone());
    let mut registration = registration();
    let quote = attempt().quote().unwrap();
    registration.limits = Amount {
        attempts: 4,
        tokens: quote.tokens * 4,
        cost_micro_cny: quote.cost_micro_cny * 4,
    };
    repo.provision(&registration).await.unwrap();
    let attempts: Vec<_> = (0..32).map(|_| attempt()).collect();
    let results = join_all(
        attempts
            .iter()
            .map(|attempt| repo.reserve(&registration, attempt)),
    )
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 4);
    assert!(results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .all(|error| *error == BudgetError::Exhausted));
    assert_eq!(
        repo.read(&registration).await.unwrap().charged,
        registration.limits
    );
    let totals = sqlx::query_as::<_, ReceiptTotals>(
        "SELECT count(*) AS count, count(DISTINCT ordinal) AS ordinals FROM diagnostic_llm_attempts WHERE budget_id = $1",
    ).bind(registration.budget_id).fetch_one(&pool).await.unwrap();
    assert_eq!((totals.count, totals.ordinals), (4, 4));
    let winner = attempts
        .iter()
        .zip(results)
        .find(|(_, result)| result.is_ok())
        .unwrap()
        .0;
    assert_eq!(
        repo.reserve(&registration, winner).await,
        Err(BudgetError::Conflict)
    );
}

#[tokio::test]
async fn settlement_refunds_once_and_compares_the_entire_usage_tuple() {
    let repo = PgDiagnosticBudgetRepository::new(pool().await);
    let mut registration = registration();
    let attempt = attempt();
    let quote = attempt.quote().unwrap();
    registration.limits = Amount {
        attempts: 2,
        tokens: quote.tokens + 12,
        cost_micro_cny: quote.cost_micro_cny + 64,
    };
    repo.provision(&registration).await.unwrap();
    let duplicates = join_all((0..8).map(|_| repo.reserve(&registration, &attempt))).await;
    assert_eq!(duplicates.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(duplicates
        .iter()
        .filter_map(|result| result.as_ref().err())
        .all(|error| *error == BudgetError::Conflict));
    let second = Attempt {
        attempt_id: Uuid::new_v4(),
        ..attempt.clone()
    };
    assert_eq!(
        repo.reserve(&registration, &second).await,
        Err(BudgetError::Exhausted)
    );
    let usage = usage();
    for result in
        join_all((0..16).map(|_| repo.settle(&registration, attempt.attempt_id, &usage))).await
    {
        result.unwrap();
    }
    let actual = Profile::compiled()
        .usage("setup_connection", 8, &usage)
        .unwrap();
    assert_eq!(repo.read(&registration).await.unwrap().charged, actual);
    // Same computed price, different full tuple: None is not Some(0).
    let different = Settlement {
        cached_input_tokens: Some(0),
        ..usage.clone()
    };
    assert_eq!(
        repo.settle(&registration, attempt.attempt_id, &different)
            .await,
        Err(BudgetError::Conflict)
    );
    assert_eq!(
        repo.reserve(&registration, &attempt).await,
        Err(BudgetError::Conflict)
    );
    // Only a proven refund releases room for another attempt; attempt count itself never refunds.
    repo.reserve(&registration, &second).await.unwrap();
    let third = Attempt {
        attempt_id: Uuid::new_v4(),
        ..attempt.clone()
    };
    assert_eq!(
        repo.reserve(&registration, &third).await,
        Err(BudgetError::Exhausted)
    );
    repo.settle(&registration, second.attempt_id, &usage)
        .await
        .unwrap();
    assert_eq!(
        repo.read(&registration).await.unwrap().charged,
        actual.checked_add(actual).unwrap()
    );
    assert_eq!(
        repo.reserve(&registration, &third).await,
        Err(BudgetError::Exhausted)
    );
}

#[tokio::test]
async fn seal_expiry_and_invalid_usage_never_refill_or_block_valid_late_settlement() {
    let pool = pool().await;
    let repo = PgDiagnosticBudgetRepository::new(pool.clone());
    let registration = registration();
    let attempt = attempt();
    repo.provision(&registration).await.unwrap();
    repo.reserve(&registration, &attempt).await.unwrap();
    let invalid = Settlement {
        output_tokens: 9,
        ..usage()
    };
    assert_eq!(
        repo.settle(&registration, attempt.attempt_id, &invalid)
            .await,
        Err(BudgetError::Invalid)
    );
    assert_eq!(
        repo.read(&registration).await.unwrap().charged,
        attempt.quote().unwrap()
    );
    repo.seal(&registration).await.unwrap();
    repo.seal(&registration).await.unwrap();
    let other = Attempt {
        attempt_id: Uuid::new_v4(),
        ..attempt.clone()
    };
    assert_eq!(
        repo.reserve(&registration, &other).await,
        Err(BudgetError::Closed)
    );
    repo.settle(&registration, attempt.attempt_id, &usage())
        .await
        .unwrap();
    assert!(repo.read(&registration).await.unwrap().sealed);
    assert_eq!(
        repo.reserve(&registration, &other).await,
        Err(BudgetError::Closed)
    );

    // Expiry fixture is inserted directly to avoid a timing-dependent wall-clock sleep.
    // Public provisioning of this expired registration remains forbidden.
    let mut expired = registration.clone();
    expired.budget_id = Uuid::new_v4();
    expired.expires_at -= Duration::hours(2);
    assert_eq!(repo.provision(&expired).await, Err(BudgetError::Invalid));
    sqlx::query(
        "INSERT INTO diagnostic_llm_budgets (budget_id, contract, profile, profile_sha256,
         max_attempts, max_tokens, max_cost_micro_cny, expires_at)
         SELECT $1, contract, profile, profile_sha256, max_attempts, max_tokens, max_cost_micro_cny, $2
         FROM diagnostic_llm_budgets WHERE budget_id = $3",
    ).bind(expired.budget_id).bind(expired.expires_at).bind(registration.budget_id)
        .execute(&pool).await.unwrap();
    assert_eq!(
        repo.reserve(&expired, &other).await,
        Err(BudgetError::Closed)
    );
    assert_eq!(
        repo.read(&expired).await.unwrap().charged,
        Amount::default()
    );
    // Advance this fixture's expiry after creating a real receipt, without a wall-clock sleep.
    // This is test-only SQL, not a mutation exposed by the repository or application.
    let mut late = Registration {
        budget_id: Uuid::new_v4(),
        ..registration.clone()
    };
    repo.provision(&late).await.unwrap();
    repo.reserve(&late, &attempt).await.unwrap();
    late.expires_at -= Duration::hours(2);
    sqlx::query("UPDATE diagnostic_llm_budgets SET expires_at = $2 WHERE budget_id = $1")
        .bind(late.budget_id)
        .bind(late.expires_at)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(repo.reserve(&late, &other).await, Err(BudgetError::Closed));
    repo.settle(&late, attempt.attempt_id, &usage())
        .await
        .unwrap();
    repo.settle(&late, attempt.attempt_id, &usage())
        .await
        .unwrap();
    assert_eq!(repo.read(&late).await.unwrap().charged.attempts, 1);
    assert_eq!(repo.reserve(&late, &other).await, Err(BudgetError::Closed));
}

#[derive(sqlx::FromRow)]
struct LockedBudget {
    _budget_id: Uuid,
}

#[tokio::test]
async fn lock_timeout_returns_no_grant_and_never_retries_the_attempt() {
    let pool = pool().await;
    let repo = PgDiagnosticBudgetRepository::new(pool.clone());
    let registration = registration();
    repo.provision(&registration).await.unwrap();
    let attempt = attempt();
    let mut lock = pool.begin().await.unwrap();
    sqlx::query_as::<_, LockedBudget>(
        "SELECT budget_id AS _budget_id FROM diagnostic_llm_budgets WHERE budget_id = $1 FOR UPDATE",
    ).bind(registration.budget_id).fetch_one(&mut *lock).await.unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        repo.reserve(&registration, &attempt).await,
        Err(BudgetError::Unavailable)
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    lock.rollback().await.unwrap();
    assert_eq!(
        repo.read(&registration).await.unwrap().charged,
        Amount::default()
    );
    let totals = sqlx::query_as::<_, ReceiptTotals>(
        "SELECT count(*) AS count, count(DISTINCT ordinal) AS ordinals FROM diagnostic_llm_attempts WHERE budget_id = $1",
    ).bind(registration.budget_id).fetch_one(&pool).await.unwrap();
    assert_eq!(totals.count, 0);
    // A separate explicit new attempt works; the timed-out operation did not schedule a retry.
    let fresh = Attempt {
        attempt_id: Uuid::new_v4(),
        ..attempt
    };
    let receipt = repo.reserve(&registration, &fresh).await.unwrap();
    assert_eq!(receipt.ordinal, 1);
}

#[tokio::test]
async fn independent_connections_retain_charges_and_missing_registration_is_not_created() {
    let first_pool = pool().await;
    let repo = PgDiagnosticBudgetRepository::new(first_pool.clone());
    let registration = registration();
    let attempt = attempt();
    assert_eq!(repo.read(&registration).await, Err(BudgetError::NotFound));
    assert_eq!(
        repo.reserve(&registration, &attempt).await,
        Err(BudgetError::NotFound)
    );
    repo.provision(&registration).await.unwrap();
    assert_eq!(
        repo.provision(&registration).await,
        Err(BudgetError::Conflict)
    );
    // Drop the returned receipt: caller does not reuse an unobserved successful grant.
    repo.reserve(&registration, &attempt).await.unwrap();
    drop(repo);
    first_pool.close().await;
    let second_pool = pool().await;
    let restarted = PgDiagnosticBudgetRepository::new(second_pool.clone());
    assert_eq!(
        restarted.read(&registration).await.unwrap().charged,
        attempt.quote().unwrap()
    );
    assert_eq!(
        restarted.reserve(&registration, &attempt).await,
        Err(BudgetError::Conflict)
    );
    assert_eq!(
        restarted.provision(&registration).await,
        Err(BudgetError::Conflict)
    );
    let mut mismatch = registration.clone();
    mismatch.limits.attempts -= 1;
    assert_eq!(restarted.read(&mismatch).await, Err(BudgetError::Conflict));
    assert_eq!(
        restarted.reserve(&mismatch, &attempt).await,
        Err(BudgetError::Conflict)
    );
    assert_eq!(restarted.seal(&mismatch).await, Err(BudgetError::Conflict));
    assert_eq!(
        restarted
            .settle(&mismatch, attempt.attempt_id, &usage())
            .await,
        Err(BudgetError::Conflict)
    );
    assert_eq!(
        restarted
            .settle(&registration, Uuid::new_v4(), &usage())
            .await,
        Err(BudgetError::NotFound)
    );

    let missing = Registration {
        budget_id: Uuid::new_v4(),
        ..registration.clone()
    };
    restarted.provision(&missing).await.unwrap();
    // Simulated accidental deletion of an unused registration. Read/bootstrap cannot repair it.
    sqlx::query("DELETE FROM diagnostic_llm_budgets WHERE budget_id = $1")
        .bind(missing.budget_id)
        .execute(&second_pool)
        .await
        .unwrap();
    assert_eq!(restarted.read(&missing).await, Err(BudgetError::NotFound));
    assert_eq!(
        restarted.reserve(&missing, &attempt).await,
        Err(BudgetError::NotFound)
    );
    assert_eq!(restarted.read(&missing).await, Err(BudgetError::NotFound));
}

#[tokio::test]
async fn zero_allowance_and_bad_registration_fail_without_receipts() {
    let repo = PgDiagnosticBudgetRepository::new(pool().await);
    let mut registration = registration();
    registration.limits = Amount::default();
    let wrong_digest = Registration {
        profile_sha256: "a".repeat(64),
        ..registration.clone()
    };
    assert_eq!(
        repo.provision(&wrong_digest).await,
        Err(BudgetError::Invalid)
    );
    repo.provision(&registration).await.unwrap();
    assert_eq!(repo.read(&wrong_digest).await, Err(BudgetError::Invalid));
    assert_eq!(
        repo.reserve(&wrong_digest, &attempt()).await,
        Err(BudgetError::Invalid)
    );
    assert_eq!(repo.seal(&wrong_digest).await, Err(BudgetError::Invalid));
    assert_eq!(
        repo.settle(&wrong_digest, Uuid::new_v4(), &usage()).await,
        Err(BudgetError::Invalid)
    );
    assert_eq!(
        repo.reserve(&registration, &attempt()).await,
        Err(BudgetError::Exhausted)
    );
    assert_eq!(
        repo.read(&registration).await.unwrap().charged,
        Amount::default()
    );
    let invalid = Attempt {
        attempt_id: Uuid::nil(),
        ..attempt()
    };
    assert_eq!(
        repo.reserve(&registration, &invalid).await,
        Err(BudgetError::Invalid)
    );
    registration.budget_id = Uuid::nil();
    assert_eq!(
        repo.provision(&registration).await,
        Err(BudgetError::Invalid)
    );
    registration.budget_id = Uuid::new_v4();
    registration.expires_at += Duration::hours(4);
    assert_eq!(
        repo.provision(&registration).await,
        Err(BudgetError::Invalid)
    );
}

#[tokio::test]
async fn authenticated_http_control_is_strict_scoped_and_durable() {
    use user_service::application::diagnostic_budget::DiagnosticBudgetHandler;
    let repo = Arc::new(PgDiagnosticBudgetRepository::new(pool().await));
    let registration = registration();
    repo.provision(&registration).await.unwrap();
    let handler =
        Arc::new(DiagnosticBudgetHandler::new(registration.clone(), repo.clone()).unwrap());
    let token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let router =
        user_service::interface::http::diagnostic_budget::router(Some(handler), token.into());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let root = format!(
        "http://{address}/internal/llm-budget/{}",
        registration.budget_id
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let binding = serde_json::json!({
        "contract": registration.contract, "profile": registration.profile,
        "profile_sha256": registration.profile_sha256, "budget_id": registration.budget_id,
    });
    let attempt_id = Uuid::new_v4();
    let reserve = serde_json::json!({
        "binding": binding, "attempt_id": attempt_id, "provider": "deepseek",
        "model": Profile::compiled().model, "origin": "https://api.deepseek.com",
        "operation": "setup_connection", "output_limit": 8,
    });
    let post = |suffix: &str| {
        client
            .post(format!("{root}/{suffix}"))
            .header("X-Internal-Service-Token", token)
            .header("X-LLM-Budget-Contract", &registration.contract)
    };
    for supplied in [None, Some("wrong")] {
        let mut request = client
            .post(format!("{root}/reserve"))
            .header("X-LLM-Budget-Contract", &registration.contract)
            .json(&reserve);
        if let Some(token) = supplied {
            request = request.header("X-Internal-Service-Token", token);
        }
        assert_eq!(request.send().await.unwrap().status(), 401);
    }
    for (key, value) in [
        ("provider", serde_json::json!("other")),
        ("model", serde_json::json!("other")),
        ("origin", serde_json::json!("http://api.deepseek.com")),
        ("operation", serde_json::json!("embedding")),
        ("output_limit", serde_json::json!(9)),
        ("extra", serde_json::json!("PRIVATE_SENTINEL")),
    ] {
        let mut invalid = reserve.clone();
        invalid[key] = value;
        let response = post("reserve").json(&invalid).send().await.unwrap();
        assert_eq!(response.status(), 400);
        let text = response.text().await.unwrap();
        assert!(text.len() <= 4096 && !text.contains("PRIVATE_SENTINEL"));
    }
    for field in ["contract", "profile", "profile_sha256"] {
        let mut invalid = reserve.clone();
        invalid["binding"][field] = serde_json::json!("wrong");
        assert_eq!(
            post("reserve")
                .json(&invalid)
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
    }
    let mut wrong_scope = reserve.clone();
    wrong_scope["binding"]["budget_id"] = serde_json::json!(Uuid::new_v4());
    assert_eq!(
        post("reserve")
            .json(&wrong_scope)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    for raw in [
        serde_json::to_string(&reserve).unwrap().replacen(
            "\"output_limit\":8",
            "\"output_limit\":8,\"output_limit\":8",
            1,
        ),
        " ".repeat(4097),
        serde_json::to_string(&reserve).unwrap().replace(
            &attempt_id.to_string(),
            "00000000-0000-4000-0000-000000000001",
        ),
    ] {
        assert_eq!(
            post("reserve")
                .header("Content-Type", "application/json")
                .body(raw)
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
    }
    assert_eq!(
        client
            .post(format!("{root}/reserve"))
            .header("X-Internal-Service-Token", token)
            .json(&reserve)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        repo.read(&registration).await.unwrap().charged,
        Amount::default()
    );

    let response = post("reserve").json(&reserve).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let receipt: serde_json::Value = response.json().await.unwrap();
    assert_eq!(receipt["binding"], binding);
    assert_eq!(receipt["attempt_id"], attempt_id.to_string());
    assert_eq!(receipt["reservation"]["tokens"], 1048584);
    assert_eq!(
        post("reserve")
            .json(&reserve)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    let mut wrong_seal = binding.clone();
    wrong_seal["budget_id"] = serde_json::json!(Uuid::new_v4());
    assert_eq!(
        post("seal")
            .json(&serde_json::json!({"binding": wrong_seal}))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert!(!repo.read(&registration).await.unwrap().sealed);
    let seal_body = serde_json::json!({"binding": binding});
    assert_eq!(
        post("seal").json(&seal_body).send().await.unwrap().status(),
        200
    );
    let mut fresh = reserve.clone();
    fresh["attempt_id"] = serde_json::json!(Uuid::new_v4());
    assert_eq!(
        post("reserve").json(&fresh).send().await.unwrap().status(),
        410
    );
    let settlement = serde_json::json!({"binding": binding, "attempt_id": attempt_id,
        "usage": {"model": Profile::compiled().model, "input_tokens":10, "output_tokens":2, "cached_input_tokens":null}});
    for _ in 0..2 {
        let response = post("settle").json(&settlement).send().await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap(),
            settlement
        );
    }
    let mut conflict = settlement.clone();
    conflict["usage"]["cached_input_tokens"] = serde_json::json!(0);
    assert_eq!(
        post("settle")
            .json(&conflict)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    conflict["attempt_id"] = serde_json::json!(Uuid::new_v4());
    assert_eq!(
        post("settle")
            .json(&conflict)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let response = client
        .get(&root)
        .header("X-Internal-Service-Token", token)
        .header("X-LLM-Budget-Contract", &registration.contract)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let snapshot: serde_json::Value = response.json().await.unwrap();
    assert_eq!(snapshot["binding"], binding);
    assert_eq!(
        snapshot["charged"],
        serde_json::json!({"attempts":1,"tokens":12,"cost_micro_cny":64})
    );
    assert_eq!(snapshot["sealed"], true);
    server.abort();
    let _ = server.await;
}
