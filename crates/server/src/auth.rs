//! Mnemonics never reach this module. Verify one-use EIP-191 challenges instead.
use crate::{Api, ApiError, AppState, constant_eq};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use causewaybay_shared::identity::{normalize_address, recover};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub struct Auth {
    pub admin: Option<String>,
    challenges: Mutex<HashMap<String, Challenge>>,
}
struct Challenge {
    address: String,
    message: String,
    expires: i64,
    host: String,
}
#[derive(Clone)]
pub struct Principal {
    pub address: Option<String>,
    pub admin: bool,
    pub node: bool,
    pub wallet_index: u32,
}
impl Auth {
    pub fn new(admin: Option<String>) -> anyhow::Result<Self> {
        Ok(Self {
            admin: admin
                .filter(|v| !v.trim().is_empty())
                .map(|v| normalize_address(v.trim()))
                .transpose()
                .map_err(anyhow::Error::msg)?,
            challenges: Mutex::new(HashMap::new()),
        })
    }
}
fn unauthorized() -> ApiError {
    ApiError(StatusCode::UNAUTHORIZED, "Sign in to your workspace".into())
}
fn stamp() -> i64 {
    chrono::Utc::now().timestamp()
}
fn random() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
pub(crate) fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
fn cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|v| v.trim().strip_prefix("cwb_session="))
}
pub fn same_origin(headers: &HeaderMap) -> Api<()> {
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if origin != format!("http://{host}") && origin != format!("https://{host}") {
            return Err(ApiError(
                StatusCode::FORBIDDEN,
                "Cross-origin requests are not allowed".into(),
            ));
        }
    }
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site")
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Cross-site requests are not allowed".into(),
        ));
    }
    Ok(())
}
pub fn principal(s: &AppState, headers: &HeaderMap) -> Api<Principal> {
    if let Some(token) = cookie(headers) {
        let row: Option<(Option<String>, i64, u32)> =
            s.db.0
                .lock()
                .unwrap()
                .query_row(
                    "SELECT address,expires,wallet_index FROM auth_sessions WHERE token_hash=?1",
                    [hash(token)],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
        if let Some((address, expires, wallet_index)) =
            row.filter(|(_, expires, _)| *expires > stamp())
        {
            let _ = expires;
            let admin = match &address {
                Some(a) => crate::users::role(s, a)? == "admin",
                None => s.auth.admin.is_none(),
            };
            return Ok(Principal {
                address,
                admin,
                node: false,
                wallet_index,
            });
        }
    }
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if s.token
        .as_ref()
        .is_some_and(|token| constant_eq(bearer.unwrap_or(""), token))
    {
        return Ok(Principal {
            address: None,
            admin: s.auth.admin.is_none(),
            node: s.auth.admin.is_some(),
            wallet_index: 0,
        });
    }
    // Retain the original local/demo mode until wallet access is configured.
    if s.auth.admin.is_none() && s.token.is_none() {
        // A matching Origin/Host alone does not prevent DNS rebinding.
        if let Some(host) = headers.get(header::HOST) {
            let host = host.to_str().unwrap_or("");
            let authority = host
                .parse::<axum::http::uri::Authority>()
                .map_err(|_| unauthorized())?;
            let name = authority
                .host()
                .trim_start_matches('[')
                .trim_end_matches(']');
            if name != "localhost"
                && !name
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
            {
                return Err(unauthorized());
            }
        }
        return Ok(Principal {
            address: None,
            admin: true,
            node: false,
            wallet_index: 0,
        });
    }
    Err(unauthorized())
}
pub fn authorize(s: &AppState, headers: &HeaderMap, method: &str, path: &str) -> Api<Principal> {
    same_origin(headers)?;
    // VPN capture nodes may connect without a token unless APP_TOKEN is configured.
    // This grants only the client transport, never workspace or setup APIs.
    let node_path = path
        .strip_prefix("/api")
        .unwrap_or(path)
        .starts_with("/client/");
    if node_path {
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if s.capture_token
            .as_ref()
            .is_some_and(|token| !constant_eq(bearer, token))
            && !s
                .token
                .as_ref()
                .is_some_and(|token| constant_eq(bearer, token))
        {
            return Err(unauthorized());
        }
        return Ok(Principal {
            address: None,
            admin: false,
            node: true,
            wallet_index: 0,
        });
    }
    let p = principal(s, headers)?;
    if let Some(address) = &p.address
        && !matches!(crate::users::role(s, address)?.as_str(), "user" | "admin")
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Registration approval required".into(),
        ));
    }
    let path = path
        .strip_prefix("/api")
        .unwrap_or(path)
        .split('?')
        .next()
        .unwrap_or(path)
        .trim_end_matches('/');
    if path.starts_with("/client/") {
        if s.auth.admin.is_some() && !p.node && !p.admin {
            return Err(ApiError(
                StatusCode::FORBIDDEN,
                "Capture node access requires APP_TOKEN".into(),
            ));
        }
    } else if p.node {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "The node token only grants capture client access".into(),
        ));
    }
    let setup = path.starts_with("/admin/")
        || path == "/settings" && method != "GET"
        || path.starts_with("/clients/") && method != "GET"
        || path == "/ai/check";
    if setup && !p.admin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Only the CWBBOT_ADMIN wallet can change server setup".into(),
        ));
    }
    // Admin authority controls setup, never another wallet's private records.
    let segments = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
    if segments.len() >= 2 {
        let table = match segments[0] {
            "memos" => Some("memos"),
            "files" => Some("files"),
            "rooms" if segments[1] != "thought" => Some("rooms"),
            _ => None,
        };
        if let Some(table) = table {
            s.db.for_owner(p.address.as_deref().unwrap_or("local"))
                .require(table, segments[1])
                .map_err(|_| ApiError(StatusCode::NOT_FOUND, "Resource not found".into()))?;
        }
    }
    Ok(p)
}
/// A backend job is delegated to an account, not to the lifetime of a browser login.
/// This is internal only: callers must already have an explicitly owner-scoped state.
pub(crate) fn background_principal(s: &AppState) -> Api<Principal> {
    let owner = s.db.2.as_deref().ok_or_else(unauthorized)?;
    let address = (owner != "local").then(|| owner.to_owned());
    let role = if let Some(address) = &address {
        crate::users::role(s, address)?
    } else if s.auth.admin.is_none() {
        "admin".into()
    } else {
        return Err(unauthorized());
    };
    if !matches!(role.as_str(), "admin" | "user") {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Session owner access was revoked".into(),
        ));
    }
    Ok(Principal {
        address,
        admin: role == "admin",
        node: false,
        wallet_index: 0,
    })
}

