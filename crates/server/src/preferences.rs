//! Backend defaults: JSONL is the durable configuration; SQLite is its query mirror.
use crate::AppState;
use anyhow::{Result, ensure};
use causewaybay_shared::{SettingsUpdate, config};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    provider: String,
    #[serde(default)]
    default_ai: Option<causewaybay_shared::UserAiPreferences>,
    #[serde(default)]
    local_ai: causewaybay_shared::LocalAiSettings,
    models: BTreeMap<String, String>,
    input_mode: String,
    capture_mode: String,
    display: u32,
    #[serde(default, skip_serializing)]
    draft: String,
}
impl Preferences {
    fn validate(&self) -> Result<()> {
        crate::local_ai::validate(&self.local_ai)?;
        if let Some(default) = &self.default_ai {
            crate::ai_preferences::validate(default)?;
            ensure!(
                !self.local_ai.local_only || default.mode == "ondevice",
                "Local-only mode requires an On-device default"
            );
        }
        ensure!(
            !self.local_ai.local_only || crate::local_ai::is_local(&self.provider),
            "Choose Apple local or MLX local before enabling Local-only mode"
        );
        ensure!(
            crate::providers::PROVIDERS
                .iter()
                .any(|p| p.0 == self.provider),
            "Unknown provider"
        );
        ensure!(
            matches!(self.input_mode.as_str(), "image" | "text"),
            "Unknown input mode"
        );
        ensure!(
            matches!(self.capture_mode.as_str(), "screen" | "region"),
            "Unknown capture mode"
        );
        ensure!(
            (1..=16).contains(&self.display),
            "Display must be between 1 and 16"
        );
        ensure!(self.draft.len() <= 100_000, "Draft too long");
        for (id, model) in &self.models {
            ensure!(
                crate::providers::PROVIDERS.iter().any(|p| p.0 == id),
                "Unknown provider"
            );
            ensure!(
                crate::local_ai::valid_model(id, model),
                "Use a valid provider model ID"
            );
        }
        Ok(())
    }
    fn apply(&self, s: &AppState) -> Result<()> {
        let mut conn = s.db.0.lock().unwrap();
        let tx = conn.transaction()?;
        let mut values = self
            .models
            .iter()
            .map(|(p, m)| (format!("model.{p}"), m.clone()))
            .collect::<Vec<_>>();
        values.push(("local_ai".into(), serde_json::to_string(&self.local_ai)?));
        if let Some(default) = &self.default_ai {
            values.push(("default_ai".into(), serde_json::to_string(default)?));
        } else {
            tx.execute("DELETE FROM settings WHERE key='default_ai'", [])?;
        }
        values.extend([
            ("provider".into(), self.provider.clone()),
            ("input_mode".into(), self.input_mode.clone()),
            ("capture_mode".into(), self.capture_mode.clone()),
            ("display".into(), self.display.to_string()),
        ]);
        for (key, value) in values {
            tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,value])?;
        }
        tx.commit()?;
        Ok(())
    }
}
pub fn restore(s: &AppState) -> Result<()> {
    let value = config::read(&s.config_dir, "settings")?;
    if !value.is_null() {
        let prefs: Preferences = serde_json::from_value(value)?;
        prefs.validate()?;
        prefs.apply(s)?;
    }
    Ok(())
}
pub fn update(s: &AppState, v: SettingsUpdate) -> Result<()> {
    ensure!(
        v.draft.as_deref().is_none_or(str::is_empty),
        "Drafts are saved in this browser, not in backend settings"
    );
    let _guard = s.settings_write.lock().unwrap();
    let old = s.settings()?;
    let mut prefs = Preferences {
        provider: v.provider.unwrap_or(old.provider),
        default_ai: v.default_ai.or(old.default_ai.clone()),
        local_ai: v.local_ai.unwrap_or(old.local_ai.clone()),
        models: old.providers.into_iter().map(|p| (p.id, p.model)).collect(),
        input_mode: v.input_mode.unwrap_or(old.input_mode),
        capture_mode: v.capture_mode.unwrap_or(old.capture_mode),
        display: v.display.unwrap_or(old.display),
        draft: v.draft.unwrap_or(old.draft),
    };
    if let Some(model) = v.model {
        prefs.models.insert(prefs.provider.clone(), model);
    }
    prefs.validate()?;
    config::append(&s.config_dir, "settings", serde_json::to_value(&prefs)?)?;
    prefs.apply(s)?;
    if prefs.default_ai != old.default_ai || prefs.local_ai != old.local_ai {
        crate::notes::invalidate_all_approvals(s);
    }
    Ok(())
}
