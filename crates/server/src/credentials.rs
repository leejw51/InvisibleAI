use crate::providers::PROVIDERS;
use std::collections::HashMap;

/// Backend-owned startup snapshot. Deliberately not serializable or Debug.
#[derive(Default)]
pub struct Credentials(HashMap<String, String>);

impl Credentials {
    pub fn from_environment() -> Self {
        Self::from_values(
            PROVIDERS
                .iter()
                .map(|(_, _, _, env)| ((*env).to_owned(), std::env::var(env).unwrap_or_default())),
        )
    }

    pub fn from_values(values: impl IntoIterator<Item = (String, String)>) -> Self {
        let values: HashMap<_, _> = values.into_iter().collect();
        Self(
            PROVIDERS
                .iter()
                .filter_map(|(id, _, _, env)| {
                    values
                        .get(*env)
                        .map(|key| ((*id).into(), key.trim().into()))
                })
                .collect(),
        )
    }

    pub fn key(&self, provider: &str) -> &str {
        self.0.get(provider).map(String::as_str).unwrap_or_default()
    }

    pub fn configured(&self, provider: &str) -> bool {
        !self.key(provider).is_empty()
    }
}
