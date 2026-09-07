use crate::diagnostic_budget::{canonical_uuid, Amount, Binding, ReserveRequest, SettleRequest};
use uuid::Uuid;

const BUDGET_ID: &str = "550e8400-e29b-41d4-a716-446655440000";
const ATTEMPT_ID: &str = "550e8400-e29b-41d4-a716-446655440001";

fn binding_json() -> &'static str {
    r#"{
        "contract":"diagnostic-v1",
        "profile":"vision-diagnostic-budget-v1",
        "profile_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "budget_id":"550e8400-e29b-41d4-a716-446655440000"
    }"#
}

fn reserve_json() -> String {
    format!(
        r#"{{
            "binding":{},
            "attempt_id":"{}",
            "provider":"deepseek",
            "model":"deepseek-v4-flash-vision-exp",
            "origin":"diagnostic",
            "operation":"judge",
            "output_limit":8192
        }}"#,
        binding_json(),
        ATTEMPT_ID
    )
}

fn settle_json() -> String {
    format!(
        r#"{{
            "binding":{},
            "attempt_id":"{}",
            "usage":{{
                "model":"deepseek-v4-flash-vision-exp",
                "input_tokens":10,
                "output_tokens":20,
                "cached_input_tokens":null
            }}
        }}"#,
        binding_json(),
        ATTEMPT_ID
    )
}

#[test]
fn canonical_uuid_accepts_only_lowercase_rfc_v4() {
    let id = canonical_uuid(BUDGET_ID).expect("canonical RFC v4 UUID");
    assert_eq!(id, Uuid::parse_str(BUDGET_ID).unwrap());
}

#[test]
fn canonical_uuid_rejects_noncanonical_or_wrong_variant_values() {
    for value in [
        "550E8400-E29B-41D4-A716-446655440000",
        "550e8400e29b41d4a716446655440000",
        "550e8400-e29b-41d4-0716-446655440000",
        "00000000-0000-0000-0000-000000000000",
        "550e8400-e29b-11d4-a716-446655440000",
    ] {
        assert!(
            canonical_uuid(value).is_err(),
            "accepted invalid UUID {value}"
        );
    }
}

#[test]
fn binding_rejects_unknown_fields_and_wrong_id_variant() {
    let unknown = format!(
        r#"{{"contract":"diagnostic-v1","profile":"vision-diagnostic-budget-v1","profile_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","budget_id":"{}","extra":true}}"#,
        BUDGET_ID
    );
    assert!(serde_json::from_str::<serde_json::Value>(&unknown).is_ok());
    assert!(serde_json::from_str::<Binding>(&unknown).is_err());

    let wrong_variant = r#"{"contract":"diagnostic-v1","profile":"vision-diagnostic-budget-v1","profile_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","budget_id":"550e8400-e29b-41d4-0716-446655440000"}"#;
    assert!(serde_json::from_str::<Binding>(wrong_variant).is_err());
}

#[test]
fn reserve_and_settle_reject_unknown_fields_and_duplicates() {
    let reserve_unknown =
        reserve_json().replace("\n        }", ",\n            \"extra\":true\n        }");
    assert!(serde_json::from_str::<serde_json::Value>(&reserve_unknown).is_ok());
    assert!(serde_json::from_str::<ReserveRequest>(&reserve_unknown).is_err());

    let settle_unknown =
        settle_json().replace("\n        }", ",\n            \"extra\":true\n        }");
    assert!(serde_json::from_str::<serde_json::Value>(&settle_unknown).is_ok());
    assert!(serde_json::from_str::<SettleRequest>(&settle_unknown).is_err());

    let reserve_duplicate = reserve_json().replace(
        "\"output_limit\":8192",
        "\"output_limit\":8192,\"output_limit\":8191",
    );
    assert!(serde_json::from_str::<ReserveRequest>(&reserve_duplicate).is_err());

    let settle_duplicate = settle_json().replace(
        "\"cached_input_tokens\":null",
        "\"cached_input_tokens\":null,\"cached_input_tokens\":null",
    );
    assert!(serde_json::from_str::<SettleRequest>(&settle_duplicate).is_err());
}

#[test]
fn integer_fields_reject_negative_float_and_overflow_values() {
    for value in ["-1", "1.5", "18446744073709551616"] {
        let json = format!(r#"{{"attempts":{},"tokens":1,"cost_micro_cny":1}}"#, value);
        assert!(
            serde_json::from_str::<Amount>(&json).is_err(),
            "accepted {value}"
        );
    }

    let reserve_negative = reserve_json().replace("\"output_limit\":8192", "\"output_limit\":-1");
    assert!(serde_json::from_str::<ReserveRequest>(&reserve_negative).is_err());

    let settle_float = settle_json().replace("\"input_tokens\":10", "\"input_tokens\":1.5");
    assert!(serde_json::from_str::<SettleRequest>(&settle_float).is_err());
}

#[test]
fn required_fields_are_required_but_cached_usage_is_optional() {
    let missing_reserve_model =
        reserve_json().replace("\"model\":\"deepseek-v4-flash-vision-exp\",", "");
    assert!(serde_json::from_str::<ReserveRequest>(&missing_reserve_model).is_err());

    let missing_settle_usage = settle_json().replace(
        ",\n            \"usage\":{\n                \"model\":\"deepseek-v4-flash-vision-exp\",\n                \"input_tokens\":10,\n                \"output_tokens\":20,\n                \"cached_input_tokens\":null\n            }",
        "",
    );
    assert!(serde_json::from_str::<serde_json::Value>(&missing_settle_usage).is_ok());
    assert!(serde_json::from_str::<SettleRequest>(&missing_settle_usage).is_err());

    let without_cached =
        settle_json().replace(",\n                \"cached_input_tokens\":null", "");
    let parsed = serde_json::from_str::<SettleRequest>(&without_cached).unwrap();
    assert_eq!(parsed.usage.cached_input_tokens, None);
}
