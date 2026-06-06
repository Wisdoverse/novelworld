#[cfg(test)]
mod tests {
    use crate::infrastructure::auth::jwt::JwtService;

    #[test]
    fn test_email_validation() {
        use crate::application::handlers::is_valid_email;

        assert!(is_valid_email("user@example.com"));
        assert!(is_valid_email("test.name@sub.domain.co"));
        assert!(is_valid_email("a@b.c"));
        assert!(!is_valid_email(""));
        assert!(!is_valid_email("missing-at-sign"));
        assert!(!is_valid_email("@no-local.com"));
        assert!(!is_valid_email("no-domain@"));
        assert!(!is_valid_email("bad@.start"));
        assert!(!is_valid_email("bad@end."));
    }

    #[test]
    fn test_jwt_roundtrip() {
        let svc = JwtService::new("test-secret-32-chars-minimum!!", 3600);
        let user_id = uuid::Uuid::new_v4();

        let token = svc.generate_token(user_id, "test@example.com", "user").unwrap();
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

        let token = svc1.generate_token(user_id, "test@example.com", "user").unwrap();
        let result = svc2.verify_token(&token);
        assert!(result.is_err());
    }

    #[test]
    fn test_bcrypt_hash_verify() {
        let password = "my_secure_password";
        let hash = bcrypt::hash(password, 12).unwrap();

        assert!(bcrypt::verify(password, &hash).unwrap());
        assert!(!bcrypt::verify("wrong_password", &hash).unwrap());
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

        let token = RefreshToken::new(
            uuid::Uuid::new_v4(),
            "test-token-string".into(),
            3600,
        );
        assert!(!token.is_expired());

        let expired = RefreshToken::new(
            uuid::Uuid::new_v4(),
            "expired".into(),
            -1,
        );
        assert!(expired.is_expired());
    }
}
