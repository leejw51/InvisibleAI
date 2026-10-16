use causewaybay_shared::CaptureClient;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Choices {
    pub active: String,
    pub default: String,
    pub continuous: Vec<String>,
}
impl Choices {
    /// Never replace a saved device just because it is disconnected or missing.
    pub fn initialize(&mut self, clients: &[CaptureClient]) -> bool {
        if !self.active.is_empty() {
            return false;
        }
        self.active = if !self.default.is_empty() {
            self.default.clone()
        } else {
            clients
                .iter()
                .filter(|c| c.online && c.can_capture)
                .min_by_key(|c| (&c.name, &c.id))
                .map(|c| c.id.clone())
                .unwrap_or_default()
        };
        !self.active.is_empty()
    }
}
impl From<causewaybay_shared::CaptureChoices> for Choices {
    fn from(c: causewaybay_shared::CaptureChoices) -> Self {
        Self {
            active: c.active,
            default: c.default,
            continuous: c.continuous,
        }
    }
}
impl From<&Choices> for causewaybay_shared::CaptureChoices {
    fn from(c: &Choices) -> Self {
        Self {
            active: c.active.clone(),
            default: c.default.clone(),
            continuous: c.continuous.clone(),
        }
    }
}
pub fn key(owner: &str) -> String {
    format!("causewaybay.capture.v1:{}", owner.to_lowercase())
}
pub fn load(owner: &str) -> Choices {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(&key(owner)).ok().flatten())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
pub fn save(owner: &str, choices: &Choices) -> bool {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .is_some_and(|s| {
            s.set_item(&key(owner), &serde_json::to_string(choices).unwrap())
                .is_ok()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(id: &str, online: bool, can_capture: bool) -> CaptureClient {
        CaptureClient {
            id: id.into(),
            name: id.into(),
            online,
            can_capture,
            ..Default::default()
        }
    }
    #[test]
    fn first_device_is_stable_and_only_uses_accessible_online_clients() {
        let mut choice = Choices::default();
        assert!(!choice.initialize(&[node("private", true, false), node("offline", false, true)]));
        assert!(choice.initialize(&[node("B", true, true), node("A", true, true)]));
        assert_eq!(choice.active, "A");
        assert!(!choice.initialize(&[node("0", true, true)]));
        assert_eq!(choice.active, "A");
    }
    #[test]
    fn account_choices_round_trip_without_losing_fields() {
        let local = Choices {
            active: "laptop".into(),
            default: "desk".into(),
            continuous: vec!["desk".into(), "laptop".into()],
        };
        let shared = causewaybay_shared::CaptureChoices::from(&local);
        assert_eq!(shared.active, "laptop");
        assert_eq!(shared.default, "desk");
        assert_eq!(Choices::from(shared), local);
    }
    #[test]
    fn default_and_explicit_choices_survive_reload_and_disconnect() {
        let mut choice: Choices = serde_json::from_str(r#"{"default":"offline"}"#).unwrap();
        choice.initialize(&[node("online", true, true)]);
        assert_eq!(choice.active, "offline");
        choice.active = "other".into();
        choice.continuous = vec!["online".into()];
        let mut restored: Choices =
            serde_json::from_str(&serde_json::to_string(&choice).unwrap()).unwrap();
        restored.initialize(&[]);
        assert_eq!(restored, choice);
    }
    #[test]
    fn storage_is_scoped_to_wallet() {
        assert_ne!(key("0xabc"), key("0xdef"));
        assert_eq!(key("0xABC"), key("0xabc"));
        assert_ne!(key("local"), key("0xabc"));
    }
}
