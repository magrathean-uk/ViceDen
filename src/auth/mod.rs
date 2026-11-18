//! Authentication domain.
//!
//! Planned responsibilities:
//! - password hashing and verification
//! - passkey registration / authentication
//! - session issuance and validation
//! - PAT creation and verification
//! - founder/admin privilege checks

use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Algorithm, Argon2, Params, Version,
};
use sha2::{Digest, Sha256};

pub mod age_attestation;
pub mod invites;
pub mod sessions;
pub mod tokens;

/// F10: Argon2id parameters, pinned at the OWASP 2024 floor.
///
/// `Params::DEFAULT` happens to match (m=19456 KiB, t=2, p=1) on Argon2
/// 0.6.x, but we spell it out so a future crate bump can't silently
/// soften the password hash. The associated `argon2id_floor_holds` test
/// reads back the parameters and fails CI if they drift.
fn argon2_hasher() -> Argon2<'static> {
    let params = Params::new(
        19_456, // m_cost (KiB) — OWASP 2024 floor
        2,      // t_cost (iterations)
        1,      // p_cost (parallelism)
        None,   // output length — default 32 bytes
    )
    .expect("argon2 params should be valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

pub fn hash_password(password: &str) -> String {
    let mut salt_bytes = [0u8; 16];
    getrandom::fill(&mut salt_bytes).expect("system RNG should be available");
    argon2_hasher()
        .hash_password_with_salt(password.as_bytes(), &salt_bytes)
        .expect("argon2 password hashing should work")
        .to_string()
}

pub fn legacy_password_hash(handle: &str, password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{handle}:{password}").as_bytes());
    hex::encode(hasher.finalize())
}

pub fn verify_password(handle: &str, password: &str, stored_hash: &str) -> bool {
    if stored_hash.starts_with("$argon2") {
        // Verify uses the parameters embedded in the PHC string, not our
        // local hasher — older hashes with different params still
        // validate. Argon2::default() is fine for verification.
        return PasswordHash::new(stored_hash).ok().is_some_and(|parsed| {
            Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
        });
    }

    stored_hash == legacy_password_hash(handle, password)
}

#[cfg(test)]
mod tests {
    #[test]
    fn password_hashes_use_argon2id_and_verify() {
        let hash = super::hash_password("secret123");

        assert!(hash.starts_with("$argon2id$"));
        assert!(super::verify_password("member", "secret123", &hash));
        assert!(!super::verify_password("member", "wrong-password", &hash));
    }

    #[test]
    fn legacy_sha256_password_hashes_still_verify() {
        let legacy = super::legacy_password_hash("root", "root");

        assert!(super::verify_password("root", "root", &legacy));
        assert!(!super::verify_password("root", "wrong-password", &legacy));
    }

    /// F10: regression test pinning the Argon2id parameters at the
    /// OWASP 2024 floor. If a future crate bump or refactor softens
    /// these, CI fails here.
    #[test]
    fn argon2id_floor_holds() {
        let hash = super::hash_password("floor-check-password");
        // PHC string format: $argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>
        assert!(hash.starts_with("$argon2id$v=19$"));
        assert!(hash.contains("m=19456"), "m_cost must be ≥ 19MiB (OWASP 2024 floor); got {hash}");
        assert!(hash.contains("t=2"), "t_cost must be ≥ 2 (OWASP 2024 floor); got {hash}");
        assert!(hash.contains("p=1"), "p_cost must be ≥ 1 (OWASP 2024 floor); got {hash}");
    }
}
