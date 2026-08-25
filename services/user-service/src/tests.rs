use crate::{
    application::handlers::DUMMY_PASSWORD_HASH,
    domain::ports::PasswordHasher,
    infrastructure::auth::{jwt::JwtService, password::BcryptPasswordHasher},
};

#[test]
fn test_email_validation() {
    use crate::application::handlers::is_valid_email;

    for valid in [
        "user@example.com",
        "test.name@sub.domain.co",
        "a@b.c",
        "a@b",
        "user@localhost",
        "user+tag@example.com",
        "!#$%&'*+-/=?^_`{|}~@example.com",
        "\"john doe\"@example.com",
        "\"a@b\"@example.com",
        "\"a\\\"b@c\"@example.com",
        "\"quoted\\\\pair\"@example.com",
        "user@[192.168.1.1]",
        "user@[001.002.003.004]",
        "user@[IPv6:2001:db8::1]",
        &format!("{}@example.com", "a".repeat(64)),
        &format!("user@{}.com", "x".repeat(63)),
        &format!(
            "a@{}.{}.{}.{}",
            "x".repeat(63),
            "x".repeat(63),
            "x".repeat(63),
            "x".repeat(60),
        ),
    ] {
        assert!(is_valid_email(valid), "should accept {valid:?}");
    }

    for invalid in [
        "",
        "missing-at-sign",
        "@no-local.com",
        "no-domain@",
        "bad@.start",
        "bad@end.",
        "user..name@example.com",
        ".leading@example.com",
        "trailing.@example.com",
        "sp ace@example.com",
        "user@exa mple.com",
        "user@-leading.com",
        "user@trailing-.com",
        "user@exa_mple.com",
        "user@exa..mple.com",
        "com,ment@example.com",
        "user(name)@example.com",
        "user@example.com\n",
        "user@[1.2.3]",
        "user@[1.2.3.4.5]",
        "user@[.1.2.3]",
        "user@[1..2.3]",
        "user@[256.1.1.1]",
        "user@[IPv6:::g]",
        "user@[IPv6:1:2:3:4:5:6:7:8:9]",
        "user@[2001:db8::1]",
        "user@[tag:some-value]",
        "user@[IPv6:]",
        "user@[IPv6:2001:db8::1",
        "user@[tag:]",
        "user@[tag:[]",
        "user@[tag:]]",
        "user@[:x]",
        "\"unterminated@example.com",
        "\"bad\\@example.com",
        "\"a\"x@example.com",
        "用户@例子.公司",
        "user@münchen.de",
        "\u{212a}@example.com",
        &format!("{}@example.com", "a".repeat(65)),
        &format!("user@{}.com", "x".repeat(64)),
        &format!(
            "a@{}.{}.{}.{}",
            "x".repeat(63),
            "x".repeat(63),
            "x".repeat(63),
            "x".repeat(61),
        ),
    ] {
        assert!(!is_valid_email(invalid), "should reject {invalid:?}");
    }
}

#[test]
fn atext_membership_matches_the_rfc_5322_list() {
    use crate::application::handlers::is_valid_email;

    // RFC 5322 §3.2.3 atext: ALPHA / DIGIT and these symbols, independently
    // restated so drift from the validator fails this check.
    const ATEXT_SYMBOLS: &str = "!#$%&'*+-/=?^_`{|}~";
    for byte in 0..=127u8 {
        let character = byte as char;
        if character == '.' {
            continue; // dot-string separator, covered by the table above
        }
        let expected = character.is_ascii_alphanumeric() || ATEXT_SYMBOLS.contains(character);
        let email = format!("u{character}x@example.com");
        assert_eq!(
            is_valid_email(&email),
            expected,
            "byte {byte} ({character:?}) must match the atext list"
        );
    }
}

#[test]
fn test_jwt_roundtrip() {
    let svc = JwtService::new("test-secret-32-chars-minimum!!", 3600);
    let user_id = uuid::Uuid::new_v4();

    let token = svc
        .generate_token(user_id, "test@example.com", "user")
        .unwrap();
    let claims = svc.verify_token(&token).unwrap();

    assert_eq!(claims.sub, user_id.to_string());
    assert_eq!(claims.email, "test@example.com");
    assert_eq!(claims.role, "user");
}

#[test]
fn test_jwt_invalid_token() {
    let svc = JwtService::new("secret-key-that-is-long-enough!!", 3600);
    let result = svc.verify_token("invalid.token.here");
    assert!(result.is_err());
}

#[test]
fn test_jwt_wrong_secret() {
    let svc1 = JwtService::new("first-secret-long-enough-key!!", 3600);
    let svc2 = JwtService::new("second-secret-long-enough-key!", 3600);
    let user_id = uuid::Uuid::new_v4();

    let token = svc1
        .generate_token(user_id, "test@example.com", "user")
        .unwrap();
    let result = svc2.verify_token(&token);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_bcrypt_hash_verify() {
    let password = format!("test-password-{}", uuid::Uuid::new_v4());
    let wrong_password = format!("{password}-wrong");
    let dummy_probe = format!("{password}-dummy-probe");
    let hasher = BcryptPasswordHasher::new(1);
    let hash = hasher.hash(&password).await.unwrap();

    assert!(hash.starts_with("$2b$12$"));
    assert!(hasher.verify(&password, &hash).await.unwrap());
    assert!(!hasher.verify(&wrong_password, &hash).await.unwrap());
    assert!(!hasher
        .verify(&dummy_probe, DUMMY_PASSWORD_HASH)
        .await
        .unwrap());
}

#[test]
fn test_user_creation() {
    use crate::domain::entities::user::{User, UserRole};

    let user = User::new(
        "test@example.com".into(),
        "hashed_password".into(),
        Some("Test User".into()),
    );

    assert_eq!(user.email, "test@example.com");
    assert_eq!(user.role, UserRole::User);
    assert!(user.name.is_some());
    assert!(!user.email_verified);
    assert!(user.last_sign_in.is_none());
}

#[test]
fn test_refresh_token() {
    use crate::domain::entities::user::RefreshToken;

    let token = RefreshToken::new(uuid::Uuid::new_v4(), "test-token-string".into(), 3600);
    assert!(!token.is_expired());

    let expired = RefreshToken::new(uuid::Uuid::new_v4(), "expired".into(), -1);
    assert!(expired.is_expired());
}
