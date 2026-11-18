//! Age attestation: DOB validation + write-once row insert.

use chrono::{Datelike, NaiveDate};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AgeError {
    #[error("date of birth is in the future")]
    FutureDob,
    #[error("must be at least 18 years old")]
    Underage,
}

pub const MINIMUM_AGE: i32 = 18;

pub fn validate_dob(dob: NaiveDate, today: NaiveDate) -> Result<(), AgeError> {
    if dob > today {
        return Err(AgeError::FutureDob);
    }
    let age = age_in_years(dob, today);
    if age < MINIMUM_AGE {
        return Err(AgeError::Underage);
    }
    Ok(())
}

fn age_in_years(dob: NaiveDate, today: NaiveDate) -> i32 {
    let mut age = today.year() - dob.year();
    let had_birthday = (today.month(), today.day()) >= (dob.month(), dob.day());
    if !had_birthday {
        age -= 1;
    }
    age
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_arithmetic_handles_birthday_today() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 8).unwrap();
        let dob = NaiveDate::from_ymd_opt(2008, 5, 8).unwrap();
        assert_eq!(age_in_years(dob, today), 18);
    }

    #[test]
    fn age_arithmetic_handles_birthday_tomorrow() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 8).unwrap();
        let dob = NaiveDate::from_ymd_opt(2008, 5, 9).unwrap();
        assert_eq!(age_in_years(dob, today), 17);
    }

    #[test]
    fn dob_must_be_at_least_18_years_old() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 8).unwrap();

        // Exactly 18.
        assert!(validate_dob(NaiveDate::from_ymd_opt(2008, 5, 8).unwrap(), today).is_ok());
        // 17 years 364 days.
        let err = validate_dob(NaiveDate::from_ymd_opt(2008, 5, 9).unwrap(), today).unwrap_err();
        assert!(matches!(err, AgeError::Underage));
        // Future DOB.
        let err = validate_dob(NaiveDate::from_ymd_opt(2030, 1, 1).unwrap(), today).unwrap_err();
        assert!(matches!(err, AgeError::FutureDob));
    }
}
