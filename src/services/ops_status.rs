use std::{fs, net::IpAddr, path::PathBuf, time::SystemTime};

use chrono::{DateTime, Duration, Utc};
use ipnet::IpNet;

#[derive(Clone, Debug)]
pub struct BackupStatus {
    pub status: &'static str,
    pub backup_dir: Option<String>,
    pub latest_backup_file: Option<String>,
    pub latest_backup_at: Option<DateTime<Utc>>,
    pub latest_backup_age_hours: Option<i64>,
    pub latest_verified_file: Option<String>,
    pub latest_verified_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct IngressStatus {
    pub status: &'static str,
    pub secure_cookies: bool,
    pub rate_limit_disabled: bool,
    pub trust_mode: &'static str,
    pub trusted_cidrs: Vec<String>,
    pub client_ip_headers: Vec<String>,
}

pub fn backup_status(settings: &crate::config::OpsSettings) -> BackupStatus {
    let Some(dir) = settings
        .backup_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    else {
        return BackupStatus {
            status: "unconfigured",
            backup_dir: None,
            latest_backup_file: None,
            latest_backup_at: None,
            latest_backup_age_hours: None,
            latest_verified_file: None,
            latest_verified_at: None,
        };
    };
    let backup_dir = Some(dir.to_string_lossy().to_string());
    let Ok(entries) = fs::read_dir(&dir) else {
        return BackupStatus {
            status: "missing",
            backup_dir,
            latest_backup_file: None,
            latest_backup_at: None,
            latest_backup_age_hours: None,
            latest_verified_file: None,
            latest_verified_at: None,
        };
    };

    let mut latest_backup: Option<(String, DateTime<Utc>, PathBuf)> = None;
    let mut latest_verified: Option<(String, DateTime<Utc>)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()).map(str::to_string)
        else {
            continue;
        };
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Some(modified) = metadata.modified().ok().and_then(system_time_to_utc) else {
            continue;
        };
        if is_backup_file(&name) {
            if latest_backup
                .as_ref()
                .is_none_or(|(_, current_modified, _)| modified > *current_modified)
            {
                latest_backup = Some((name, modified, path));
            }
        } else if let Some(marker_target) = name.strip_suffix(".verified") {
            if latest_verified
                .as_ref()
                .is_none_or(|(_, current_modified)| modified > *current_modified)
            {
                latest_verified = Some((marker_target.to_string(), modified));
            }
        }
    }

    let Some((latest_backup_file, latest_backup_at, latest_backup_path)) = latest_backup else {
        return BackupStatus {
            status: "missing",
            backup_dir,
            latest_backup_file: None,
            latest_backup_at: None,
            latest_backup_age_hours: None,
            latest_verified_file: latest_verified.as_ref().map(|(file, _)| file.clone()),
            latest_verified_at: latest_verified.as_ref().map(|(_, at)| *at),
        };
    };

    let latest_backup_age = Utc::now().signed_duration_since(latest_backup_at);
    let latest_backup_age_hours = Some(latest_backup_age.num_hours());
    let latest_backup_verified = marker_path(&latest_backup_path)
        .metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(system_time_to_utc);
    let latest_verified_file = latest_verified.as_ref().map(|(file, _)| file.clone());
    let latest_verified_at = latest_verified.as_ref().map(|(_, at)| *at);

    let status = if latest_backup_age > Duration::hours(settings.backup_max_age_hours as i64) {
        "stale"
    } else if latest_backup_verified.is_none() {
        "unverified"
    } else {
        "ok"
    };

    BackupStatus {
        status,
        backup_dir,
        latest_backup_file: Some(latest_backup_file),
        latest_backup_at: Some(latest_backup_at),
        latest_backup_age_hours,
        latest_verified_file,
        latest_verified_at,
    }
}

pub fn ingress_status(settings: &crate::config::ServerSettings) -> IngressStatus {
    let trusted_cidrs =
        settings.proxy.trusted_cidrs.iter().map(ToString::to_string).collect::<Vec<_>>();
    let client_ip_headers = settings.proxy.client_ip_headers.clone();
    let trust_mode = trust_mode(&settings.proxy.trusted_cidrs);
    let status = if !settings.secure_cookies
        || settings.rate_limit.disabled
        || settings.proxy.trusted_cidrs.is_empty()
        || settings.proxy.client_ip_headers.is_empty()
    {
        "risky"
    } else {
        "ok"
    };

    IngressStatus {
        status,
        secure_cookies: settings.secure_cookies,
        rate_limit_disabled: settings.rate_limit.disabled,
        trust_mode,
        trusted_cidrs,
        client_ip_headers,
    }
}

