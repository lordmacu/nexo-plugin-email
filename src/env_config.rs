//! Phase 81.19.b — env-driven boot for the standalone email subprocess
//! plugin. Mirror of `nexo-plugin-telegram::env_config` but adapted to
//! email's single-process / multi-account internal model.
//!
//! The daemon-side helper `seed_email_subprocess_env_for` (in
//! `proyecto/src/main.rs`) populates four env vars before spawn:
//!
//!   * `NEXO_BROKER_URL`                       — broker URL (NATS or local)
//!   * `NEXO_PLUGIN_EMAIL_CONFIG_PATH`         — absolute path to `email.yaml`
//!   * `NEXO_PLUGIN_EMAIL_GOOGLE_AUTH_PATH`    — absolute path to `google-auth.yaml`
//!                                                (empty string if Gmail OAuth is unused)
//!   * `NEXO_PLUGIN_EMAIL_SECRETS_DIR`         — absolute path to `secrets/`
//!   * `NEXO_PLUGIN_EMAIL_DATA_DIR`            — absolute path for SQLite stores
//!
//! `email_config_from_env` re-loads YAML from disk on every boot. This
//! keeps the source-of-truth where it belongs (config files), avoids
//! adding `Serialize` derives to `EmailPluginConfig`/`GoogleAuthConfig`,
//! and matches how the daemon itself reads these files.
//!
//! Failure modes:
//!   * Missing env var          → [`EnvConfigError::Missing`]
//!   * Unreadable YAML path     → [`EnvConfigError::ReadConfig`]
//!   * Malformed YAML           → [`EnvConfigError::ParseConfig`]
//!   * Email secrets bundle has fatal `BuildError`s → [`EnvConfigError::Creds`]
//!
//! Non-fatal warnings from `load_email_secrets` are logged at WARN
//! level and discarded — they should not gate boot.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nexo_auth::email::{load_email_secrets, EmailCredentialStore};
use nexo_auth::google::{GoogleAccount, GoogleCredentialStore};
use crate::config::{EmailPluginConfig, EmailPluginConfigFile};

const ENV_CONFIG_PATH: &str = "NEXO_PLUGIN_EMAIL_CONFIG_PATH";
const ENV_GOOGLE_AUTH_PATH: &str = "NEXO_PLUGIN_EMAIL_GOOGLE_AUTH_PATH";
const ENV_SECRETS_DIR: &str = "NEXO_PLUGIN_EMAIL_SECRETS_DIR";
const ENV_DATA_DIR: &str = "NEXO_PLUGIN_EMAIL_DATA_DIR";

/// Bundle returned by [`email_config_from_env`]. The four fields map
/// 1:1 to `EmailPlugin::new(cfg, creds, google, data_dir)`.
#[derive(Debug)]
pub struct EmailSubprocessBoot {
    pub cfg: EmailPluginConfig,
    pub creds: Arc<EmailCredentialStore>,
    pub google: Arc<GoogleCredentialStore>,
    pub data_dir: PathBuf,
}

