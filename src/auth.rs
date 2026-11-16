//! Password hashing. Passwords are never stored; only Argon2id hashes
//! (with a random per-user salt) go into the database.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

pub const MIN_PASSWORD_LEN: usize = 8;

pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("Could not secure the password: {e}"))
}

pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    match PasswordHash::new(stored_hash) {
        Ok(h) => Argon2::default().verify_password(password.as_bytes(), &h).is_ok(),
        Err(_) => false,
    }
}

/// Checks a new password and its confirmation.
pub fn check_new_password(password: &str, confirm: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(format!("Password must be at least {MIN_PASSWORD_LEN} characters."));
    }
    if password != confirm {
        return Err("The two passwords don't match.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify() {
        let h = hash_password("correct horse").unwrap();
        assert!(h.starts_with("$argon2"));
        assert!(!h.contains("correct horse"));
        assert!(verify_password("correct horse", &h));
        assert!(!verify_password("Correct horse", &h));
        assert!(!verify_password("anything", "not a hash"));
        // A reset (empty) hash never matches, not even an empty password.
        assert!(!verify_password("", ""));
        assert!(!verify_password("anything", ""));
        // Same password, different salt -> different hash.
        assert_ne!(h, hash_password("correct horse").unwrap());
    }

    #[test]
    fn new_password_rules() {
        assert!(check_new_password("short", "short").is_err());
        assert!(check_new_password("longenough", "longenougH").is_err());
        assert!(check_new_password("longenough", "longenough").is_ok());
    }
}