fn is_backup_file(name: &str) -> bool {
    (name.ends_with(".dump") || name.ends_with(".dump.gz"))
        && !name.ends_with(".dump.verified")
        && !name.ends_with(".dump.gz.verified")
}

fn marker_path(path: &std::path::Path) -> PathBuf {
    PathBuf::from(format!("{}.verified", path.to_string_lossy()))
}

fn system_time_to_utc(value: SystemTime) -> Option<DateTime<Utc>> {
    Some(DateTime::<Utc>::from(value))
}

fn trust_mode(cidrs: &[IpNet]) -> &'static str {
    if cidrs.iter().all(is_loopback_cidr) {
        "loopback_only"
    } else {
        "custom"
    }
}

fn is_loopback_cidr(cidr: &IpNet) -> bool {
    let ipv4 = "127.0.0.1".parse::<IpAddr>().unwrap();
    let ipv6 = "::1".parse::<IpAddr>().unwrap();
    cidr.contains(&ipv4) || cidr.contains(&ipv6)
}

#[cfg(test)]
mod tests {
    use super::{backup_status, ingress_status};
    use crate::config::{OpsSettings, ProxySettings, RateLimitSettings, ServerSettings};
    use std::{fs, path::PathBuf};
    use uuid::Uuid;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("viceden-backup-status-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn backup_status_is_unconfigured_without_dir() {
        let status = backup_status(&OpsSettings::default());
        assert_eq!(status.status, "unconfigured");
        assert!(status.latest_backup_file.is_none());
    }

    #[test]
    fn backup_status_reports_unverified_when_marker_missing() {
        let dir = temp_dir();
        let backup = dir.join("viceden-20260531.dump.gz");
        fs::write(&backup, b"fake").expect("backup");

        let status = backup_status(&OpsSettings {
            backup_dir: Some(dir.to_string_lossy().to_string()),
            backup_max_age_hours: 36,
        });
        assert_eq!(status.status, "unverified");
        assert_eq!(status.latest_backup_file.as_deref(), Some("viceden-20260531.dump.gz"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn backup_status_reports_ok_when_latest_backup_has_marker() {
        let dir = temp_dir();
        let backup = dir.join("viceden-20260531.dump.gz");
        let marker = dir.join("viceden-20260531.dump.gz.verified");
        fs::write(&backup, b"fake").expect("backup");
        fs::write(&marker, b"verified").expect("marker");

        let status = backup_status(&OpsSettings {
            backup_dir: Some(dir.to_string_lossy().to_string()),
            backup_max_age_hours: 36,
        });
        assert_eq!(status.status, "ok");
        assert_eq!(status.latest_verified_file.as_deref(), Some("viceden-20260531.dump.gz"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn backup_status_reports_stale_when_latest_backup_is_older_than_limit() {
        let dir = temp_dir();
        let backup = dir.join("viceden-20260531.dump.gz");
        let marker = dir.join("viceden-20260531.dump.gz.verified");
        fs::write(&backup, b"fake").expect("backup");
        fs::write(&marker, b"verified").expect("marker");

        let status = backup_status(&OpsSettings {
            backup_dir: Some(dir.to_string_lossy().to_string()),
            backup_max_age_hours: 0,
        });
        assert_eq!(status.status, "stale");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ingress_status_is_ok_with_secure_cookies_and_proxy_headers() {
        let status = ingress_status(&ServerSettings {
            bind: "127.0.0.1:3000".into(),
            max_request_body_bytes: 131072,
            request_timeout_seconds: 15,
            secure_cookies: true,
            proxy: ProxySettings::default(),
            rate_limit: RateLimitSettings::default(),
        });
        assert_eq!(status.status, "ok");
        assert_eq!(status.trust_mode, "loopback_only");
        assert!(status.client_ip_headers.iter().any(|value| value == "Cf-Connecting-Ip"));
    }

    #[test]
    fn ingress_status_is_risky_when_rate_limits_or_secure_cookies_are_off() {
        let status = ingress_status(&ServerSettings {
            bind: "127.0.0.1:3000".into(),
            max_request_body_bytes: 131072,
            request_timeout_seconds: 15,
            secure_cookies: false,
            proxy: ProxySettings::default(),
            rate_limit: RateLimitSettings { disabled: true },
        });
        assert_eq!(status.status, "risky");
    }
}
