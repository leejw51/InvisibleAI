//! Approved wallet list. JSONL snapshots are durable; SQLite serves live checks.
use crate::{Api, ApiError, AppState, auth};
use anyhow::Result;
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use causewaybay_shared::{config, identity::normalize_address};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone, Deserialize, Serialize)]
pub struct User {
    pub address: String,
    pub role: String,
    pub name: String,
    pub requested_at: String,
    pub updated_at: String,
}
pub fn role(s: &AppState, address: &str) -> Result<String> {
    if s.auth.admin.as_deref() == Some(address) {
        return Ok("admin".into());
    }
    Ok(s.db
        .0
        .lock()
        .unwrap()
        .query_row("SELECT role FROM users WHERE address=?1", [address], |r| {
            r.get(0)
        })
        .optional()?
        .unwrap_or_else(|| "unregistered".into()))
}
fn read(s: &AppState) -> Result<Vec<User>> {
    let conn = s.db.0.lock().unwrap();
    Ok(conn.prepare("SELECT address,role,name,requested_at,updated_at FROM users ORDER BY requested_at,address")?.query_map([],|r|Ok(User { address:r.get(0)?,role:r.get(1)?,name:r.get(2)?,requested_at:r.get(3)?,updated_at:r.get(4)? }))?.collect::<rusqlite::Result<Vec<_>>>()?)
}
fn replace(s: &AppState, users: &[User]) -> Result<()> {
    let mut conn = s.db.0.lock().unwrap();
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM users", [])?;
    for u in users {
        tx.execute(
            "INSERT INTO users(address,role,name,requested_at,updated_at) VALUES(?1,?2,?3,?4,?5)",
            params![u.address, u.role, u.name, u.requested_at, u.updated_at],
        )?;
    }
    tx.commit()?;
    Ok(())
}
pub fn restore(s: &AppState) -> Result<()> {
    let value = config::read(&s.config_dir, "users")?;
    if !value.is_null() {
        let users: Vec<User> = serde_json::from_value(value["users"].clone())?;
        replace(s, &users)?;
    }
    Ok(())
}
fn save(s: &AppState, users: &[User], actor: &str, action: &str, target: &str) -> Result<()> {
    config::append(
        &s.config_dir,
        "users",
        json!({"users":users,"actor":actor,"action":action,"target":target}),
    )?;
    replace(s, users)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Application {
    #[serde(default)]
    name: Option<String>,
}
pub async fn apply(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(v): Json<Application>,
) -> Api<Json<Value>> {
    auth::same_origin(&headers)?;
    let p = auth::principal(&s, &headers)?;
    let address = p.address.ok_or_else(|| {
        ApiError(
            StatusCode::UNAUTHORIZED,
            "Sign in with your recovery phrase first".into(),
        )
    })?;
    if role(&s, &address)? == "blocked" {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Access was declined. Contact a workspace admin.".into(),
        ));
    }
    register(&s, &address, v.name.as_deref())?;
    Ok(Json(json!({"ok":true,"role":role(&s,&address)?})))
}
/// Names are labels only. Wallet addresses remain the authorization/ownership key.
pub fn name(s: &AppState, address: &str) -> Result<String> {
    let stored: Option<String> =
        s.db.0
            .lock()
            .unwrap()
            .query_row("SELECT name FROM users WHERE address=?1", [address], |r| {
                r.get(0)
            })
            .optional()?;
    match stored.filter(|name| !name.trim().is_empty()) {
        Some(name) => Ok(name),
        None => causewaybay_shared::identity::username(address).map_err(anyhow::Error::msg),
    }
}
pub fn register(s: &AppState, address: &str, requested_name: Option<&str>) -> Api<()> {
    let requested_name = requested_name.map(str::trim);
    if requested_name.is_some_and(|name| {
        name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control)
    }) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "User name must contain 1 to 80 characters without control characters.".into(),
        ));
    }
    let _lock = s.settings_write.lock().unwrap();
    let mut users = read(s)?;
    if let Some(user) = users.iter_mut().find(|user| user.address == address) {
        if let Some(name) = requested_name.filter(|name| *name != user.name) {
            user.name = name.into();
            user.updated_at = crate::db::now();
            save(s, &users, address, "rename", address)?;
        }
    } else if s.auth.admin.as_deref() != Some(address) || requested_name.is_some() {
        users.push(User {
            address: address.into(),
            role: if s.auth.admin.as_deref() == Some(address) {
                "admin"
            } else {
                "pending"
            }
            .into(),
            name: requested_name.map(str::to_owned).unwrap_or(
                causewaybay_shared::identity::username(address).map_err(anyhow::Error::msg)?,
            ),
            requested_at: crate::db::now(),
            updated_at: crate::db::now(),
        });
        save(s, &users, address, "apply", address)?;
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    name: String,
}
pub async fn profile(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(v): Json<Profile>,
) -> Api<Json<Value>> {
    auth::same_origin(&headers)?;
    let address = auth::principal(&s, &headers)?.address.ok_or_else(|| {
        ApiError(
            StatusCode::UNAUTHORIZED,
            "Sign in with your recovery phrase first".into(),
        )
    })?;
    register(&s, &address, Some(&v.name))?;
    Ok(Json(json!({"ok":true,"name":name(&s,&address)?})))
}

pub async fn list(State(s): State<Arc<AppState>>) -> Api<Json<Value>> {
    let mut users = read(&s)?;
    if let Some(address) = &s.auth.admin {
        if let Some(root) = users.iter_mut().find(|u| u.address == *address) {
            root.role = "admin".into();
        } else {
            users.insert(
                0,
                User {
                    address: address.clone(),
                    role: "admin".into(),
                    name: causewaybay_shared::identity::username(address)
                        .map_err(anyhow::Error::msg)?,
                    requested_at: String::new(),
                    updated_at: String::new(),
                },
            );
        }
    }
    Ok(Json(json!({"users":users,"owner":s.auth.admin})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeRole {
    role: String,
}
pub async fn update(
    State(s): State<Arc<AppState>>,
    Path(address): Path<String>,
    headers: HeaderMap,
    Json(v): Json<ChangeRole>,
) -> Api<Json<Value>> {
    let actor = auth::authorize(&s, &headers, "PATCH", "/api/admin/users")?
        .address
        .unwrap_or_default();
    let address = normalize_address(&address).map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?;
    if s.auth.admin.as_ref() == Some(&address) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "CWBBOT_ADMIN is the permanent server owner".into(),
        ));
    }
    if !matches!(v.role.as_str(), "user" | "admin" | "blocked") {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Choose user, admin or blocked".into(),
        ));
    }
    let _lock = s.settings_write.lock().unwrap();
    let mut users = read(&s)?;
    let u = users
        .iter_mut()
        .find(|u| u.address == address)
        .ok_or_else(|| {
            ApiError(
                StatusCode::NOT_FOUND,
                "The wallet must apply for registration first".into(),
            )
        })?;
    if v.role == "admin" && !matches!(u.role.as_str(), "user" | "admin") {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Approve this user before making them an admin".into(),
        ));
    }
    u.role = v.role.clone();
    u.updated_at = crate::db::now();
    save(&s, &users, &actor, &v.role, &address)?;
    Ok(Json(json!({"ok":true})))
}
