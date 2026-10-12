use crate::{
    api::{ConnectionState, api},
    i18n::{Language, t, tf},
    icons::icon,
};
use causewaybay_shared::{CaptureClient, WorkspaceStatus};
use gloo_timers::callback::Interval;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: Language,
    pub connection: ConnectionState,
    pub copy: Callback<String>,
}
pub struct Status {
    data: Option<WorkspaceStatus>,
    error: String,
    system: bool,
    loading: bool,
    _refresh: Interval,
}
pub enum Msg {
    Load,
    Loaded(Box<Result<WorkspaceStatus, String>>),
    Scope(bool),
}
impl Component for Status {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        ctx.link().send_message(Msg::Load);
        let link = ctx.link().clone();
        Self {
            data: None,
            error: String::new(),
            system: false,
            loading: false,
            _refresh: Interval::new(3000, move || link.send_message(Msg::Load)),
        }
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Load => {
                if self.loading || ctx.props().connection != ConnectionState::Connected {
                    return false;
                }
                self.loading = true;
                ctx.link().send_future(async {
                    Msg::Loaded(Box::new(api("GET", "/api/status", None).await))
                });
            }
            Msg::Loaded(result) => {
                self.loading = false;
                match *result {
                    Ok(data) => {
                        self.data = Some(data);
                        self.error.clear();
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::Scope(system) => self.system = system,
        }
        true
    }
    fn changed(&mut self, ctx: &Context<Self>, _: &Props) -> bool {
        ctx.link().send_message(Msg::Load);
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let live = ctx.props().connection == ConnectionState::Connected && self.error.is_empty();
        let Some(data) = &self.data else {
            return html! {<section class="status-page"><div class="page-intro"><h1>{t("Your workspace, at a glance.")}</h1></div><div class="status-loading" role="status">{icon("dashboard")}{if self.error.is_empty(){t("Connecting to your backend…")}else{t(&self.error)}}<button class="text-button" onclick={ctx.link().callback(|_|Msg::Load)}>{t("Refresh")}</button></div></section>};
        };
        let system = data.system.as_ref().filter(|_| self.system);
        let stats = system.map(|s| &s.stats).unwrap_or(&data.stats);
        let connections = system.map(|s| &s.connections).unwrap_or(&data.connections);
        let scene=serde_json::json!({"scope":if system.is_some(){"system"}else{"personal"},"live":live,"counts":[stats.web_online,stats.rust_online,stats.rooms,stats.memos,stats.files]}).to_string();
        let checked = js_sys::Date::new(&data.updated_at.clone().into())
            .to_locale_time_string("en-GB")
            .as_string()
            .unwrap_or_default();
        html! {<section class="status-page" aria-label={t("Dashboard")}>
            <div class="status-intro"><div><div class="eyebrow">{icon("dashboard")}{t("THE BIG PICTURE")}</div><h1>{if system.is_some(){t("Everything, working together.")}else{t("Your workspace, at a glance.")}}</h1><p>{if system.is_some(){t("A live view of your system, people, and connected devices.")}else{t("Your thoughts, files, and devices. One clear view.")}}</p></div><span class={classes!("status-live",(!live).then_some("stale"))}><i/>{if live{t("Live overview")}else{t("Last known status")}}</span></div>
            <div class="status-toolbar">
                if data.system.is_some(){<div class="status-tabs" role="group" aria-label={t("Dashboard scope")}><button aria-pressed={(!self.system).to_string()} onclick={ctx.link().callback(|_|Msg::Scope(false))}>{icon("home")}{t("My workspace")}</button><button aria-pressed={self.system.to_string()} onclick={ctx.link().callback(|_|Msg::Scope(true))}>{icon("shield")}{t("System overview")}</button></div>}
                else {<span class="status-scope-label">{icon("shield")}{t("My workspace")}</span>}
                <div class="status-updated"><span>{tf("Updated {time}",&[("time",checked)])}</span><button class="icon-button" aria-label={t("Refresh dashboard")} disabled={self.loading} onclick={ctx.link().callback(|_|Msg::Load)}>{icon("refresh")}</button></div>
            </div>
            if !self.error.is_empty(){<p class="error-banner" role="alert">{t(&self.error)}</p>}
            <div class="status-metrics">
                {metric("Memos","book",stats.memos,&tf("{count} pinned",&[("count",stats.pinned.to_string())]),"memos")}
                {metric("Files","folder",stats.files,&tf("{count} images",&[("count",stats.images.to_string())]),"files")}
                {metric("Chatrooms","message",stats.rooms,&tf("{count} messages",&[("count",stats.messages.to_string())]),"rooms")}
                {metric("Connected clients","monitor",stats.web_online+stats.rust_online,&tf("{web} web · {rust} Rust",&[("web",stats.web_online.to_string()),("rust",stats.rust_online.to_string())]),"clients")}
            </div>
            <div class="status-middle">
                <section class="status-map-panel" aria-label={t("Workspace diagram")}><div class="status-panel-heading"><div><span class="status-kicker">{t("CONNECTED WORKSPACE")}</span><h2>{t("A place for every thought.")}</h2></div><span class="status-map-key"><i/>{if live{t("Connected")}else{t("Unknown")}}</span></div>
                    <div class="status-map"><cwb-status-diagram data-state={scene} aria-hidden="true"/>
                        <div class="status-map-center"><span>{icon("spark")}</span><strong>{t("Causewaybay")}</strong><small>{if live{t("Backend connected")}else{t("Waiting for the backend")}}</small></div>
                        {node("web", "home", "Web clients", stats.web_online, true)}
                        {node("rust", "screen", "Rust clients", stats.rust_online, true)}
                        {node("rooms", "message", "Chatrooms", stats.rooms, false)}
                        {node("memos", "book", "Memos", stats.memos, false)}
                        {node("files", "folder", "Files", stats.files, false)}
                    </div><div class="status-map-footer">{icon("shield")}<span>{if system.is_some(){t("System totals. Saved content stays private to its owner.")}else{t("Only your saved content is included in this view.")}}</span></div>
                </section>
                <section class="status-health"><div class="status-kicker">{t("RIGHT NOW")}</div><h2>{if stats.running>0{t("Ideas in motion.")}else{t("Room to think.")}}</h2><p>{if stats.running>0{t("Your backend is working on these requests.")}else{t("No requests running. Your workspace is ready.")}}</p>
                    <dl>{health("bolt","Running requests",stats.running)}{health("screen","Pending captures",stats.queued_captures)}{health("info","Failed requests",stats.failed)}</dl>
                    <div class="status-uptime">{icon("clock")}<div><small>{t("Backend uptime")}</small><strong>{duration(connections.uptime_seconds)}</strong></div><span>{format!("v{}",connections.server_version)}</span></div>
                </section>
            </div>
            if let Some(system)=system {<section class="status-system" aria-label={t("System health")}>
                <div class="status-user-summary"><span class="status-kicker">{t("PEOPLE & ACCESS")}</span><h2>{tf("{count} registered wallets",&[("count",system.users.total.to_string())])}</h2><div>{pill("Admins",system.users.admins)}{pill("Approved",system.users.approved)}{pill("Pending",system.users.pending)}{pill("Blocked",system.users.blocked)}</div><a class="text-button" href="/admin">{t("Manage access")}{icon("arrow")}</a></div>
                <div class="status-services"><span class="status-kicker">{t("SERVICES")}</span><div class="status-service"><strong>{t("Semantic search")}</strong><span>{t(&system.semantic_status)}</span></div>{for system.providers.iter().map(|p|html!{<div class="status-service"><strong>{&p.name}</strong><span class={classes!(p.configured.then_some("configured"))}>{if system.demo{t("Demo mode")}else if p.configured{t("Configured")}else{t("Not configured")}}</span></div>})}<small>{t("Configuration status does not test provider availability.")}</small></div>
            </section>}
            <div class="status-bottom"><section class="status-storage"><div class="status-panel-heading"><div><span class="status-kicker">{t("SAVED HERE")}</span><h2>{t("Your digital shelf")}</h2></div>{icon("folder")}</div><div class="status-storage-total"><strong>{bytes(stats.file_bytes+stats.screenshot_bytes)}</strong><span>{t("stored media")}</span></div>
                <div class="status-storage-bar" role="img" aria-label={tf("Files {files}, screenshots {screenshots}",&[("files",bytes(stats.file_bytes)),("screenshots",bytes(stats.screenshot_bytes))])}><i style={format!("width:{}%",if stats.file_bytes+stats.screenshot_bytes==0{0.}else{100.*stats.file_bytes as f64/(stats.file_bytes+stats.screenshot_bytes) as f64})}/><b style={format!("width:{}%",if stats.file_bytes+stats.screenshot_bytes==0{0.}else{100.*stats.screenshot_bytes as f64/(stats.file_bytes+stats.screenshot_bytes) as f64})}/></div><dl><div><dt><i/>{t("Uploaded files")}</dt><dd>{bytes(stats.file_bytes)}</dd></div><div><dt><i/>{t("Captured screenshots")}</dt><dd>{bytes(stats.screenshot_bytes)}</dd></div></dl><p>{t("File and screenshot bytes. Text, indexes, and database overhead are excluded.")}</p><div class="status-shortcuts"><a href="/files">{t("Files")}{icon("arrow")}</a><a href="/library">{t("Memos")}{icon("arrow")}</a><a href="/chat">{t("Chatrooms")}{icon("arrow")}</a></div>
            </section><section class="status-package" aria-label={t("Capture client package")}><div class="status-package-icon">{icon("screen")}</div><div class="status-kicker">{t("YOUR NEXT CONNECTION")}</div><h2>{t("A capture node, in one download.")}</h2><p>{t("Unpack client.zip and run go.sh. Your computer connects to this workspace as your capture node.")}</p><div class="status-package-meta"><span>{icon("monitor")}{&data.package.platform}</span><span>{icon("shield")}{t("Your wallet · Tailscale IP")}</span></div>
                if let Some(url)=&data.package.backend_url{<code class="status-package-url">{url}</code>}
                if let Some(owner)=&data.package.owner_wallet{<button class="status-wallet" title={owner.clone()} aria-label={t("Copy owner wallet")} onclick={ctx.props().copy.reform({let owner=owner.clone();move |_| owner.clone()})}><span>{owner}</span>{icon("copy")}</button>}
                if data.package.available{<a class="primary-button status-download" href="/api/download/client.zip" download="client.zip">{icon("download")}{t("Download client.zip")}{icon("arrow")}</a>}
                else {<button class="primary-button status-download" disabled=true>{icon("download")}{t("Download client.zip")}</button><p class="status-package-reason">{t(&data.package.reason)}</p>}
                if data.package.token_required{<small>{t("The launcher asks for your backend node token. It is not included in the download.")}</small>}
                <small>{t("Use a computer matching the package platform. Allow Screen Recording when prompted.")}</small>
            </section></div>
            <section class="status-connections"><div class="status-panel-heading"><div><span class="status-kicker">{t("DEVICE ACTIVITY")}</span><h2>{t("Connected, wherever you work.")}</h2></div><a class="text-button" href="/devices">{t("Manage devices")}{icon("arrow")}</a></div><div class="status-connection-grid">{clients("Rust clients","screen",&connections.rust_clients,stats.rust_total,live)}{clients("Web clients","home",&connections.web_clients,stats.web_total,live)}</div></section>
        </section>}
    }
}
fn metric(label: &str, ico: &str, value: u64, detail: &str, id: &str) -> Html {
    html! {<article class={classes!("status-metric",format!("metric-{id}"))}><span class="status-metric-icon">{icon(ico)}</span><div><span>{t(label)}</span><strong data-testid={format!("status-{id}")}>{value}</strong><small>{detail}</small></div></article>}
}
fn node(id: &str, ico: &str, label: &str, value: u64, online: bool) -> Html {
    html! {<div class={classes!("status-node",format!("node-{id}"))}><span>{icon(ico)}</span><strong>{value}</strong><small>{t(label)}if online{<em>{t("online")}</em>}</small></div>}
}
fn health(ico: &str, label: &str, value: u64) -> Html {
    html! {<div><dt>{icon(ico)}{t(label)}</dt><dd>{value}</dd></div>}
}
fn pill(label: &str, value: u64) -> Html {
    html! {<span class="status-pill"><b>{value}</b>{t(label)}</span>}
}
fn clients(label: &str, ico: &str, clients: &[CaptureClient], total: u64, live: bool) -> Html {
    let row = |c: &CaptureClient| html! {<li key={c.id.clone()}><span class="status-client-icon">{icon(ico)}</span><div><strong>{&c.name}</strong><small>{&c.platform}</small></div><span class={classes!("presence-badge",(live&&c.online).then_some("online"))}>{if !live{t("Unknown")}else if c.online{t("Connected")}else{t("Offline")}}</span></li>};
    html! {<div class="status-client-list"><h3>{icon(ico)}{t(label)}<span>{total}</span></h3><ul>{for clients.iter().take(6).map(row)}</ul>if clients.is_empty(){<p>{t("No devices connected yet.")}</p>}if clients.len()>6{<details class="status-more-clients"><summary>{tf("Show {count} more devices",&[("count",(clients.len()-6).to_string())])}</summary><ul>{for clients.iter().skip(6).map(row)}</ul></details>}</div>}
}
fn duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h {}m", seconds / 3600, seconds / 60 % 60)
    } else {
        format!("{}d {}h", seconds / 86400, seconds / 3600 % 24)
    }
}
fn bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1048576 {
        format!("{:.1} KB", n as f64 / 1024.)
    } else if n < 1073741824 {
        format!("{:.1} MB", n as f64 / 1048576.)
    } else {
        format!("{:.2} GB", n as f64 / 1073741824.)
    }
}
