use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub const TOKEN_BYTES: usize = 32;
pub const PAT_PREFIX: &str = "vdn_";
pub const LEGACY_SHA256_PREFIX: &str = "sha256:";
pub const HMAC_SHA256_PREFIX: &str = "hmac-sha256:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenDigest {
    HmacSha256(String),
    LegacySha256(String),
}

impl TokenDigest {
    pub fn for_storage(&self) -> String {
        match self {
            Self::HmacSha256(value) => format!("{HMAC_SHA256_PREFIX}{value}"),
            Self::LegacySha256(value) => format!("{LEGACY_SHA256_PREFIX}{value}"),
        }
    }

    pub fn version_label(&self) -> &'static str {
        match self {
            Self::HmacSha256(_) => "hmac-sha256",
            Self::LegacySha256(_) => "sha256",
        }
    }
}

pub fn new_session_token() -> String {
    random_hex(TOKEN_BYTES)
}

pub fn new_pat_token() -> String {
    format!("{PAT_PREFIX}{}", random_hex(TOKEN_BYTES))
}

pub fn digest_token(token: &str, pepper: Option<&str>) -> TokenDigest {
    match pepper.map(str::trim).filter(|value| !value.is_empty()) {
        Some(pepper) => {
            TokenDigest::HmacSha256(hmac_sha256_hex(pepper.as_bytes(), token.as_bytes()))
        }
        None => TokenDigest::LegacySha256(sha256_hex(token.as_bytes())),
    }
}

pub fn legacy_digest_for_lookup(token: &str) -> String {
    TokenDigest::LegacySha256(sha256_hex(token.as_bytes())).for_storage()
}

pub fn lookup_digests(token: &str, pepper: Option<&str>) -> Vec<String> {
    let primary = digest_token(token, pepper).for_storage();
    let legacy = legacy_digest_for_lookup(token);
    if primary == legacy {
        vec![primary]
    } else {
        vec![primary, legacy]
    }
}

pub fn validate_deployed_pepper(pepper: Option<&str>) -> Result<(), &'static str> {
    let Some(pepper) = pepper.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err("required-Postgres deployments must set VICEDEN_AUTH__TOKEN_PEPPER");
    };
    if pepper.len() < 32 {
        return Err("VICEDEN_AUTH__TOKEN_PEPPER must be at least 32 characters");
    }
    Ok(())
}

fn random_hex(byte_len: usize) -> String {
    let mut bytes = vec![0u8; byte_len];
    getrandom::fill(&mut bytes).expect("system RNG should be available");
    hex::encode(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn hmac_sha256_hex(key: &[u8], bytes: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts arbitrary key length");
    mac.update(bytes);
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_tokens_are_not_uuid_shaped() {
        let token = new_session_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(token.matches('-').count(), 0);
    }

    #[test]
    fn pats_keep_prefix_and_have_256_bits_of_random_material() {
        let token = new_pat_token();
        assert!(token.starts_with(PAT_PREFIX));
        assert_eq!(token.len(), PAT_PREFIX.len() + 64);
        assert!(token[PAT_PREFIX.len()..].chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn peppered_digests_are_versioned_and_do_not_contain_raw_token() {
        let token = new_pat_token();
        let digest = digest_token(&token, Some("a-long-random-pepper-for-tests-1234567890"));
        let storage = digest.for_storage();
        assert!(storage.starts_with(HMAC_SHA256_PREFIX));
        assert!(!storage.contains(&token));
        assert_eq!(digest.version_label(), "hmac-sha256");
    }

    #[test]
    fn legacy_lookup_is_available_for_transitional_rows() {
        let token = "legacy-token";
        let digests = lookup_digests(token, Some("a-long-random-pepper-for-tests-1234567890"));
        assert_eq!(digests.len(), 2);
        assert!(digests.iter().any(|item| item.starts_with(HMAC_SHA256_PREFIX)));
        assert!(digests.iter().any(|item| item.starts_with(LEGACY_SHA256_PREFIX)));
    }

    #[test]
    fn deployed_pepper_policy_rejects_empty_and_short_values() {
        assert!(validate_deployed_pepper(None).is_err());
        assert!(validate_deployed_pepper(Some("short")).is_err());
        assert!(validate_deployed_pepper(Some("a-long-random-pepper-for-tests-1234567890")).is_ok());
    }
}
