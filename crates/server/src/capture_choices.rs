//! The capture computer each account picked, kept on the backend so a new
//! login or browser does not ask again.
use crate::{Api, AppState, PrivateState};
use axum::Json;
use causewaybay_shared::CaptureChoices;

fn key(s: &AppState) -> String {
    format!("user.capture.{}", s.db.2.as_deref().unwrap_or("local"))
}
pub fn read(s: &AppState) -> anyhow::Result<Option<CaptureChoices>> {
    s.db.get_setting(&key(s))?
        .map(|v| serde_json::from_str(&v).map_err(Into::into))
        .transpose()
}
fn valid_id(id: &str) -> bool {
    id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}
pub fn validate(v: &CaptureChoices) -> anyhow::Result<()> {
    anyhow::ensure!(
        v.continuous.len() <= 64
            && [&v.active, &v.default]
                .into_iter()
                .chain(&v.continuous)
                .all(|id| valid_id(id)),
        "Choose a valid capture device"
    );
    Ok(())
}
pub async fn get(PrivateState(s): PrivateState) -> Api<Json<Option<CaptureChoices>>> {
    Ok(Json(read(&s)?))
}
pub async fn update(
    PrivateState(s): PrivateState,
    Json(v): Json<CaptureChoices>,
) -> Api<Json<CaptureChoices>> {
    validate(&v)?;
    s.db.set_setting(
        &key(&s),
        &serde_json::to_string(&v).map_err(anyhow::Error::from)?,
    )?;
    Ok(Json(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn choices_are_account_scoped_and_validated() {
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
        let choice = CaptureChoices {
            active: "mac-1".into(),
            default: "mac-1".into(),
            continuous: vec!["mac-2".into()],
        };
        let Json(saved) = update(PrivateState(a.clone()), Json(choice.clone()))
            .await
            .unwrap();
        assert_eq!(saved, choice);
        assert_eq!(a.settings().unwrap().user_capture, Some(choice));
        assert_eq!(b.settings().unwrap().user_capture, None);
        let bad = CaptureChoices {
            active: "bad id; drop".into(),
            ..Default::default()
        };
        assert!(update(PrivateState(b.clone()), Json(bad)).await.is_err());
        assert_eq!(b.settings().unwrap().user_capture, None);
    }
}
