//! Tests for credential storage.

use super::*;

// === Serialization tests (don't require keyring) ===

#[test]
fn test_credential_serialization_full() {
    let cred = Credential {
        access_token: "ghp_abc123".to_string(),
        refresh_token: Some("ghr_xyz789".to_string()),
        expires_at: Some("2024-12-31T23:59:59Z".to_string()),
    };

    let json = serde_json::to_string(&cred).unwrap();
    assert!(json.contains("ghp_abc123"));
    assert!(json.contains("ghr_xyz789"));
    assert!(json.contains("2024-12-31T23:59:59Z"));
}

#[test]
fn test_credential_serialization_minimal() {
    let cred = Credential {
        access_token: "token123".to_string(),
        refresh_token: None,
        expires_at: None,
    };

    let json = serde_json::to_string(&cred).unwrap();
    assert!(json.contains("token123"));
    // Optional fields should be omitted when None
    assert!(!json.contains("refresh_token"));
    assert!(!json.contains("expires_at"));
}

#[test]
fn test_credential_deserialization_full() {
    let json = r#"{"access_token":"abc","refresh_token":"xyz","expires_at":"2024-01-01"}"#;
    let cred: Credential = serde_json::from_str(json).unwrap();

    assert_eq!(cred.access_token, "abc");
    assert_eq!(cred.refresh_token, Some("xyz".to_string()));
    assert_eq!(cred.expires_at, Some("2024-01-01".to_string()));
}

#[test]
fn test_credential_deserialization_minimal() {
    let json = r#"{"access_token":"token_only"}"#;
    let cred: Credential = serde_json::from_str(json).unwrap();

    assert_eq!(cred.access_token, "token_only");
    assert_eq!(cred.refresh_token, None);
    assert_eq!(cred.expires_at, None);
}

#[test]
fn test_credential_deserialization_with_null_fields() {
    let json = r#"{"access_token":"tok","refresh_token":null,"expires_at":null}"#;
    let cred: Credential = serde_json::from_str(json).unwrap();

    assert_eq!(cred.access_token, "tok");
    assert_eq!(cred.refresh_token, None);
    assert_eq!(cred.expires_at, None);
}

#[test]
fn test_credential_roundtrip_serialization() {
    let original = Credential {
        access_token: "access".to_string(),
        refresh_token: Some("refresh".to_string()),
        expires_at: Some("2025-06-15T12:00:00Z".to_string()),
    };

    let json = serde_json::to_string(&original).unwrap();
    let restored: Credential = serde_json::from_str(&json).unwrap();

    assert_eq!(original.access_token, restored.access_token);
    assert_eq!(original.refresh_token, restored.refresh_token);
    assert_eq!(original.expires_at, restored.expires_at);
}

// === Mock store tests ===

#[test]
fn test_credential_roundtrip() {
    let test_service = "_isq_test_credential";

    // Set a credential
    set_credential(
        test_service,
        "test_access_token",
        Some("test_refresh_token"),
        Some("2024-12-31T23:59:59Z"),
    )
    .unwrap();

    // Retrieve
    let cred = get_credential(test_service)
        .expect("Failed to get credential")
        .expect("Credential not found");

    assert_eq!(cred.access_token, "test_access_token");
    assert_eq!(cred.refresh_token, Some("test_refresh_token".to_string()));
    assert_eq!(cred.expires_at, Some("2024-12-31T23:59:59Z".to_string()));

    // Clean up
    let _ = remove_credential(test_service);

    // Verify removal
    let cred = get_credential(test_service).expect("Failed to get credential");
    assert!(cred.is_none());
}

#[test]
fn test_credential_minimal() {
    let test_service = "_isq_test_minimal";

    // Set a credential with only access token
    set_credential(test_service, "minimal_token", None, None).unwrap();

    let cred = get_credential(test_service)
        .expect("Failed to get credential")
        .expect("Credential not found");

    assert_eq!(cred.access_token, "minimal_token");
    assert_eq!(cred.refresh_token, None);
    assert_eq!(cred.expires_at, None);

    // Clean up
    let _ = remove_credential(test_service);
}

#[test]
fn test_get_nonexistent_credential() {
    // Getting a credential that doesn't exist should return None, not error
    let result = get_credential("_isq_definitely_does_not_exist_xyz123");
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

// === File format tests ===

#[test]
fn test_store_serialization() {
    use std::collections::HashMap;

    let mut store: HashMap<String, Credential> = HashMap::new();
    store.insert(
        "github".to_string(),
        Credential {
            access_token: "ghp_xxx".to_string(),
            refresh_token: None,
            expires_at: None,
        },
    );
    store.insert(
        "linear".to_string(),
        Credential {
            access_token: "lin_api_xxx".to_string(),
            refresh_token: None,
            expires_at: None,
        },
    );

    let json = serde_json::to_string_pretty(&store).unwrap();
    assert!(json.contains("github"));
    assert!(json.contains("linear"));
    assert!(json.contains("ghp_xxx"));
    assert!(json.contains("lin_api_xxx"));
}

#[test]
fn test_store_deserialization() {
    use std::collections::HashMap;

    let json = r#"{
        "github": {
            "access_token": "ghp_xxx",
            "refresh_token": null,
            "expires_at": null
        },
        "linear": {
            "access_token": "lin_api_xxx"
        }
    }"#;

    let store: HashMap<String, Credential> = serde_json::from_str(json).unwrap();
    assert_eq!(store.len(), 2);
    assert_eq!(store.get("github").unwrap().access_token, "ghp_xxx");
    assert_eq!(store.get("linear").unwrap().access_token, "lin_api_xxx");
}
