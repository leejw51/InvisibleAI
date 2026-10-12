use crate::{Api, AppState, PrivateState, providers};
use axum::Json;
use causewaybay_shared::AiInfo;

pub fn endpoint(provider: &str) -> &'static str {
    match provider {
        "apple" => "on-device:FoundationModels",
        "mlx" => "on-device:MLX",
        "anthropic" => "https://api.anthropic.com/v1/messages",
        "grok" => "https://api.x.ai/v1/chat/completions",
        "openai" => "https://api.openai.com/v1/chat/completions",
        "gemini" => "https://generativelanguage.googleapis.com/v1beta",
        _ => "",
    }
}
fn endpoint_for(c: &providers::Config) -> String {
    if c.provider == "mlx" && c.local.mlx_engine == "service" {
        return "backend-loopback:MLX".into();
    }
    endpoint(&c.provider).into()
}
fn info(s: &AppState) -> anyhow::Result<AiInfo> {
    let c = s.provider_config()?;
    Ok(AiInfo {
        endpoint: endpoint_for(&c),
        configured: s.provider_configured(&c.provider),
        audio_provider: crate::media::audio_provider(s),
        provider: c.provider,
        model: c.model,
        demo: s.demo,
        status: if s.demo {
            "Demo mode — no cloud call"
        } else {
            "Not checked"
        }
        .into(),
    })
}
pub async fn get(PrivateState(s): PrivateState) -> Api<Json<AiInfo>> {
    Ok(Json(info(&s)?))
}
pub async fn check(
    PrivateState(s): PrivateState,
    body: Option<Json<serde_json::Value>>,
) -> Api<Json<AiInfo>> {
    let mut result = info(&s)?;
    let body = body.map(|v| v.0).unwrap_or_default();
    let mut config = if let Some(provider) = body["provider"].as_str() {
        s.provider_config_for(provider)?
    } else {
        s.provider_config()?
    };
    crate::ai_preferences::allow(&s, &config.provider)?;
    if let Some(model) = body["model"].as_str() {
        if config.provider == "mlx" && model != config.model {
            return Err(
                anyhow::anyhow!("MLX uses the model installed and selected by the admin").into(),
            );
        }
        if !crate::local_ai::valid_model(&config.provider, model) {
            return Err(anyhow::anyhow!("Invalid model name").into());
        }
        config.model = model.into();
    }
    result.provider = config.provider.clone();
    result.model = config.model.clone();
    result.endpoint = endpoint_for(&config);
    result.configured = s.provider_configured(&config.provider);
    if config.local.local_only && !crate::local_ai::is_local(&config.provider) {
        return Err(anyhow::anyhow!("Cloud inference is disabled in Local-only mode").into());
    }
    if s.demo {
        return Ok(Json(result));
    }
    // A tiny inference checks this exact model as well as account authentication.
    let messages = [providers::Message::User("Reply with OK only.".into())];
    let response = providers::stream(
        &config,
        "This is a connection test. Reply OK.",
        &messages,
        &[],
        |_| Ok(()),
    )
    .await;
    result.status = match response {
        Ok(_) => "Connected — model responded".into(),
        Err(e) => format!("Connection check failed: {e}"),
    };
    Ok(Json(result))
}
