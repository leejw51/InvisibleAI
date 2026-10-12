use crate::i18n::{t, tf};
use crate::{api::api, icons::icon};
use gloo_timers::callback::Interval;
use serde_json::{Value, json};
use yew::prelude::*;
#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: crate::i18n::Language,
    pub settings: Callback<()>,
}
pub struct Admin {
    data: Value,
    error: String,
    busy: bool,
    _poll: Interval,
}
pub enum Msg {
    Load,
    Loaded(Result<Value, String>),
    Role(String, String),
    Saved(Result<Value, String>),
}
impl Component for Admin {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        ctx.link().send_message(Msg::Load);
        let link = ctx.link().clone();
        Self {
            data: Value::Null,
            error: String::new(),
            busy: false,
            _poll: Interval::new(3000, move || link.send_message(Msg::Load)),
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Load => {
                ctx.link()
                    .send_future(async { Msg::Loaded(api("GET", "/api/admin/users", None).await) });
                return false;
            }
            Msg::Loaded(r) => match r {
                Ok(v) => self.data = v,
                Err(e) => self.error = e,
            },
            Msg::Role(address, role) => {
                self.busy = true;
                ctx.link().send_future(async move {
                    Msg::Saved(
                        api(
                            "PATCH",
                            &format!("/api/admin/users/{address}"),
                            Some(json!({"role":role})),
                        )
                        .await,
                    )
                });
            }
            Msg::Saved(r) => {
                self.busy = false;
                match r {
                    Ok(_) => {
                        self.error.clear();
                        ctx.link().send_message(Msg::Load);
                    }
                    Err(e) => self.error = e,
                }
            }
        }
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let rows = self.data["users"].as_array().cloned().unwrap_or_default();
        let pending = rows.iter().filter(|u| u["role"] == "pending").count();
        html! {<section class="admin-page"><div class="page-intro"><div><div class="eyebrow">{icon("shield")}{t("ADMINISTRATION")}</div><h1>{t("Your workspace, together.")}</h1><p>{t("Review registrations and manage access.")}</p></div><button class="secondary-button" onclick={ctx.props().settings.reform(|_|())}>{icon("settings")}{t("Backend settings")}</button></div>
            if !self.error.is_empty(){<p class="error-banner" role="alert">{t(&self.error)}</p>}
            <h2>{tf("Waiting for approval · {count}", &[("count", pending.to_string())])}</h2>
            if pending==0{<p class="small-muted">{t("No registrations waiting.")}</p>}
            <div class="user-list">{for rows.iter().filter(|u|u["role"]=="pending").map(|u|self.row(ctx,u))}</div>
            <h2>{t("Approved users & access")}</h2><div class="user-list">{for rows.iter().filter(|u|u["role"]!="pending").map(|u|self.row(ctx,u))}</div>
        </section>}
    }
}
impl Admin {
    fn row(&self, ctx: &Context<Self>, u: &Value) -> Html {
        let address = u["address"].as_str().unwrap_or_default();
        let role = u["role"].as_str().unwrap_or_default();
        let root = self.data["owner"] == u["address"];
        let action = |label: &str, new_role: &str| {
            let a = address.to_string();
            let r = new_role.to_string();
            html! {<button class="secondary-button" disabled={self.busy} onclick={ctx.link().callback(move |_|Msg::Role(a.clone(),r.clone()))}>{t(label)}</button>}
        };
        html! {<article class="user-row" data-address={address.to_string()}><div><strong>{u["name"].as_str().unwrap_or("Wallet")}</strong><code>{address}</code><span class="role-badge">{t(if root{"Server owner"}else{role})}</span></div><div class="user-actions">if !root {
            if role=="pending"||role=="blocked" {{action("Approve user","user")}}
            if role=="user" {{action("Make admin","admin")}}
            if role=="admin" {{action("Make user","user")}}
            if role!="blocked" {{action(if role=="pending"{"Decline"}else{"Block access"},"blocked")}}
        }</div></article>}
    }
}
