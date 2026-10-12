//! Wallet-owned execution policy, persisted centrally and checked on every request.
use crate::{Api, AppState, PrivateState};
use axum::Json;
use causewaybay_shared::UserAiPreferences;

fn key(s: &AppState) -> String {
    format!("user.ai.{}", s.db.2.as_deref().unwrap_or("local"))
}
pub fn read(s: &AppState) -> anyhow::Result<Option<UserAiPreferences>> {
    let saved: Option<UserAiPreferences> =
        s.db.get_setting(&key(s))?
            .or(s.db.get_setting("default_ai")?)
            .map(|v| serde_json::from_str(&v))
            .transpose()?;
    let local = s.local_ai()?;
    if !local.local_only {
        // Without a saved choice, the shared computed default (Hybrid when a local
        // and a cloud provider are both ready) applies on every device and job.
        return Ok(Some(saved.unwrap_or(computed_default(s)?)));
    }
    // Expose the effective policy without destroying the user's saved choice.
    // Turning off the workspace restriction restores that choice on every device.
    let default = default_policy(s)?;
    let selected = s.db.get_setting("provider")?;
    let enabled = |id: &&str| match *id {
        "apple" => local.apple_enabled,
        "mlx" => local.mlx_enabled,
        _ => false,
    };
    let provider = saved
        .as_ref()
        .map(|p| p.local_provider.as_str())
        .into_iter()
        .chain(default.as_ref().map(|p| p.local_provider.as_str()))
        .chain(selected.as_deref())
        .chain(["apple", "mlx"])
        .find(enabled)
        .unwrap_or("apple")
        .to_owned();
    let mut effective = saved.unwrap_or(UserAiPreferences {
        mode: "ondevice".into(),
        local_provider: provider.clone(),
        cloud_provider: "grok".into(),
    });
    effective.mode = "ondevice".into();
    effective.local_provider = provider;
    Ok(Some(effective))
}
pub fn computed_default(s: &AppState) -> anyhow::Result<UserAiPreferences> {
    // Codex and Claude Code count as ready only when chosen as the mode, so an
    // installed tool never turns the computed default into Hybrid-with-codex.
    let providers = crate::providers::PROVIDERS
        .iter()
        .map(|p| {
            (
                p.0.to_owned(),
                !crate::cli_ai::is_cli(p.0) && s.provider_configured(p.0),
            )
        })
        .collect::<Vec<_>>();
    Ok(UserAiPreferences::computed_default(
        s.local_ai()?.local_only,
        &s.selected_provider()?,
        &providers,
    ))
}
pub fn default_policy(s: &AppState) -> anyhow::Result<Option<UserAiPreferences>> {
    s.db.get_setting("default_ai")?
        .map(|v| serde_json::from_str(&v).map_err(Into::into))
        .transpose()
}
pub async fn get(PrivateState(s): PrivateState) -> Api<Json<Option<UserAiPreferences>>> {
    Ok(Json(read(&s)?))
}
pub async fn update(
    PrivateState(s): PrivateState,
    Json(v): Json<UserAiPreferences>,
) -> Api<Json<UserAiPreferences>> {
    validate(&v)?;
    if s.local_ai()?.local_only && v.mode != "ondevice" {
        return Err(
            anyhow::anyhow!("The admin has restricted this backend to On-device mode").into(),
        );
    }
    if !s.demo && !s.provider_configured(v.provider()) {
        return Err(anyhow::anyhow!("Ask the admin to enable this AI provider first").into());
    }
    s.db.set_setting(
        &key(&s),
        &serde_json::to_string(&v).map_err(anyhow::Error::from)?,
    )?;
    // Existing Auto jobs read the policy again before each inference and write.
    crate::notes::invalidate_approvals(&s);
    Ok(Json(v))
}
pub fn validate(v: &UserAiPreferences) -> anyhow::Result<()> {
    if !matches!(
        v.mode.as_str(),
        "ondevice" | "hybrid" | "cloudai" | "codex" | "claude_code"
    ) || !crate::local_ai::is_local(&v.local_provider)
        || !crate::providers::PROVIDERS
            .iter()
            .any(|p| p.0 == v.cloud_provider && !crate::local_ai::is_local(p.0))
    {
        anyhow::bail!(
            "Choose On-device, Hybrid, Cloud AI, Codex, or Claude Code and valid providers"
        );
    }
    Ok(())
}
pub fn allow(s: &AppState, provider: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !s.local_ai()?.local_only || crate::local_ai::is_local(provider),
        "Cloud inference is disabled in Local-only mode"
    );
    if let Some(prefs) = read(s)? {
        anyhow::ensure!(
            prefs.mode == "cloudai"
                || crate::local_ai::is_local(provider)
                || (crate::cli_ai::is_cli(provider) && prefs.mode == provider),
            "This user requires local processing. Hybrid cloud help needs explicit approval."
        );
    }
    Ok(())
}