#[derive(thiserror::Error, Debug)]
pub enum EnvConfigError {
    #[error("required env var not set: {0}")]
    Missing(&'static str),
    #[error("cannot read config at {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid YAML in {path}: {source}")]
    ParseConfig {
        path: PathBuf,
        source: serde_yaml::Error,
    },
    #[error("env placeholder resolution in {path}: {source}")]
    PlaceholderResolution {
        path: PathBuf,
        source: anyhow::Error,
    },
    #[error("google auth load: {0}")]
    GoogleAuth(#[from] anyhow::Error),
    #[error("fatal credential errors loading email secrets: {0}")]
    Creds(String),
}

/// Read all four env vars and rebuild the constructor inputs the
/// subprocess needs to instantiate an `EmailPlugin`. Pure function over
/// the environment + filesystem; safe to call multiple times.
pub fn email_config_from_env() -> Result<EmailSubprocessBoot, EnvConfigError> {
    let cfg_path = required_env(ENV_CONFIG_PATH)?;
    let secrets_dir = required_env(ENV_SECRETS_DIR)?;
    let data_dir = required_env(ENV_DATA_DIR)?;
    // Google auth path is optional — empty string means "no Gmail OAuth
    // accounts". An empty store still satisfies `EmailPlugin::new`.
    let google_path = std::env::var(ENV_GOOGLE_AUTH_PATH).unwrap_or_default();

    let cfg = load_email_yaml(Path::new(&cfg_path))?;
    let creds = build_email_store(&cfg, Path::new(&secrets_dir))?;
    let google = build_google_store(google_path.as_str())?;

    Ok(EmailSubprocessBoot {
        cfg,
        creds: Arc::new(creds),
        google: Arc::new(google),
        data_dir: PathBuf::from(data_dir),
    })
}

fn required_env(key: &'static str) -> Result<String, EnvConfigError> {
    std::env::var(key).map_err(|_| EnvConfigError::Missing(key))
}

fn load_email_yaml(path: &Path) -> Result<EmailPluginConfig, EnvConfigError> {
    let raw = std::fs::read_to_string(path).map_err(|source| EnvConfigError::ReadConfig {
        path: path.to_path_buf(),
        source,
    })?;
    let resolved = nexo_config::env::resolve_placeholders(
        &raw,
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("email.yaml"),
    )
    .map_err(|source| EnvConfigError::PlaceholderResolution {
        path: path.to_path_buf(),
        source,
    })?;
    let file: EmailPluginConfigFile =
        serde_yaml::from_str(&resolved).map_err(|source| EnvConfigError::ParseConfig {
            path: path.to_path_buf(),
            source,
        })?;
    // 0.5.0: top-level `email` is `EmailPluginShape`. Pick the
    // first entry — the env-var bootstrap path is single-instance
    // by construction (the daemon would seed per-instance env vars
    // when multi-tenant is wired daemon-side).
    Ok(file
        .email
        .into_vec()
        .into_iter()
        .next()
        .ok_or_else(|| EnvConfigError::ParseConfig {
            path: path.to_path_buf(),
            source: serde::de::Error::custom(
                "email plugin config has no entries (empty sequence)",
            ),
        })?)
}

fn build_email_store(
    cfg: &EmailPluginConfig,
    secrets_dir: &Path,
) -> Result<EmailCredentialStore, EnvConfigError> {
    // Mirror of `nexo_auth::wire::build_credentials` for the email-only
    // subset: `(instance, address)` pairs feed `load_email_secrets`.
    let declared: Vec<(String, String)> = cfg
        .accounts
        .iter()
        .map(|a| (a.instance.clone(), a.address.clone()))
        .collect();
    let (accounts, warnings, errors) = load_email_secrets(secrets_dir, &declared);
    for w in warnings {
        tracing::warn!(target: "nexo_plugin_email::env_config", "{}", w);
    }
    if !errors.is_empty() {
        return Err(EnvConfigError::Creds(
            errors
                .into_iter()
                .map(|e| format!("{e}"))
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    Ok(EmailCredentialStore::new(accounts))
}

fn build_google_store(path: &str) -> Result<GoogleCredentialStore, EnvConfigError> {
    if path.is_empty() {
        return Ok(GoogleCredentialStore::empty());
    }
    // The daemon side already validates the file; here we only re-load
    // it. Reuse `nexo_auth::wire::load_google_auth` by giving it the
    // *parent of the parent* of the file (it appends `plugins/google-auth.yaml`).
    let p = PathBuf::from(path);
    // Subprocess gets the canonical config dir as an env var when the
    // daemon points to a custom location. To avoid coupling the
    // subprocess to internal directory layout, accept either a full
    // `google-auth.yaml` path or an `<config_dir>` parent.
    let auth = if p.is_file() {
        load_google_auth_from_file(&p)?
    } else {
        nexo_auth::load_google_auth(&p).map_err(EnvConfigError::GoogleAuth)?
    };
    let accounts: Vec<GoogleAccount> = auth
        .accounts
        .into_iter()
        .map(|c| GoogleAccount {
            id: c.id,
            agent_id: c.agent_id,
            client_id_path: c.client_id_path,
            client_secret_path: c.client_secret_path,
            token_path: c.token_path,
            scopes: c.scopes,
        })
        .collect();
    Ok(GoogleCredentialStore::new(accounts))
}

/// Direct file path variant — handy when the operator mounts the
/// google-auth.yaml at a non-standard location (Docker secrets, k8s
/// projected volumes). Bypasses the `<dir>/plugins/google-auth.yaml`
/// search that `nexo_auth::load_google_auth` uses.
fn load_google_auth_from_file(
    path: &Path,
) -> Result<nexo_config::types::credentials::GoogleAuthConfig, EnvConfigError> {
    use nexo_config::types::credentials::GoogleAuthConfig;
    let raw = std::fs::read_to_string(path).map_err(|source| EnvConfigError::ReadConfig {
        path: path.to_path_buf(),
        source,
    })?;
    let resolved = nexo_config::env::resolve_placeholders(
        &raw,
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("google-auth.yaml"),
    )
    .map_err(|source| EnvConfigError::PlaceholderResolution {
        path: path.to_path_buf(),
        source,
    })?;
    #[derive(serde::Deserialize)]
    struct GoogleAuthFile {
        google_auth: GoogleAuthConfig,
    }
    let file: GoogleAuthFile =
        serde_yaml::from_str(&resolved).map_err(|source| EnvConfigError::ParseConfig {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(file.google_auth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_env_returns_named_var() {
        // SAFETY: tests run sequentially per-binary so env mutation here
        // is fine; serial_test marker not needed because we only check
        // a single absence path.
        std::env::remove_var(ENV_CONFIG_PATH);
        let err = email_config_from_env().unwrap_err();
        assert!(matches!(err, EnvConfigError::Missing(ENV_CONFIG_PATH)));
    }
}
