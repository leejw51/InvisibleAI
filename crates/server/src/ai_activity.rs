//! Ephemeral, owner-scoped inference activity. Never stores request content.
use crate::{Api, PrivateState};
use axum::{
    http::HeaderMap,
    response::{
        Sse,
        sse::{Event, KeepAlive},
    },
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Debug, Serialize)]
pub struct Call {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub location: String,
}
#[derive(Debug)]
pub struct Tracker {
    calls: Mutex<BTreeMap<String, (String, Call)>>,
    revision: tokio::sync::broadcast::Sender<(String, Vec<Call>)>,
}
impl Default for Tracker {
    fn default() -> Self {
        Self {
            calls: Mutex::default(),
            revision: tokio::sync::broadcast::channel(128).0,
        }
    }
}
pub struct Guard {
    tracker: Arc<Tracker>,
    id: String,
    owner: String,
}
impl Tracker {
    pub fn begin(
        self: &Arc<Self>,
        owner: &str,
        provider: &str,
        model: &str,
        location: &str,
    ) -> Guard {
        let id = crate::db::id();
        let mut calls = self.calls.lock().unwrap();
        calls.insert(
            id.clone(),
            (
                owner.into(),
                Call {
                    id: id.clone(),
                    provider: provider.into(),
                    model: model.into(),
                    location: location.into(),
                },
            ),
        );
        let snapshot = calls
            .values()
            .filter(|(o, _)| o == owner)
            .map(|(_, c)| c.clone())
            .collect();
        let _ = self.revision.send((owner.into(), snapshot));
        drop(calls);
        Guard {
            tracker: self.clone(),
            id,
            owner: owner.into(),
        }
    }
    fn snapshot(&self, owner: &str) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap()
            .values()
            .filter(|(o, _)| o == owner)
            .map(|(_, c)| c.clone())
            .collect()
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let mut calls = self.tracker.calls.lock().unwrap();
        calls.remove(&self.id);
        let snapshot = calls
            .values()
            .filter(|(o, _)| o == &self.owner)
            .map(|(_, c)| c.clone())
            .collect();
        let _ = self.tracker.revision.send((self.owner.clone(), snapshot));
    }
}
pub async fn events(
    PrivateState(s): PrivateState,
    headers: HeaderMap,
) -> Api<impl axum::response::IntoResponse> {
    let mut revision = s.local_runtime.activity.revision.subscribe();
    let stream = async_stream::stream! {
        yield Ok::<_, Infallible>(Event::default().json_data(s.local_runtime.activity.snapshot(s.db.owner())).unwrap());
        loop {
            if crate::auth::authorize(&s, &headers, "GET", "/api/activity/events").is_err() { break; }
            let snapshot = tokio::select! {
                result = revision.recv() => { match result {
                    Ok((owner, calls)) if owner == s.db.owner() => calls,
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => s.local_runtime.activity.snapshot(s.db.owner()),
                    Err(_) => break,
                } }
                _ = tokio::time::sleep(Duration::from_secs(10)) => s.local_runtime.activity.snapshot(s.db.owner()),
            };
            yield Ok(Event::default().json_data(snapshot).unwrap());
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn brief_calls_emit_both_start_and_end_in_order() {
        let tracker = Arc::new(Tracker::default());
        let mut events = tracker.revision.subscribe();
        drop(tracker.begin("alice", "mlx", "9B", "local"));
        let (owner, start) = events.try_recv().unwrap();
        assert_eq!(owner, "alice");
        assert_eq!(start.len(), 1);
        let (owner, end) = events.try_recv().unwrap();
        assert_eq!(owner, "alice");
        assert!(end.is_empty());
    }
    #[test]
    fn scopes_concurrent_calls_and_clears_cancelled_requests() {
        let tracker = Arc::new(Tracker::default());
        let a = tracker.begin("alice", "mlx", "9B", "local");
        let b = tracker.begin("alice", "grok", "cloud", "cloud");
        let _c = tracker.begin("bob", "apple", "system", "local");
        assert_eq!(tracker.snapshot("alice").len(), 2);
        assert_eq!(tracker.snapshot("bob").len(), 1);
        assert!(tracker.snapshot("eve").is_empty());
        drop(a);
        assert_eq!(tracker.snapshot("alice")[0].location, "cloud");
        drop(b);
        assert!(tracker.snapshot("alice").is_empty());
    }
}
