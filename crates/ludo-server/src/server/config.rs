//! Typed environment configuration with fail-fast production validation.

use std::{env, net::SocketAddr, time::Duration};

#[derive(Clone, Debug)]
pub(super) struct FeatureFlags {
    pub(super) ranked: bool,
    pub(super) social: bool,
    pub(super) replays: bool,
}

#[derive(Clone, Debug)]
pub(super) struct ServerConfig {
    pub(super) database_url: String,
    pub(super) address: SocketAddr,
    pub(super) allowed_origins: Vec<String>,
    pub(super) max_db_connections: u32,
    pub(super) min_db_connections: u32,
    pub(super) db_acquire_timeout: Duration,
    pub(super) db_idle_timeout: Duration,
    pub(super) outbound_queue_capacity: usize,
    pub(super) admin_token: Option<String>,
    pub(super) protocol_version: u16,
    pub(super) flags: FeatureFlags,
}

impl ServerConfig {
    pub(super) fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let database_url = required("DATABASE_URL")?;
        let address = env::var("LUDO_SERVER_ADDR")
            .unwrap_or_else(|_| {
                format!(
                    "0.0.0.0:{}",
                    env::var("PORT").unwrap_or_else(|_| "8080".to_owned())
                )
            })
            .parse()?;
        let allowed_origins = env::var("LUDO_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.trim_end_matches('/').to_owned())
            .collect::<Vec<_>>();
        let production = env::var("LUDO_ENV").is_ok_and(|value| value == "production");
        if production && allowed_origins.is_empty() {
            return Err("LUDO_ALLOWED_ORIGINS is required in production".into());
        }
        let max_db_connections = parse("LUDO_DB_MAX_CONNECTIONS", 20_u32)?;
        let min_db_connections = parse("LUDO_DB_MIN_CONNECTIONS", 2_u32)?;
        if min_db_connections > max_db_connections {
            return Err("LUDO_DB_MIN_CONNECTIONS cannot exceed LUDO_DB_MAX_CONNECTIONS".into());
        }
        let outbound_queue_capacity = parse("LUDO_OUTBOUND_QUEUE_CAPACITY", 256_usize)?;
        if outbound_queue_capacity < 32 {
            return Err("LUDO_OUTBOUND_QUEUE_CAPACITY must be at least 32".into());
        }
        Ok(Self {
            database_url,
            address,
            allowed_origins,
            max_db_connections,
            min_db_connections,
            db_acquire_timeout: Duration::from_secs(parse(
                "LUDO_DB_ACQUIRE_TIMEOUT_SECONDS",
                5_u64,
            )?),
            db_idle_timeout: Duration::from_secs(parse("LUDO_DB_IDLE_TIMEOUT_SECONDS", 300_u64)?),
            outbound_queue_capacity,
            admin_token: env::var("LUDO_ADMIN_TOKEN")
                .ok()
                .filter(|value| !value.is_empty()),
            protocol_version: 1,
            flags: FeatureFlags {
                ranked: flag("LUDO_FEATURE_RANKED", true),
                social: flag("LUDO_FEATURE_SOCIAL", true),
                replays: flag("LUDO_FEATURE_REPLAYS", true),
            },
        })
    }
}

fn required(key: &str) -> Result<String, Box<dyn std::error::Error>> {
    env::var(key)
        .map_err(|_| format!("{key} is required").into())
        .and_then(|value| {
            if value.trim().is_empty() {
                Err(format!("{key} cannot be empty").into())
            } else {
                Ok(value)
            }
        })
}

fn parse<T>(key: &str, default: T) -> Result<T, Box<dyn std::error::Error>>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + 'static,
{
    env::var(key).map_or(Ok(default), |value| value.parse().map_err(Into::into))
}

fn flag(key: &str, default: bool) -> bool {
    flag_value(env::var(key).ok().as_deref(), default)
}

fn flag_value(value: Option<&str>, default: bool) -> bool {
    value.map_or(default, |value| {
        matches!(value, "1" | "true" | "yes" | "on")
    })
}

#[cfg(test)]
mod tests {
    use super::{flag, flag_value};

    #[test]
    fn feature_flag_default_is_stable() {
        assert!(flag("LUDO_TEST_FLAG_THAT_DOES_NOT_EXIST", true));
    }

    #[test]
    fn feature_flag_accepts_documented_enabled_values() {
        for value in ["1", "true", "yes", "on"] {
            assert!(flag_value(Some(value), false), "{value} should enable");
        }
    }

    #[test]
    fn feature_flag_rejects_other_values() {
        for value in ["0", "false", "off", "TRUE", "", "enabled"] {
            assert!(!flag_value(Some(value), true), "{value} should disable");
        }
    }

    #[test]
    fn feature_flag_uses_the_requested_default_when_absent() {
        assert!(flag_value(None, true));
        assert!(!flag_value(None, false));
    }
}