pub async fn info(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Json<Value> {
    let p = principal(&s, &headers).ok().filter(|p| !p.node);
    let role = p
        .as_ref()
        .and_then(|p| p.address.as_ref())
        .and_then(|a| crate::users::role(&s, a).ok())
        .unwrap_or_else(|| {
            if p.is_some() {
                "admin".into()
            } else {
                "anonymous".into()
            }
        });
    Json(
        json!({"role":role,"approved":matches!(role.as_str(),"user"|"admin"),"required":s.auth.admin.is_some()||s.token.is_some(), "wallet_login":s.auth.admin.is_some(),
        "authenticated":p.is_some(),"address":p.as_ref().and_then(|p|p.address.clone()),"name":p.as_ref().and_then(|p|p.address.as_deref()).and_then(|a|crate::users::name(&s,a).ok()),"wallet_index":p.as_ref().and_then(|p|p.address.as_ref().map(|_|p.wallet_index)),"is_admin":p.is_some_and(|p|p.admin)}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeRequest {
    address: String,
}
pub async fn challenge(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(v): Json<ChallengeRequest>,
) -> Api<Json<Value>> {
    same_origin(&headers)?;
    if s.auth.admin.is_none() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Wallet login requires CWBBOT_ADMIN on the backend".into(),
        ));
    }
    let address =
        normalize_address(&v.address).map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?;
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost")
        .to_string();
    let mut challenges = s.auth.challenges.lock().unwrap();
    challenges.retain(|_, c| c.expires > stamp());
    if challenges.len() >= 1024 {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Please retry login in two minutes".into(),
        ));
    }
    let nonce = random();
    let expires = stamp() + 120;
    let message = format!(
        "Causewaybay Bot login\nHost: {host}\nAddress: {address}\nNonce: {nonce}\nExpires: {expires}\nSign in to your private wallet-owned workspace. No transaction or payment."
    );
    challenges.insert(
        nonce.clone(),
        Challenge {
            address,
            message: message.clone(),
            expires,
            host,
        },
    );
    Ok(Json(
        json!({"nonce":nonce,"message":message,"expires":expires}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyRequest {
    nonce: String,
    signature: String,
    #[serde(default)]
    wallet_index: u32,
    name: Option<String>,
}
pub async fn verify(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(v): Json<VerifyRequest>,
) -> Api<Response> {
    same_origin(&headers)?;
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let mut challenges = s.auth.challenges.lock().unwrap();
    let c = challenges
        .get(&v.nonce)
        .filter(|c| c.expires > stamp() && c.host == host)
        .ok_or_else(unauthorized)?;
    if recover(&c.message, &v.signature).ok().as_ref() != Some(&c.address) {
        return Err(unauthorized());
    }
    let address = c.address.clone();
    causewaybay_shared::identity::validate_wallet_index(v.wallet_index)
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?;
    crate::users::register(&s, &address, v.name.as_deref())?;
    challenges.remove(&v.nonce);
    create_wallet_session(&s, &headers, Some(&address), v.wallet_index)
}
pub fn create_session(s: &AppState, headers: &HeaderMap, address: Option<&str>) -> Api<Response> {
    create_wallet_session(s, headers, address, 0)
}
fn create_wallet_session(
    s: &AppState,
    headers: &HeaderMap,
    address: Option<&str>,
    wallet_index: u32,
) -> Api<Response> {
    let token = random();
    let conn = s.db.0.lock().unwrap();
    conn.execute("DELETE FROM auth_sessions WHERE expires<=?1", [stamp()])?;
    if let Some(old) = cookie(headers) {
        conn.execute("DELETE FROM auth_sessions WHERE token_hash=?1", [hash(old)])?;
    }
    conn.execute(
        "INSERT INTO auth_sessions(token_hash,address,expires,wallet_index) VALUES(?1,?2,?3,?4)",
        params![hash(&token), address, stamp() + 604800, wallet_index],
    )?;
    let secure = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("https://"));
    let cookie = format!(
        "cwb_session={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=604800{}",
        if secure { "; Secure" } else { "" }
    );
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({"ok":true}))).into_response())
}
pub async fn logout(State(s): State<Arc<AppState>>, headers: HeaderMap) -> Api<Response> {
    same_origin(&headers)?;
    if let Some(token) = cookie(&headers) {
        s.db.0.lock().unwrap().execute(
            "DELETE FROM auth_sessions WHERE token_hash=?1",
            [hash(token)],
        )?;
    }
    Ok((
        [(
            header::SET_COOKIE,
            "cwb_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
        )],
        Json(json!({"ok":true})),
    )
        .into_response())
}
