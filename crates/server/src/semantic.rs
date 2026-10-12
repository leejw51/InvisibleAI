use anyhow::Result;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

/// Multilingual, so a question in English finds a note kept in Korean (and the
/// reverse). Vectors from another model are re-indexed, never compared.
#[cfg(feature = "semantic")]
pub const MODEL: fastembed::EmbeddingModel = fastembed::EmbeddingModel::ParaphraseMLMiniLML12V2;
/// The `embeddings.model` value written for [`MODEL`].
pub const MODEL_NAME: &str = "paraphrase-multilingual-MiniLM-L12-v2";

pub struct Semantic {
    pub status: Mutex<String>,
    #[cfg(feature = "semantic")]
    model: Mutex<Option<fastembed::TextEmbedding>>,
}
impl Semantic {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            status: Mutex::new("starting".into()),
            #[cfg(feature = "semantic")]
            model: Mutex::new(None),
        })
    }
    pub fn start(self: &Arc<Self>, cache: PathBuf, disabled: bool) {
        if disabled {
            *self.status.lock().unwrap() = "off".into();
            return;
        }
        let this = self.clone();
        std::thread::spawn(move || {
            #[cfg(feature = "semantic")]
            {
                *this.status.lock().unwrap() = "loading".into();
                match fastembed::TextEmbedding::try_new(
                    fastembed::InitOptions::new(MODEL)
                        .with_cache_dir(cache)
                        .with_show_download_progress(true),
                ) {
                    Ok(model) => {
                        *this.model.lock().unwrap() = Some(model);
                        *this.status.lock().unwrap() = "ready".into();
                    }
                    Err(e) => {
                        tracing::warn!("Semantic model unavailable: {e}");
                        *this.status.lock().unwrap() = "unavailable".into();
                    }
                }
            }
            #[cfg(not(feature = "semantic"))]
            {
                let _ = cache;
                *this.status.lock().unwrap() = "not compiled".into();
            }
        });
    }
    pub fn ready(&self) -> bool {
        *self.status.lock().unwrap() == "ready"
    }
    pub fn embed(&self, text: &str) -> Result<Option<Vec<f32>>> {
        #[cfg(feature = "semantic")]
        {
            if !self.ready() {
                return Ok(None);
            }
            let mut guard = self.model.lock().unwrap();
            if let Some(model) = guard.as_mut() {
                return Ok(model.embed(vec![text], None)?.into_iter().next());
            }
        }
        let _ = text;
        Ok(None)
    }
}