/// Model names belong to the account, independently of the provider policy.
pub fn models(s: &AppState) -> anyhow::Result<std::collections::BTreeMap<String, String>> {
    s.db.get_setting(&format!(
        "user.models.{}",
        s.db.2.as_deref().unwrap_or("local")
    ))?
    .map(|v| serde_json::from_str(&v).map_err(Into::into))
    .unwrap_or_else(|| Ok(Default::default()))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelChoice {
    provider: String,
    model: String,
}
pub async fn update_model(
    PrivateState(s): PrivateState,
    Json(v): Json<ModelChoice>,
) -> Api<Json<serde_json::Value>> {
    if !(crate::providers::PROVIDERS
        .iter()
        .any(|p| p.0 == v.provider)
        && (!crate::local_ai::is_local(&v.provider)
            || (v.provider == "mlx"
                && matches!(
                    v.model.as_str(),
                    "mlx-community/Qwen3.5-9B-4bit" | "mlx-community/Qwen3.8-27B-4bit"
                ))))
    {
        return Err(anyhow::anyhow!("Choose a supported AI model").into());
    }
    if v.model.len() > 160 || !crate::local_ai::valid_model(&v.provider, &v.model) {
        return Err(anyhow::anyhow!("Invalid model name").into());
    }
    let _guard = s.settings_write.lock().unwrap();
    let mut saved = models(&s)?;
    saved.insert(v.provider, v.model);
    s.db.set_setting(
        &format!("user.models.{}", s.db.2.as_deref().unwrap_or("local")),
        &serde_json::to_string(&saved).map_err(anyhow::Error::from)?,
    )?;
    Ok(Json(serde_json::json!({"ok":true})))
}

#[cfg(test)]
mod model_tests {
    use super::*;
    #[tokio::test]
    async fn model_choice_is_account_scoped_and_used_by_backend_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open_configured(
            dir.path().join("data"),
            dir.path().join("config"),
            true,
            None,
        )
        .unwrap();
        let a = state.for_owner("alice");
        let b = state.for_owner("bob");
        let _ = update_model(
            PrivateState(a.clone()),
            Json(ModelChoice {
                provider: "grok".into(),
                model: "my-model".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(a.settings().unwrap().user_models["grok"], "my-model");
        assert_eq!(a.provider_config_for("grok").unwrap().model, "my-model");
        assert!(b.settings().unwrap().user_models.is_empty());
        for model in [
            "mlx-community/Qwen3.5-9B-4bit",
            "mlx-community/Qwen3.8-27B-4bit",
        ] {
            let _ = update_model(
                PrivateState(a.clone()),
                Json(ModelChoice {
                    provider: "mlx".into(),
                    model: model.into(),
                }),
            )
            .await
            .unwrap();
            assert_eq!(a.provider_config_for("mlx").unwrap().model, model);
            assert_eq!(a.settings().unwrap().user_models["mlx"], model);
            assert!(!b.settings().unwrap().user_models.contains_key("mlx"));
        }

        assert!(
            update_model(
                PrivateState(a),
                Json(ModelChoice {
                    provider: "apple".into(),
                    model: "other".into()
                })
            )
            .await
            .is_err()
        );
    }
}
