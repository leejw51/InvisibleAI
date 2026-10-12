//! The Wiki: every message is one node, tags organize them, and the feed, the
//! map and the related panel are three views of the same organized knowledge.
//! Routes: `/wiki` (feed), `/wiki/tag/{tag}` (one tag), `/wiki/map` (3D map),
//! `/wiki/memo/{id}` (one message read in place) and `/wiki/{id}` (a tag's
//! derived document). A reader or a picture opens over the collection, which
//! stays exactly as it was (scroll, map camera, search) for the one Back.
use crate::{
    api::{api, encode},
    i18n::{t, tf},
    icons::icon,
    motion::FadeSwap,
};
use gloo_events::EventListener;
use gloo_timers::callback::{Interval, Timeout};
use serde_json::Value;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{HtmlElement, HtmlInputElement};
use yew::prelude::*;

/// Feed or map, and the tag that was open, survive leaving the Wiki (for example
/// to read a source) and coming back.
const VIEW_KEY: &str = "causewaybay.wiki.view.v2";
const TAG_KEY: &str = "causewaybay.wiki.tag.v2";
/// Fullscreen is a mode of the Wiki alone; the rest of the app is untouched.
const FULLSCREEN_KEY: &str = "causewaybay.wiki.fullscreen.v1";
const FULLSCREEN_CLASS: &str = "wiki-fullscreen";
/// While a reader or a picture is open the page under it must not scroll.
const OVERLAY_CLASS: &str = "wiki-overlay-open";
/// Searches that found something, newest first, offered again after a few letters.
const QUERIES_KEY: &str = "causewaybay.wiki.queries.v1";
const REMEMBERED_QUERIES: usize = 30;

fn remember(key: &str, value: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = if value.is_empty() {
            storage.remove_item(key)
        } else {
            storage.set_item(key, value)
        };
    }
}
fn remembered(key: &str) -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key).ok().flatten())
        .unwrap_or_default()
}
fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit())
}
/// A memo id as the backend mints them.
fn valid_memo(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}
fn body_class(name: &str, on: bool) {
    if let Some(body) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.body())
    {
        let _ = if on {
            body.class_list().add_1(name)
        } else {
            body.class_list().remove_1(name)
        };
    }
}
fn remembered_queries() -> Vec<String> {
    serde_json::from_str::<Vec<String>>(&remembered(QUERIES_KEY)).unwrap_or_default()
}
/// The media behind one `/api/...` link in a document, if it is this backend's.
fn media_from_link(href: &str, poster: &str) -> Option<Value> {
    let kind = if href.starts_with("/api/files/") && href.ends_with("/stream") {
        "video"
    } else if (href.starts_with("/api/files/") || href.starts_with("/api/memos/"))
        && href.ends_with("/image")
    {
        "image"
    } else {
        return None;
    };
    Some(serde_json::json!({"kind":kind,"url":href,"poster":poster,"name":""}))
}
fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 200
        && tag
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '&'))
}
fn decode(v: &str) -> String {
    js_sys::decode_uri_component(v)
        .ok()
        .and_then(|s| s.as_string())
        .unwrap_or_else(|| v.to_string())
}
fn location_path() -> String {
    web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .unwrap_or_default()
}
/// What the address bar asks for.
#[derive(Clone, PartialEq)]
enum Route {
    Feed(String),
    Map,
    Document(String),
    /// One message, read without leaving the collection.
    Source(String),
}
fn route_of(path: &str) -> Option<Route> {
    if path == "/wiki" {
        return Some(Route::Feed(String::new()));
    }
    if path == "/wiki/map" {
        return Some(Route::Map);
    }
    if let Some(tag) = path.strip_prefix("/wiki/tag/") {
        let tag = decode(tag);
        return valid_tag(&tag).then_some(Route::Feed(tag));
    }
    if let Some(id) = path.strip_prefix("/wiki/memo/") {
        return valid_memo(id).then(|| Route::Source(id.to_string()));
    }
    path.strip_prefix("/wiki/")
        .filter(|id| valid_id(id))
        .map(|id| Route::Document(id.to_string()))
}
fn route_path(route: &Route) -> String {
    match route {
        Route::Feed(tag) if tag.is_empty() => "/wiki".into(),
        Route::Feed(tag) => format!("/wiki/tag/{}", encode(tag)),
        Route::Map => "/wiki/map".into(),
        Route::Document(id) => format!("/wiki/{id}"),
        Route::Source(id) => format!("/wiki/memo/{id}"),
    }
}
/// The screen as it was before a tag was opened, so closing the tag brings
/// it back exactly: the earlier tag or search, feed or map, the related panel,
/// any reader, and how far down the page was.
struct Before {
    tag: String,
    query: String,
    offset: usize,
    show_graph: bool,
    focus: String,
    document: String,
    source: String,
    scroll: (f64, i32),
}
const BEFORE_DEPTH: usize = 20;
/// What the lightbox shows: a message's pictures and clips, one at a time.
struct Media {
    items: Vec<Value>,
    index: usize,
    title: String,
    /// The message the pictures belong to, when known: one tap reads it.
    memo: String,
}
/// Call a zero-argument DOM method by name and let its promise settle quietly.
/// Browsers refuse fullscreen without a fresh gesture; the Wiki's own fullscreen
/// layout works either way, so a refusal is not an error.
fn call_quietly(target: &JsValue, name: &str) {
    if let Ok(f) = js_sys::Reflect::get(target, &name.into())
        && let Some(f) = f.dyn_ref::<js_sys::Function>()
        && let Ok(result) = f.call0(target)
        && result.is_instance_of::<js_sys::Promise>()
    {
        wasm_bindgen_futures::spawn_local(async move {
            let _ = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::from(result)).await;
        });
    }
}
fn browser_fullscreen() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.fullscreen_element())
        .is_some()
}
/// The whole app steps aside: only the Wiki stays on screen.
fn apply_fullscreen(on: bool) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Some(body) = document.body() {
        let _ = if on {
            body.class_list().add_1(FULLSCREEN_CLASS)
        } else {
            body.class_list().remove_1(FULLSCREEN_CLASS)
        };
    }
    if on {
        if let Some(root) = document.document_element() {
            call_quietly(root.as_ref(), "requestFullscreen");
        }
    } else if browser_fullscreen() {
        call_quietly(document.as_ref(), "exitFullscreen");
    }
}

pub struct Wiki {
    /// The feed: items, tag counts, related tags, settings.
    data: Value,
    /// The open tag document, when the route names one.
    page: Value,
    document: String,
    /// The one message open in the reader, and what the backend sent for it.
    source: String,
    source_page: Value,
    media: Option<Media>,
    /// Earlier searches that found something; shown under the box as you type.
    queries: Vec<String>,
    /// The suggestion list is open: typing opened it, nothing closed it yet.
    search_open: bool,
    /// One entry per tag opened on top of a screen; Close pops the latest.
    before: Vec<Before>,
    /// Scroll to put back once the restored feed has rendered.
    restore_scroll: Option<(f64, i32)>,
    /// The open tag's delete confirmation is showing.
    confirm_delete: bool,
    main_ref: NodeRef,
    /// Nodes and edges for the 3D map; fetched when the map is shown.
    graph: Value,
    show_graph: bool,
    fullscreen: bool,
    /// The map alone fills the viewport.
    map_full: bool,
    _graph_open_listener: Option<Closure<dyn FnMut(web_sys::Event)>>,
    /// The active tag filter, from the route or the rail.
    tag: String,
    query: String,
    offset: usize,
    /// The message whose relations are shown, and what the backend found.
    focus: String,
    related: Value,
    error: String,
    busy: bool,
    epoch: u64,
    graph_ref: NodeRef,
    _graph_listener: Option<Closure<dyn FnMut(web_sys::Event)>>,
    /// Browser Back/Forward between Wiki routes keeps this view in step.
    _history: EventListener,
    _keys: EventListener,
    _fullscreen: EventListener,
    article_ref: NodeRef,
    search_ref: NodeRef,
    focus_article: bool,
    focus_search: bool,
    debounce: Option<Timeout>,
    _poll: Interval,
    _notifications: Option<crate::api::Notifications>,
    /// A refresh is in flight (the toolbar button spins until the feed lands).
    refreshing: bool,
}
pub enum Msg {
    Load,
    Loaded(u64, Result<Value, String>),
    /// Reload everything on screen: the feed, the open document, the map and
    /// the related panel. From the toolbar button or a `wiki.updated` event.
    Refresh,
    /// Map camera: closer (negative) or farther; and the map alone filling the screen.
    Zoom(i32),
    ToggleMapFull,
    /// Esc: leave the map's own fullscreen first, then the Wiki's.
    Escape,
    Notification(Value),
    Query(String),
    ClearQuery,
    Offset(usize),
    /// Filter the feed by one tag (empty: everything).
    Tag(String),
    /// Close the open tag: the screen before it comes back as it was.
    CloseTag,
    /// Delete the open tag from every message, after a confirmation.
    AskDeleteTag(bool),
    DeleteTag,
    TagDeleted(String, Result<Value, String>),
    ShowMap,
    ShowFeed,
    OpenDocument(String),
    DocumentLoaded(u64, String, Result<Value, String>),
    /// The one Back: closes whichever reader is open; the collection is untouched.
    CloseDocument,
    /// Read one message in place.
    OpenSource(String),
    /// Tags saved from the reader: shown at once; the feed follows the backend's event.
    SourceTagged(String, Vec<String>),
    SourceLoaded(u64, String, Result<Value, String>),
    /// A tap on a map node: a picture or a clip plays, any other message opens.
    NodeTapped(String),
    /// Show pictures and clips over the page, starting at one of them.
    OpenMedia(Vec<Value>, usize, String, String),
    MediaStep(i32),
    CloseMedia,
    /// A click inside a document's text: its pictures, clips and source links
    /// open in place instead of leaving the Wiki.
    ReaderClick(MouseEvent),
    SearchFocus(bool),
    Suggest(String),
    Build,
    Built(Result<Value, String>),
    Graph(Result<Value, String>),
    Focus(String),
    Focused(u64, String, Result<Value, String>),
    ClearFocus,
    ToggleFullscreen,
    ExitFullscreen,
    /// The browser left fullscreen on its own (Esc in browser fullscreen).
    BrowserFullscreenChanged,
    History,
    ViewReady,
}
impl Component for Wiki {
    type Message = Msg;
    type Properties = crate::i18n::Props;
    fn create(ctx: &Context<Self>) -> Self {
        let link = ctx.link().clone();
        // Consumed on every mount so a Back that lands on the Wiki shows the URL
        // as it is rather than something remembered.
        let traversal = crate::take_history_traversal();
        let route = route_of(&location_path()).unwrap_or(Route::Feed(String::new()));
        let route = match route {
            // Arriving at the bare /wiki after reading a source returns to the
            // tag or the map that was open, not to everything.
            Route::Feed(tag) if tag.is_empty() && !traversal => {
                let saved_tag = remembered(TAG_KEY);
                if remembered(VIEW_KEY) == "map" {
                    Route::Map
                } else if valid_tag(&saved_tag) {
                    Route::Feed(saved_tag)
                } else {
                    Route::Feed(String::new())
                }
            }
            other => other,
        };
        crate::replace_location(route_path(&route));
        let (tag, show_graph, document, source) = match &route {
            Route::Feed(tag) => (tag.clone(), false, String::new(), String::new()),
            Route::Map => (remembered(TAG_KEY), true, String::new(), String::new()),
            Route::Document(id) => (remembered(TAG_KEY), false, id.clone(), String::new()),
            // A message opens over whichever view was last used.
            Route::Source(id) => (
                remembered(TAG_KEY),
                remembered(VIEW_KEY) == "map",
                String::new(),
                id.clone(),
            ),
        };
        remember(TAG_KEY, &tag);
        remember(VIEW_KEY, if show_graph { "map" } else { "feed" });
        let window = web_sys::window().unwrap();
        let history_link = ctx.link().clone();
        let history = EventListener::new(&window, "popstate", move |_| {
            history_link.send_message(Msg::History);
        });
        let keys_link = ctx.link().clone();
        let keys = EventListener::new(&window, "keydown", move |event| {
            // Esc in a search box with text clears the box (the browser's own
            // behaviour); only a second Esc leaves fullscreen.
            let clearing = event
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                .is_some_and(|input| !input.value().is_empty());
            if let Some(key) = event.dyn_ref::<web_sys::KeyboardEvent>()
                && key.key() == "Escape"
                && !clearing
            {
                keys_link.send_message(Msg::Escape);
            }
        });
        let fullscreen_link = ctx.link().clone();
        let fullscreen_change =
            EventListener::new(&window.document().unwrap(), "fullscreenchange", move |_| {
                fullscreen_link.send_message(Msg::BrowserFullscreenChanged);
            });
        let fullscreen = remembered(FULLSCREEN_KEY) == "on";
        if fullscreen {
            apply_fullscreen(true);
        }
        if show_graph {
            ctx.link()
                .send_future(async { Msg::Graph(api("GET", "/api/wiki/graph", None).await) });
        }
        link.send_message(Msg::Load);
        Self {
            data: Value::Null,
            page: Value::Null,
            focus_article: !document.is_empty() || !source.is_empty(),
            document,
            source,
            source_page: Value::Null,
            media: None,
            queries: remembered_queries(),
            search_open: false,
            before: vec![],
            restore_scroll: None,
            confirm_delete: false,
            main_ref: NodeRef::default(),
            graph: Value::Null,
            show_graph,
            fullscreen,
            map_full: false,
            _graph_open_listener: None,
            tag,
            query: String::new(),
            offset: 0,
            focus: String::new(),
            related: Value::Null,
            error: String::new(),
            busy: false,
            epoch: 0,
            graph_ref: NodeRef::default(),
            _graph_listener: None,
            _history: history,
            _keys: keys,
            _fullscreen: fullscreen_change,
            article_ref: NodeRef::default(),
            search_ref: NodeRef::default(),
            focus_search: false,
            debounce: None,
            // Changes arrive through the notification stream; the poll is a slow
            // fallback for a connection that missed one.
            _poll: Interval::new(60_000, move || link.send_message(Msg::Load)),
            _notifications: crate::api::notifications(ctx.link().callback(Msg::Notification)).ok(),
            refreshing: false,
        }
    }
    fn destroy(&mut self, _ctx: &Context<Self>) {
        // The mode belongs to the Wiki: leaving the page restores the app shell.
        apply_fullscreen(false);
        body_class(OVERLAY_CLASS, false);
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::ViewReady => return self.focus_article || self.focus_search,
            Msg::Load => {
                if self.busy {
                    return false;
                }
                self.epoch += 1;
                let epoch = self.epoch;
                let url = format!(
                    "/api/wiki?q={}&tag={}&offset={}",
                    encode(&self.query),
                    encode(&self.tag),
                    self.offset
                );
                ctx.link()
                    .send_future(async move { Msg::Loaded(epoch, api("GET", &url, None).await) });
                if !self.document.is_empty() {
                    let id = self.document.clone();
                    ctx.link().send_future(async move {
                        Msg::DocumentLoaded(
                            epoch,
                            id.clone(),
                            api("GET", &format!("/api/wiki/{id}"), None).await,
                        )
                    });
                }
                if !self.source.is_empty() {
                    let id = self.source.clone();
                    ctx.link().send_future(async move {
                        Msg::SourceLoaded(
                            epoch,
                            id.clone(),
                            api("GET", &format!("/api/wiki/memo/{id}"), None).await,
                        )
                    });
                }
                return false;
            }
            Msg::Refresh => {
                self.refreshing = true;
                self.busy = false;
                ctx.link().send_message(Msg::Load);
                if self.show_graph {
                    ctx.link().send_future(async {
                        Msg::Graph(api("GET", "/api/wiki/graph", None).await)
                    });
                }
                if !self.focus.is_empty() {
                    let epoch = self.epoch + 1;
                    let id = self.focus.clone();
                    ctx.link().send_future(async move {
                        Msg::Focused(
                            epoch,
                            id.clone(),
                            api("GET", &format!("/api/wiki/related/{id}"), None).await,
                        )
                    });
                }
            }
            Msg::Escape => {
                // Nearest layer first: the picture, then the reader, then the map
                // alone, then the Wiki's fullscreen.
                if self.media.is_some() {
                    self.media = None;
                } else if !self.document.is_empty() || !self.source.is_empty() {
                    ctx.link().send_message(Msg::CloseDocument);
                    return false;
                } else if self.map_full {
                    self.map_full = false;
                } else {
                    ctx.link().send_message(Msg::ExitFullscreen);
                    return false;
                }
            }
            Msg::ToggleMapFull => {
                self.map_full = !self.map_full;
                // The canvas measures itself on the next frame.
                if let Some(el) = self.graph_ref.cast::<HtmlElement>() {
                    let link = ctx.link().clone();
                    let _ = link;
                    let _ = js_sys::Reflect::get(el.as_ref(), &"resize".into())
                        .ok()
                        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
                        .map(|f| f.call0(el.as_ref()));
                }
            }
            Msg::Zoom(direction) => {
                if let Some(el) = self.graph_ref.cast::<HtmlElement>() {
                    let name = if direction < 0 { "zoomIn" } else { "zoomOut" };
                    let _ = js_sys::Reflect::get(el.as_ref(), &name.into())
                        .ok()
                        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
                        .map(|f| f.call0(el.as_ref()));
                }
                return false;
            }
            Msg::Notification(v) => {
                // Live Wiki changes only; replayed history is already on screen.
                if v["kind"] == "wiki.updated" && v["replay"] != true {
                    ctx.link().send_message(Msg::Refresh);
                }
                return false;
            }
            Msg::Loaded(epoch, r) => {
                if epoch != self.epoch {
                    return false;
                }
                self.refreshing = false;
                match r {
                    Ok(v) => {
                        self.data = v;
                        self.error.clear();
                        self.remember_query();
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::AskDeleteTag(open) => self.confirm_delete = open,
            Msg::DeleteTag => {
                if self.busy || self.tag.is_empty() {
                    return false;
                }
                self.busy = true;
                let tag = self.tag.clone();
                let url = format!("/api/wiki/tag/{}", encode(&tag));
                ctx.link().send_future(async move {
                    Msg::TagDeleted(tag, api("DELETE", &url, None).await)
                });
            }
            Msg::TagDeleted(tag, r) => {
                self.busy = false;
                self.confirm_delete = false;
                match r {
                    Ok(_) => {
                        self.error.clear();
                        // The tag is gone everywhere; the screen before it comes back.
                        if tag == self.tag {
                            ctx.link().send_message(Msg::CloseTag);
                        } else {
                            ctx.link().send_message(Msg::Load);
                        }
                        if self.show_graph {
                            ctx.link().send_future(async {
                                Msg::Graph(api("GET", "/api/wiki/graph", None).await)
                            });
                        }
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::CloseTag => {
                let Some(before) = self.before.pop() else {
                    ctx.link().send_message(Msg::Tag(String::new()));
                    return false;
                };
                self.epoch += 1;
                self.tag = before.tag;
                self.query = before.query;
                self.offset = before.offset;
                self.close_layers();
                self.document = before.document;
                self.source = before.source;
                self.focus_article = !self.document.is_empty() || !self.source.is_empty();
                if before.show_graph && !self.show_graph {
                    ctx.link().send_future(async {
                        Msg::Graph(api("GET", "/api/wiki/graph", None).await)
                    });
                }
                self.show_graph = before.show_graph;
                self.focus.clear();
                self.related = Value::Null;
                if !before.focus.is_empty() {
                    ctx.link().send_message(Msg::Focus(before.focus));
                }
                remember(TAG_KEY, &self.tag);
                remember(VIEW_KEY, if self.show_graph { "map" } else { "feed" });
                crate::push_location(route_path(&if !self.source.is_empty() {
                    Route::Source(self.source.clone())
                } else if !self.document.is_empty() {
                    Route::Document(self.document.clone())
                } else if self.show_graph {
                    Route::Map
                } else {
                    Route::Feed(self.tag.clone())
                }));
                self.restore_scroll = Some(before.scroll);
                ctx.link().send_message(Msg::Load);
            }
            Msg::SearchFocus(open) => self.search_open = open,
            Msg::Suggest(text) => {
                self.search_open = false;
                self.focus_search = true;
                ctx.link().send_message(Msg::Query(text));
            }
            Msg::Query(q) => {
                self.query = q;
                self.offset = 0;
                self.epoch += 1;
                let link = ctx.link().clone();
                self.debounce = Some(Timeout::new(250, move || link.send_message(Msg::Load)));
            }
            Msg::ClearQuery => {
                self.focus_search = true;
                ctx.link().send_message(Msg::Query(String::new()));
            }
            Msg::Offset(o) => {
                self.offset = o;
                ctx.link().send_message(Msg::Load);
            }
            Msg::Tag(tag) => {
                self.confirm_delete = false;
                if tag.is_empty() {
                    // Showing everything is a fresh start, not a step to come back from.
                    self.before.clear();
                } else if tag != self.tag {
                    self.before.push(Before {
                        tag: self.tag.clone(),
                        query: self.query.clone(),
                        offset: self.offset,
                        show_graph: self.show_graph,
                        focus: self.focus.clone(),
                        document: self.document.clone(),
                        source: self.source.clone(),
                        scroll: self.scroll_position(),
                    });
                    if self.before.len() > BEFORE_DEPTH {
                        self.before.remove(0);
                    }
                }
                self.tag = tag;
                self.offset = 0;
                self.focus.clear();
                self.related = Value::Null;
                self.close_layers();
                remember(TAG_KEY, &self.tag);
                // The address follows: a reader closed by a tag must not linger in it.
                crate::push_location(route_path(&if self.show_graph {
                    Route::Map
                } else {
                    Route::Feed(self.tag.clone())
                }));
                self.focus_search = !self.show_graph;
                ctx.link().send_message(Msg::Load);
            }
            Msg::ShowMap => {
                self.show_graph = true;
                self.close_layers();
                remember(VIEW_KEY, "map");
                crate::push_location(route_path(&Route::Map));
                ctx.link()
                    .send_future(async { Msg::Graph(api("GET", "/api/wiki/graph", None).await) });
            }
            Msg::ShowFeed => {
                self.show_graph = false;
                self.close_layers();
                remember(VIEW_KEY, "feed");
                crate::push_location(route_path(&Route::Feed(self.tag.clone())));
                self.focus_search = true;
                ctx.link().send_message(Msg::Load);
            }
            Msg::OpenDocument(id) => {
                if !valid_id(&id) {
                    return false;
                }
                self.epoch += 1;
                crate::push_location(route_path(&Route::Document(id.clone())));
                self.close_layers();
                self.document = id;
                self.focus_article = true;
                ctx.link().send_message(Msg::Load);
            }
            Msg::OpenSource(id) => {
                if !valid_memo(&id) {
                    return false;
                }
                self.epoch += 1;
                crate::push_location(route_path(&Route::Source(id.clone())));
                // A message opened from a tag document reads over it; Back
                // returns to the document, not past it.
                self.source_page = Value::Null;
                self.media = None;
                self.source = id;
                self.focus_article = true;
                ctx.link().send_message(Msg::Load);
            }
            Msg::SourceTagged(id, tags) => {
                if id != self.source || self.source_page.is_null() {
                    return false;
                }
                self.source_page["tags"] = serde_json::json!(tags);
            }
            Msg::SourceLoaded(epoch, id, r) => {
                if id != self.source || epoch != self.epoch {
                    return false;
                }
                match r {
                    Ok(v) => self.source_page = v,
                    Err(e) => {
                        self.source_page = Value::Null;
                        self.source.clear();
                        self.error = e;
                    }
                }
            }
            Msg::NodeTapped(id) => {
                // The map's node, or the same message as the feed or the related
                // panel knows it while the map is still loading.
                let node = self.graph["nodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .chain(self.data["items"].as_array().into_iter().flatten())
                    .chain(self.related["related"].as_array().into_iter().flatten())
                    .find(|n| n["id"] == id.as_str())
                    .cloned()
                    .unwrap_or(Value::Null);
                let media = node["media"].as_array().cloned().unwrap_or_default();
                if media.is_empty() {
                    ctx.link().send_message(Msg::OpenSource(id));
                } else {
                    let title = node["title"].as_str().unwrap_or("").to_owned();
                    ctx.link().send_message(Msg::OpenMedia(media, 0, title, id));
                }
                return false;
            }
            Msg::OpenMedia(items, index, title, memo) => {
                if items.is_empty() {
                    return false;
                }
                self.media = Some(Media {
                    index: index.min(items.len() - 1),
                    items,
                    title,
                    memo,
                });
            }
            Msg::MediaStep(step) => {
                if let Some(media) = &mut self.media {
                    let len = media.items.len() as i32;
                    media.index = ((media.index as i32 + step).rem_euclid(len)) as usize;
                }
            }
            Msg::CloseMedia => self.media = None,
            Msg::ReaderClick(event) => {
                let Some(target) = event
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                else {
                    return false;
                };
                let link = target.closest("a").ok().flatten();
                let href = link
                    .as_ref()
                    .and_then(|a| a.get_attribute("href"))
                    .unwrap_or_default();
                let picture = (target.tag_name() == "IMG")
                    .then(|| target.get_attribute("src"))
                    .flatten()
                    .unwrap_or_default();
                let title = self.page["title"].as_str().unwrap_or("").to_owned();
                if let Some(id) = href.strip_prefix("/memos/").filter(|id| valid_memo(id)) {
                    event.prevent_default();
                    ctx.link().send_message(Msg::OpenSource(id.to_string()));
                } else if let Some(item) = media_from_link(&href, &picture) {
                    event.prevent_default();
                    ctx.link()
                        .send_message(Msg::OpenMedia(vec![item], 0, title, String::new()));
                } else if let Some(item) = media_from_link(&picture, &picture) {
                    event.prevent_default();
                    ctx.link()
                        .send_message(Msg::OpenMedia(vec![item], 0, title, String::new()));
                }
                return false;
            }
            Msg::DocumentLoaded(epoch, id, r) => {
                if id != self.document || epoch != self.epoch {
                    return false;
                }
                match r {
                    Ok(v) => self.page = v,
                    Err(e) => {
                        self.page = Value::Null;
                        self.document.clear();
                        self.error = e;
                    }
                }
            }
            Msg::CloseDocument => {
                self.epoch += 1;
                if !self.source.is_empty() && !self.document.is_empty() {
                    // One layer at a time: the message closes, the document stays.
                    self.source.clear();
                    self.source_page = Value::Null;
                    self.media = None;
                    self.focus_article = true;
                    crate::push_location(route_path(&Route::Document(self.document.clone())));
                } else {
                    self.close_layers();
                    self.focus_article = false;
                    self.focus_search = true;
                    crate::push_location(route_path(&if self.show_graph {
                        Route::Map
                    } else {
                        Route::Feed(self.tag.clone())
                    }));
                }
                ctx.link().send_message(Msg::Load);
            }
            Msg::Build => {
                if self.busy {
                    return false;
                }
                self.busy = true;
                self.epoch += 1;
                ctx.link().send_future(async {
                    Msg::Built(api("POST", "/api/wiki/rebuild", None).await)
                });
            }
            Msg::Built(r) => {
                self.busy = false;
                match r {
                    Ok(_) => {
                        ctx.link().send_message(Msg::Load);
                        if self.show_graph {
                            ctx.link().send_future(async {
                                Msg::Graph(api("GET", "/api/wiki/graph", None).await)
                            });
                        }
                    }
                    Err(e) => self.error = e,
                }
            }
            Msg::Graph(r) => match r {
                Ok(v) => self.graph = v,
                Err(e) => self.error = e,
            },
            Msg::Focus(id) => {
                if id == self.focus {
                    return false;
                }
                self.epoch += 1;
                let epoch = self.epoch;
                self.focus = id.clone();
                self.related = Value::Null;
                ctx.link().send_future(async move {
                    Msg::Focused(
                        epoch,
                        id.clone(),
                        api("GET", &format!("/api/wiki/related/{id}"), None).await,
                    )
                });
            }
            Msg::Focused(epoch, id, r) => {
                if id != self.focus || epoch != self.epoch {
                    return false;
                }
                match r {
                    Ok(v) => self.related = v,
                    Err(e) => {
                        self.focus.clear();
                        self.error = e;
                    }
                }
            }
            Msg::ClearFocus => {
                self.focus.clear();
                self.related = Value::Null;
            }
            Msg::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen;
                remember(FULLSCREEN_KEY, if self.fullscreen { "on" } else { "" });
                apply_fullscreen(self.fullscreen);
            }
            Msg::ExitFullscreen => {
                if !self.fullscreen {
                    return false;
                }
                self.fullscreen = false;
                remember(FULLSCREEN_KEY, "");
                apply_fullscreen(false);
            }
            Msg::BrowserFullscreenChanged => {
                // Leaving browser fullscreen (Esc) leaves the Wiki mode too; the
                // layout alone would otherwise keep hiding the app.
                if self.fullscreen && !browser_fullscreen() {
                    return Component::update(self, ctx, Msg::ExitFullscreen);
                }
                return false;
            }
            Msg::History => {
                let Some(route) = route_of(&location_path()) else {
                    return false;
                };
                self.epoch += 1;
                self.media = None;
                match route {
                    Route::Feed(tag) => {
                        self.show_graph = false;
                        self.close_layers();
                        self.tag = tag;
                        remember(TAG_KEY, &self.tag);
                        remember(VIEW_KEY, "feed");
                    }
                    Route::Map => {
                        self.close_layers();
                        if !self.show_graph {
                            self.show_graph = true;
                            ctx.link().send_future(async {
                                Msg::Graph(api("GET", "/api/wiki/graph", None).await)
                            });
                        }
                        remember(VIEW_KEY, "map");
                    }
                    Route::Document(id) => {
                        self.source.clear();
                        self.source_page = Value::Null;
                        if id != self.document {
                            self.document = id;
                            self.page = Value::Null;
                        }
                        self.focus_article = true;
                    }
                    Route::Source(id) => {
                        if id != self.source {
                            self.source = id;
                            self.source_page = Value::Null;
                            self.focus_article = true;
                        }
                    }
                }
                ctx.link().send_message(Msg::Load);
            }
        }
        true
    }
    fn rendered(&mut self, ctx: &Context<Self>, _first: bool) {
        // The feed is back on screen: put the page where it was.
        if let Some((window_y, main_top)) = self.restore_scroll
            && !self.data.is_null()
            && self.document.is_empty()
            && self.source.is_empty()
        {
            if let Some(window) = web_sys::window() {
                window.scroll_to_with_x_and_y(0.0, window_y);
            }
            if let Some(main) = self.main_ref.cast::<HtmlElement>() {
                main.set_scroll_top(main_top);
            }
            self.restore_scroll = None;
        } else if self.restore_scroll.is_some()
            && (!self.document.is_empty() || !self.source.is_empty())
        {
            self.restore_scroll = None;
        }
        // The map is a custom element; a click on a node arrives as a DOM event.
        if self._graph_listener.is_none()
            && let Some(element) = self.graph_ref.cast::<HtmlElement>()
        {
            let link = ctx.link().clone();
            let closure =
                Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                    let detail = js_sys::Reflect::get(event.as_ref(), &"detail".into()).ok();
                    let field = |name: &str| {
                        detail
                            .as_ref()
                            .and_then(|d| js_sys::Reflect::get(d, &name.into()).ok())
                            .and_then(|v| v.as_string())
                            .unwrap_or_default()
                    };
                    let (id, kind) = (field("id"), field("kind"));
                    if kind == "tag" {
                        if let Some(tag) = id.strip_prefix("tag:")
                            && valid_tag(tag)
                        {
                            link.send_message(Msg::Tag(tag.to_string()));
                        }
                    } else if !id.is_empty() {
                        link.send_message(Msg::NodeTapped(id));
                    }
                });
            let _ = element
                .add_event_listener_with_callback("wiki-select", closure.as_ref().unchecked_ref());
            self._graph_listener = Some(closure);
            // A double tap (or double click) opens the node's page: a tag's
            // document, a message's source.
            let open_link = ctx.link().clone();
            let open = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                let detail = js_sys::Reflect::get(event.as_ref(), &"detail".into()).ok();
                let field = |name: &str| {
                    detail
                        .as_ref()
                        .and_then(|d| js_sys::Reflect::get(d, &name.into()).ok())
                        .and_then(|v| v.as_string())
                        .unwrap_or_default()
                };
                let (id, kind) = (field("id"), field("kind"));
                // A tag's double tap shows its messages; a message's reads it in place.
                if kind == "tag" {
                    if let Some(tag) = id.strip_prefix("tag:")
                        && valid_tag(tag)
                    {
                        open_link.send_message(Msg::Tag(tag.to_string()));
                        open_link.send_message(Msg::ShowFeed);
                    }
                } else if valid_memo(&id) {
                    open_link.send_message(Msg::OpenSource(id));
                }
            });
            let _ = element
                .add_event_listener_with_callback("wiki-open", open.as_ref().unchecked_ref());
            self._graph_open_listener = Some(open);
        } else if self.graph_ref.cast::<HtmlElement>().is_none() {
            self._graph_listener = None;
            self._graph_open_listener = None;
        }
        body_class(
            OVERLAY_CLASS,
            !self.document.is_empty() || !self.source.is_empty() || self.media.is_some(),
        );
        if self.focus_article
            && let Some(article) = self.article_ref.cast::<HtmlElement>()
        {
            let _ = article.focus();
            if let Some(reader) = article.parent_element() {
                reader.scroll_into_view();
            }
            self.focus_article = false;
        }
        if self.focus_search
            && let Some(search) = self.search_ref.cast::<HtmlInputElement>()
        {
            let _ = search.focus();
            self.focus_search = false;
        }
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let enabled = self.data["settings"]["wiki_enabled"] == true;
        let auto = self.data["settings"]["wiki_auto_update"] == true;
        let link = ctx.link();
        html! {<section class={classes!("wiki-page","wiki-shell",self.fullscreen.then_some("is-fullscreen"),self.show_graph.then_some("is-map"))} data-testid="wiki-shell">
            <header class="wiki-bar" aria-label={t("Wiki toolbar")}>
                <div class="wiki-bar-title"><span class="feature-mark">{icon("book")}</span><h1>{t("Wiki")}</h1></div>
                <div class="wiki-search">{icon("search")}<input ref={self.search_ref.clone()} type="search" aria-label={t("Search Wiki")} placeholder={t("Search, or #tag")} value={self.query.clone()} autocomplete="off" oninput={link.batch_callback(|e:InputEvent|vec![Msg::SearchFocus(true),Msg::Query(e.target_unchecked_into::<HtmlInputElement>().value())])} onblur={link.callback(|_|Msg::SearchFocus(false))}/>if !self.query.is_empty(){<button class="wiki-clear" aria-label={t("Clear Wiki search")} onclick={link.callback(|_|Msg::ClearQuery)}>{icon("close")}</button>}{self.suggestions(ctx)}</div>
                <div class="wiki-bar-actions">
                    <div class="wiki-view-toggle" role="group" aria-label={t("Wiki view")}><button type="button" class={classes!("chip-button",(!self.show_graph).then_some("is-active"))} aria-pressed={(!self.show_graph).to_string()} onclick={link.callback(|_|Msg::ShowFeed)} data-testid="wiki-feed-toggle">{icon("message")}{t("Feed")}</button><button type="button" class={classes!("chip-button",self.show_graph.then_some("is-active"))} aria-pressed={self.show_graph.to_string()} onclick={link.callback(|_|Msg::ShowMap)} data-testid="wiki-graph-toggle">{icon("spark")}{t("Map")}</button></div>
                    <button type="button" class={classes!("chip-button","wiki-refresh",self.refreshing.then_some("is-busy"))} aria-label={t("Refresh Wiki")} title={t("Refresh Wiki")} data-testid="wiki-refresh" onclick={link.callback(|_|Msg::Refresh)}>{icon("refresh")}</button>
                    <button type="button" class={classes!("chip-button","wiki-fullscreen-toggle",self.fullscreen.then_some("is-active"))} aria-pressed={self.fullscreen.to_string()} data-testid="wiki-fullscreen" title={if self.fullscreen{t("Press Esc to leave fullscreen")}else{t("Only the Wiki on screen")}} onclick={link.callback(|_|Msg::ToggleFullscreen)}>{icon(if self.fullscreen{"close"}else{"screen"})}<span>{if self.fullscreen{t("Exit fullscreen")}else{t("Fullscreen")}}</span></button>
                    if enabled && !auto {<button type="button" class="chip-button" aria-label={t("Build from saved sources")} disabled={self.busy} onclick={link.callback(|_|Msg::Build)}>{icon("refresh")}<span>{if self.busy{t("Organizing…")} else{t("Update Wiki")} }</span></button>}
                    <a class="chip-button wiki-settings-link" aria-label={t("Wiki & TextEdit settings")} title={t("Wiki & TextEdit settings")} href="/assistance">{icon("settings")}</a>
                </div>
            </header>
            if !self.error.is_empty(){<p class="notes-error" role="alert">{t(&self.error)}</p>}
            <div class="wiki-body">
                {self.rail(ctx)}
                <div class="wiki-main" ref={self.main_ref.clone()}>
                    // The collection stays mounted under any reader, so Back finds
                    // the feed's scroll, the map's camera and the search as they were.
                    <FadeSwap identity="collection" on_swap={link.callback(|_|Msg::ViewReady)}><div class="wiki-collection" aria-hidden={(!self.document.is_empty()||!self.source.is_empty()).to_string()} inert={(!self.document.is_empty()||!self.source.is_empty()).then_some("")}>
                        {self.heading(ctx)}
                        if !self.focus.is_empty() {{self.related_panel(ctx)}}
                        if self.show_graph {{self.map(ctx)}}
                        else {{self.feed(ctx)}}
                    </div></FadeSwap>
                    // One layer above the collection: a message, a tag document, or
                    // nothing. It fades out before it goes and fades in as it comes.
                    <div class="wiki-layer"><FadeSwap identity={if !self.source.is_empty(){format!("source:{}",self.source)}else if !self.document.is_empty(){format!("article:{}",self.document)}else{"none".to_string()}} on_swap={link.callback(|_|Msg::ViewReady)}>
                        if !self.source.is_empty() {<div class="wiki-overlay" data-testid="wiki-overlay">{self.source_reader(ctx)}</div>}
                        else if !self.document.is_empty() {<div class="wiki-overlay" data-testid="wiki-overlay">{self.reader(ctx)}</div>}
                    </FadeSwap></div>
                </div>
            </div>
            if let Some(media)=&self.media {{self.lightbox(ctx,media)}}
        </section>}
    }
}

impl Wiki {
    /// How far the page (and, in fullscreen, the Wiki's own pane) is scrolled.
    fn scroll_position(&self) -> (f64, i32) {
        let window_y = web_sys::window()
            .and_then(|w| w.scroll_y().ok())
            .unwrap_or(0.0);
        let main_top = self
            .main_ref
            .cast::<HtmlElement>()
            .map(|m| m.scroll_top())
            .unwrap_or(0);
        (window_y, main_top)
    }
    /// Close every reader and picture; the collection beneath is left alone.
    fn close_layers(&mut self) {
        self.document.clear();
        self.page = Value::Null;
        self.source.clear();
        self.source_page = Value::Null;
        self.media = None;
    }
    /// A search that found something is worth offering again.
    fn remember_query(&mut self) {
        let query = self.query.trim().to_string();
        if query.is_empty() || self.data["total"].as_u64().unwrap_or(0) == 0 {
            return;
        }
        let lower = query.to_lowercase();
        // Typing "writing" slowly must not keep "wr", "wri" and "writ" as well.
        if self.queries.iter().any(|q| {
            let q = q.to_lowercase();
            q.starts_with(&lower) && q != lower
        }) {
            return;
        }
        self.queries
            .retain(|q| !lower.starts_with(&q.to_lowercase()));
        self.queries.insert(0, query);
        self.queries.truncate(REMEMBERED_QUERIES);
        remember(
            QUERIES_KEY,
            &serde_json::to_string(&self.queries).unwrap_or_default(),
        );
    }
    /// Under the search box while typing: earlier searches and tags that match
    /// the first letters. It closes on blur, on a pick, and never opens on a
    /// focus the page gave the box itself.
    fn suggestions(&self, ctx: &Context<Self>) -> Html {
        let typed = self.query.trim().to_lowercase();
        // Nothing typed, nothing offered: the list must not cover the collection.
        if !self.search_open || typed.is_empty() {
            return Html::default();
        }
        let tags = self.data["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t["tag"].as_str())
            .map(|t| format!("#{t}"));
        let mut offered: Vec<String> = vec![];
        for candidate in self.queries.iter().cloned().chain(tags) {
            let lower = candidate.to_lowercase();
            let matches = lower != typed
                && (lower.contains(&typed) || lower.trim_start_matches('#').starts_with(&typed));
            if matches && !offered.iter().any(|o| o.eq_ignore_ascii_case(&candidate)) {
                offered.push(candidate);
            }
            if offered.len() >= 8 {
                break;
            }
        }
        if offered.is_empty() {
            return Html::default();
        }
        html! {<ul class="wiki-suggest" role="listbox" aria-label={t("Search suggestions")} data-testid="wiki-suggest">
            {for offered.into_iter().map(|text|{let pick=text.clone();html!{<li key={text.clone()} role="option"><button type="button" onmousedown={Callback::from(|e:MouseEvent|e.prevent_default())} onclick={ctx.link().callback(move |_|Msg::Suggest(pick.clone()))}>{icon(if text.starts_with('#'){"hash"}else{"search"})}<span>{text}</span></button></li>}})}
        </ul>}
    }
    /// Tags with counts; one click filters the feed or the map.
    fn rail(&self, ctx: &Context<Self>) -> Html {
        let tags = self.data["tags"].as_array().cloned().unwrap_or_default();
        let total: u64 = tags.iter().map(|t| t["count"].as_u64().unwrap_or(0)).sum();
        let all = self.tag.is_empty();
        html! {<aside class="wiki-rail" aria-label={t("Tags")}>
            <button type="button" class={classes!("wiki-rail-tag","wiki-rail-all",all.then_some("is-active"))} aria-pressed={all.to_string()} onclick={ctx.link().callback(|_|Msg::Tag(String::new()))}>{icon("book")}<span>{t("Everything")}</span><small>{total}</small></button>
            if tags.is_empty() {<p class="wiki-rail-empty">{t("Tags appear here as you keep things.")}</p>}
            if tags.iter().any(|t|t["type"]==true) {<p class="wiki-rail-group">{t("Types")}</p>
            <ul class="wiki-rail-types">{for tags.iter().filter(|t|t["type"]==true).map(|tag|{let name=tag["tag"].as_str().unwrap_or("").to_owned();let active=name==self.tag;let count=tag["count"].as_u64().unwrap_or(0);let pick=name.clone();html!{<li key={name.clone()}><button type="button" class={classes!("wiki-rail-tag","is-type",active.then_some("is-active"))} aria-pressed={active.to_string()} onclick={ctx.link().callback(move |_|Msg::Tag(pick.clone()))} ondblclick={ctx.link().callback(|_|Msg::ShowFeed)}><span>{"#"}{name}</span><small>{count}</small></button></li>}})}</ul>}
            if tags.iter().any(|t|t["category"]==true) {<p class="wiki-rail-group">{t("Categories")}</p>
            <ul class="wiki-rail-categories">{for tags.iter().filter(|t|t["category"]==true).map(|tag|{let name=tag["tag"].as_str().unwrap_or("").to_owned();let active=name==self.tag;let count=tag["count"].as_u64().unwrap_or(0);let pick=name.clone();html!{<li key={name.clone()}><button type="button" class={classes!("wiki-rail-tag","is-category",active.then_some("is-active"))} aria-pressed={active.to_string()} onclick={ctx.link().callback(move |_|Msg::Tag(pick.clone()))} ondblclick={ctx.link().callback(|_|Msg::ShowFeed)}><span>{"#"}{name}</span><small>{count}</small></button></li>}})}</ul>
            <p class="wiki-rail-group">{t("Tags")}</p>}
            <ul>{for tags.iter().filter(|t|t["category"]!=true&&t["type"]!=true).map(|tag|{let name=tag["tag"].as_str().unwrap_or("").to_owned();let active=name==self.tag;let count=tag["count"].as_u64().unwrap_or(0);let pick=name.clone();html!{<li key={name.clone()}><button type="button" class={classes!("wiki-rail-tag",active.then_some("is-active"))} aria-pressed={active.to_string()} onclick={ctx.link().callback(move |_|Msg::Tag(pick.clone()))} ondblclick={ctx.link().callback(|_|Msg::ShowFeed)}><span>{"#"}{name}</span><small>{count}</small></button></li>}})}</ul>
        </aside>}
    }
    /// What the collection shows right now: everything, a search, or one tag with
    /// the tags that travel with it.
    fn heading(&self, ctx: &Context<Self>) -> Html {
        let total = self.data["total"].as_u64().unwrap_or(0);
        let count = tf(
            if total == 1 {
                "{count} message"
            } else {
                "{count} messages"
            },
            &[("count", total.to_string())],
        );
        let related = self.data["related_tags"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let document = self.data["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| t["tag"] == self.tag.as_str())
            .and_then(|t| t["document"].as_str())
            .map(str::to_owned);
        html! {<div class="wiki-library-heading">
            <div>
                if self.tag.is_empty() {<h2>{if self.query.trim().is_empty(){t("All messages")} else{t("Search results")} }</h2>}
                else {<h2 class="wiki-active-tag"><span>{"#"}{&self.tag}</span></h2><button type="button" class="wiki-close-tag" data-testid="wiki-close-tag" aria-label={tf("Close #{tag}", &[("tag",self.tag.clone())])} title={tf("Close #{tag}", &[("tag",self.tag.clone())])} onclick={ctx.link().callback(|_|Msg::CloseTag)}>{icon("close")}<span>{t("Close")}</span></button>}
                <span class="collection-count">{count}</span>
                if !self.tag.is_empty() {
                    if let Some(id)=document {<button type="button" class="text-button" data-testid="wiki-open-document" onclick={ctx.link().callback(move |_|Msg::OpenDocument(id.clone()))}>{icon("file")}{t("Open as document")}</button>}
                    <button type="button" class="text-button" data-testid="wiki-clear-tag" onclick={ctx.link().callback(|_|Msg::Tag(String::new()))}>{icon("close")}{t("Show everything")}</button>
                    // A type or a category is what a message is; only a real tag can go.
                    if !causewaybay_shared::is_fixed_tag(&self.tag) && self.tag!="inbox" {<button type="button" class="text-button danger-text" data-testid="wiki-delete-tag" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::AskDeleteTag(true))}>{icon("trash")}{t("Delete tag")}</button>}
                }
            </div>
            if self.confirm_delete {<div class="wiki-delete-tag" role="alertdialog" aria-label={t("Delete tag")} data-testid="wiki-delete-tag-dialog"><span>{tf("Remove #{tag} from every message? The messages stay; only the tag goes, and it will not come back on its own.", &[("tag",self.tag.clone())])}</span><button type="button" class="danger-button" data-testid="wiki-delete-tag-confirm" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::DeleteTag)}>{if self.busy{t("Deleting…")}else{t("Confirm delete tag")}}</button><button type="button" class="text-button" disabled={self.busy} onclick={ctx.link().callback(|_|Msg::AskDeleteTag(false))}>{t("Cancel")}</button></div>}
            if !related.is_empty() {<div class="wiki-related-tags" aria-label={t("Also tagged")}><span>{t("Also tagged")}</span>{for related.iter().map(|tag|{let name=tag["tag"].as_str().unwrap_or("").to_owned();let pick=name.clone();html!{<button type="button" key={name.clone()} class="wiki-tag" onclick={ctx.link().callback(move |_|Msg::Tag(pick.clone()))}>{"#"}{name}<small>{tag["count"].as_u64().unwrap_or(0)}</small></button>}})}</div>}
            <p class="wiki-local-note">{icon("shield")}{t("Tags stay on-device in On-device and Hybrid modes. Cloud AI mode can use your cloud model. Search and storage stay on this computer.")}</p>
        </div>}
    }
    fn feed(&self, ctx: &Context<Self>) -> Html {
        let enabled = self.data["settings"]["wiki_enabled"] == true;
        let items = self.data["items"].as_array().cloned().unwrap_or_default();
        let total = self.data["total"].as_u64().unwrap_or(0) as usize;
        let size = self.data["page_size"].as_u64().unwrap_or(30) as usize;
        let searching = !self.query.trim().is_empty() || !self.tag.is_empty();
        if self.data.is_null() {
            return html! {<div class="wiki-empty" role="status"><span class="spinner"/>{t("Opening your collection…")} </div>};
        }
        if items.is_empty() {
            return html! {<div class="wiki-empty"><span class="feature-mark">{icon(if searching{"search"}else{"book"})}</span><h2>{if !enabled{t("Wiki is off")} else if searching{t("No matching messages.")} else{t("Every collection starts somewhere.")} }</h2><p>{if !enabled{t("Enable Wiki in settings to start your collection.")} else if searching{t("Try a word from the message, or another tag.")} else{t("Put anything into New thought or a chat. It is tagged and organized here, one message at a time.")} }</p>
                if !enabled {<a class="secondary-button" href="/assistance">{t("Set up your Wiki")}{icon("arrow")}</a>}
                else if !self.query.trim().is_empty() {<button class="secondary-button" onclick={ctx.link().batch_callback(|_|vec![Msg::Tag(String::new()),Msg::Query(String::new())])}>{t("Show all messages")} {icon("arrow")}</button>}
                else {<a class="secondary-button" href="/">{t("Capture your first thought")}{icon("arrow")}</a>}
            </div>};
        }
        html! {<>
            <div class="wiki-feed" data-testid="wiki-feed">{for items.iter().enumerate().map(|(i,item)| self.card(ctx,item,i))}</div>
            if total>size {<div class="wiki-pagination"><button disabled={self.offset==0} onclick={ctx.link().callback({let o=self.offset.saturating_sub(size);move |_|Msg::Offset(o)})}>{t("Previous")} </button><span>{tf("{start}–{end} of {total} messages", &[("start",(self.offset+1).to_string()),("end",(self.offset+items.len()).min(total).to_string()),("total",total.to_string())])}</span><button disabled={self.offset+size>=total} onclick={ctx.link().callback({let o=self.offset+size;move |_|Msg::Offset(o)})}>{t("Next")} </button></div>}
        </>}
    }
    /// One message: what it says, its tags, where it was said, what it cost.
    fn card(&self, ctx: &Context<Self>, item: &Value, index: usize) -> Html {
        let id = item["id"].as_str().unwrap_or("").to_owned();
        let title = item["title"].as_str().unwrap_or("").to_owned();
        let url = item["url"].as_str().unwrap_or("#").to_owned();
        let thumbnail = item["thumbnail"].as_str().map(str::to_owned);
        let tags = item["tags"].as_array().cloned().unwrap_or_default();
        let room = &item["room"];
        let calls = item["cloud_calls"].as_u64().unwrap_or(0);
        let usd = item["estimated_usd"].as_f64();
        let focused = id == self.focus;
        let focus_id = id.clone();
        let media = item["media"].as_array().cloned().unwrap_or_default();
        let has_video = media.iter().any(|m| m["kind"] == "video");
        // The thumbnail plays or shows what it pictures, in place; the title reads the message.
        let open_media = {
            let (items, title, id) = (media.clone(), title.clone(), id.clone());
            ctx.link().callback(move |_| {
                if items.is_empty() {
                    Msg::OpenSource(id.clone())
                } else {
                    Msg::OpenMedia(items.clone(), 0, title.clone(), id.clone())
                }
            })
        };
        let open_source = {
            let id = id.clone();
            ctx.link().callback(move |_| Msg::OpenSource(id.clone()))
        };
        let _ = &url;
        html! {<article key={id.clone()} data-id={id.clone()} class={classes!("wiki-card",item["kind"].as_str().map(|k|format!("is-{k}")),focused.then_some("is-active"))} style={format!("--reveal-delay:{}ms",index.min(5)*35)} aria-label={title.clone()}>
            if let Some(src)=thumbnail {<button type="button" class={classes!("wiki-card-thumb",has_video.then_some("is-video"))} aria-label={if has_video{tf("Play {name}", &[("name",title.clone())])}else{tf("Show {name}", &[("name",title.clone())])}} data-testid="wiki-card-thumb" onclick={open_media}><img src={src} alt="" loading="lazy"/>if has_video{<span class="video-play">{icon("play")}</span>}</button>}
            <div class="wiki-card-body">
                <div class="wiki-card-head"><button type="button" class="wiki-card-title" data-testid="wiki-card-title" onclick={open_source.clone()}>{title}</button><time datetime={item["created_at"].as_str().unwrap_or("").to_owned()}>{crate::short_date(item["created_at"].as_str().unwrap_or(""))}</time></div>
                if item["tidied"]==true && item["answer_snippet"].as_str().is_some_and(|a|!a.is_empty()) {
                    // The organized form is the body; the words as written stay in the chat and the source.
                    <p class="wiki-card-snippet is-tidied" data-testid="wiki-card-answer">{item["answer_snippet"].as_str().unwrap_or("").trim_start_matches('#').trim()}</p>
                    <details class="wiki-card-original"><summary>{t("Original")}</summary><p>{item["snippet"].as_str().unwrap_or("")}</p></details>
                } else {
                    <p class="wiki-card-snippet">{item["snippet"].as_str().unwrap_or("")}</p>
                    if let Some(answer)=item["answer_snippet"].as_str().filter(|a|!a.is_empty()) {<p class="wiki-card-answer" data-testid="wiki-card-answer"><span>{icon("spark")}{t("Answer")}</span>{answer}</p>}
                }
                <div class="wiki-card-tags">{for tags.iter().filter_map(|v|v.as_str()).map(|tag|{let name=tag.to_owned();let active=name==self.tag;let pick=name.clone();let category=causewaybay_shared::is_category(&name);let kind=causewaybay_shared::is_type(&name);html!{<button type="button" key={name.clone()} class={classes!("wiki-tag",kind.then_some("is-type"),category.then_some("is-category"),active.then_some("is-active"))} aria-pressed={active.to_string()} onclick={ctx.link().callback(move |_|Msg::Tag(pick.clone()))}>{"#"}{name}</button>}})}</div>
                <div class="wiki-card-foot">
                    if room.is_object() {<a class="wiki-room" href={room["url"].as_str().unwrap_or("/chat").to_owned()} data-testid="wiki-room">{icon("message")}<span>{room["title"].as_str().unwrap_or("")}</span></a>}
                    else {<span class="wiki-room is-library">{icon("book")}<span>{t("Library")}</span></span>}
                    if calls>0 {<span class="wiki-cloud" title={t("Not a provider bill.")}>{icon("globe")}<span>{tf("{count} cloud", &[("count",calls.to_string())])}{usd.map(|v|format!(" · ${v:.4}")).unwrap_or_default()}</span></span>}
                    else {<span class="wiki-local">{icon("shield")}<span>{t("Local only")}</span></span>}
                    if item["has_answer"]==true {<span class="wiki-answered">{icon("spark")}<span>{t("AI answer included")}</span></span>}
                    <button type="button" class={classes!("text-button","wiki-related-button",focused.then_some("is-active"))} aria-pressed={focused.to_string()} onclick={ctx.link().callback(move |_|Msg::Focus(focus_id.clone()))}>{icon("spark")}{t("Related")}</button>
                    <button type="button" class="text-button wiki-open" onclick={open_source}>{t("Open")}{icon("arrow")}</button>
                </div>
            </div>
        </article>}
    }
    /// Messages that share tags with, or sit near in meaning to, the focused one.
    fn related_panel(&self, ctx: &Context<Self>) -> Html {
        let related = self.related["related"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let title = self.related["item"]["title"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        html! {<section class="wiki-related" aria-label={t("Related messages")} data-testid="wiki-related">
            <div class="wiki-related-head"><h3>{icon("spark")}{t("Related to")}{" "}<em>{title}</em></h3><button type="button" class="text-button" aria-label={t("Close related messages")} onclick={ctx.link().callback(|_|Msg::ClearFocus)}>{icon("close")}{t("Close")}</button></div>
            if self.related.is_null() {<div class="wiki-empty is-inline" role="status"><span class="spinner"/>{t("Finding relations…")}</div>}
            else if related.is_empty() {<p class="wiki-related-none">{t("Nothing shares a tag with this message yet.")}</p>}
            else {<div class="wiki-feed is-related">{for related.iter().enumerate().map(|(i,item)| self.card(ctx,item,i))}</div>}
        </section>}
    }
    fn map(&self, ctx: &Context<Self>) -> Html {
        let tags: Vec<Value> = self.graph["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|n| n["kind"] == "tag")
            .cloned()
            .collect();
        html! {<section class={classes!("wiki-graph-panel",self.map_full.then_some("is-full"))} aria-label={t("Wiki map")} data-testid="wiki-graph-panel">
            <div class="wiki-graph-tools" role="group" aria-label={t("Map controls")}>
                <button type="button" class="chip-button" aria-label={t("Zoom in")} title={t("Zoom in")} data-testid="wiki-zoom-in" onclick={ctx.link().callback(|_|Msg::Zoom(-1))}>{icon("zoom-in")}</button>
                <button type="button" class="chip-button" aria-label={t("Zoom out")} title={t("Zoom out")} data-testid="wiki-zoom-out" onclick={ctx.link().callback(|_|Msg::Zoom(1))}>{icon("zoom-out")}</button>
                <button type="button" class={classes!("chip-button",self.map_full.then_some("is-active"))} aria-pressed={self.map_full.to_string()} aria-label={if self.map_full{t("Exit map fullscreen")}else{t("Map fullscreen")}} title={if self.map_full{t("Exit map fullscreen")}else{t("Map fullscreen")}} data-testid="wiki-map-full" onclick={ctx.link().callback(|_|Msg::ToggleMapFull)}>{icon(if self.map_full{"close"}else{"expand"})}</button>
            </div>
            <wiki-graph ref={self.graph_ref.clone()} data-graph={self.graph.to_string()} data-selected={if !self.source.is_empty(){self.source.clone()}else if self.focus.is_empty(){format!("tag:{}",self.tag)}else{self.focus.clone()}} data-testid="wiki-graph"></wiki-graph>
            <ul class="wiki-graph-legend" aria-label={t("Map legend")} data-testid="wiki-graph-legend"><li><i style="background:#a98ac7"></i>{t("Tag")}</li><li><i style="background:#7aaea1"></i>{t("Q&A")}</li><li><i style="background:#8fb38a"></i>{t("Note")}</li><li><i style="background:#6f8fbf"></i>{t("Source code")}</li><li><i style="background:#d9b26b"></i>{t("Photo")}</li><li><i style="background:#c98aa6"></i>{t("Video")}</li></ul>
            <p class="wiki-graph-hint">{t("Tags are the large spheres, messages the small ones, and pictures show their thumbnail. Drag to rotate; scroll, pinch or use the buttons to zoom. Tap a message to read it, a picture or a clip to play it, a tag to filter. Double-tap a tag for its messages.")}</p>
            <ul class="wiki-graph-fallback" aria-label={t("Tags in the map")}>{for tags.iter().map(|n|{let name=n["title"].as_str().unwrap_or("").to_owned();let active=name==self.tag;let pick=name.clone();html!{<li key={name.clone()} class={classes!(active.then_some("is-active"))}><button type="button" class="text-button" aria-pressed={active.to_string()} onclick={ctx.link().callback(move |_|Msg::Tag(pick.clone()))}>{"#"}{name}</button><small>{" · "}{n["size"].as_u64().unwrap_or(0)}</small></li>}})}</ul>
        </section>}
    }
    /// A tag's derived document: every message under that tag as one readable page.
    fn reader(&self, ctx: &Context<Self>) -> Html {
        let count = self.page["sources"].as_array().map(Vec::len).unwrap_or(0);
        html! {<section class="wiki-reading">
            {self.reader_nav(ctx,"file","TAG DOCUMENT")}
            if self.page.is_null(){<div class="wiki-empty" role="status"><span class="spinner"/>{t("Opening page…")} </div>}
            else{<article class="wiki-article" ref={self.article_ref.clone()} tabindex="-1" aria-label={self.page["title"].as_str().unwrap_or("Wiki page").to_owned()}><div class="wiki-source-count">{icon("book")}{tf(if count==1{"{count} linked source"}else{"{count} linked sources"}, &[("count", count.to_string())])}<span>{t("Always connected to the original")} </span></div>{self.usage_summary()}<div class="markdown" onclick={ctx.link().callback(Msg::ReaderClick)}>{crate::markdown(self.page["markdown"].as_str().unwrap_or(""))}</div></article>}
        </section>}
    }
    /// One big Back above every reader; Esc does the same.
    fn reader_nav(&self, ctx: &Context<Self>, symbol: &str, label: &str) -> Html {
        html! {<nav class="wiki-reader-nav" aria-label={t("Wiki page navigation")}><button class="wiki-back" data-testid="wiki-back" aria-label={t("Back to Wiki")} title={t("Back to Wiki")} onclick={ctx.link().callback(|_|Msg::CloseDocument)}>{icon("arrow")}<span>{t("Back")}</span></button><span>{icon(symbol)}{t(label)} </span></nav>}
    }
    /// One message read in place: what it pictures, what was said, what the AI
    /// answered, where it came from. Nothing here leaves the Wiki.
    fn source_reader(&self, ctx: &Context<Self>) -> Html {
        let m = &self.source_page;
        let title = m["title"].as_str().unwrap_or("").to_owned();
        let media = m["media"].as_array().cloned().unwrap_or_default();
        let tags = m["tags"].as_array().cloned().unwrap_or_default();
        let room = &m["room"];
        let calls = m["cloud_calls"].as_u64().unwrap_or(0);
        let answer = m["answer"].as_str().unwrap_or("").trim().to_owned();
        let input = m["input"].as_str().unwrap_or("").to_owned();
        let id = self.source.clone();
        html! {<section class="wiki-reading wiki-source" data-testid="wiki-source">
            {self.reader_nav(ctx,"message","MESSAGE")}
            if m.is_null(){<div class="wiki-empty" role="status"><span class="spinner"/>{t("Opening message…")} </div>}
            else{<article class="wiki-article" ref={self.article_ref.clone()} tabindex="-1" aria-label={title.clone()}>
                <h1 class="wiki-source-title">{&title}</h1>
                <p class="wiki-source-meta"><time datetime={m["created_at"].as_str().unwrap_or("").to_owned()}>{crate::short_date(m["created_at"].as_str().unwrap_or(""))}</time>
                    if room.is_object() {<a class="wiki-room" href={room["url"].as_str().unwrap_or("/chat").to_owned()} data-testid="wiki-room">{icon("message")}<span>{room["title"].as_str().unwrap_or("")}</span></a>}
                    if calls>0 {<span class="wiki-cloud">{icon("globe")}<span>{tf("{count} cloud", &[("count",calls.to_string())])}</span></span>} else {<span class="wiki-local">{icon("shield")}<span>{t("Local only")}</span></span>}
                </p>
                if !media.is_empty() {<div class="wiki-source-media" data-testid="wiki-source-media">{for media.iter().enumerate().map(|(i,item)|{
                    let kind=item["kind"].as_str().unwrap_or("");let url=item["url"].as_str().unwrap_or("").to_owned();let poster=item["poster"].as_str().unwrap_or("").to_owned();let name=item["name"].as_str().unwrap_or("").to_owned();
                    let open={let (items,title,memo)=(media.clone(),title.clone(),id.clone());ctx.link().callback(move |_|Msg::OpenMedia(items.clone(),i,title.clone(),memo.clone()))};
                    // The first clip plays as soon as the message opens; the rest wait for a tap.
                    if kind=="video" {html!{<video key={url.clone()} controls=true autoplay={i==0} playsinline=true preload="metadata" src={url} poster={poster} aria-label={name}/>}}
                    else {html!{<button key={url.clone()} type="button" class="wiki-media-open" aria-label={tf("Show {name}", &[("name",name)])} onclick={open}><img src={url} alt="" loading="lazy"/></button>}}
                })}</div>}
                // The tags are the user's to change here too: a picture or a clip
                // kept without a word is tagged the same way as any message.
                <crate::tags::TagEditor language={ctx.props().language} memo_id={id.clone()} tags={tags.iter().filter_map(|v|v.as_str().map(str::to_owned)).collect::<Vec<String>>()} pending=true pick={ctx.link().callback(Msg::Tag)} saved={ctx.link().callback({let id=id.clone();move |tags|Msg::SourceTagged(id.clone(),tags)})}/>
                <div class="markdown wiki-source-text" onclick={ctx.link().callback(Msg::ReaderClick)}>{crate::markdown(&input)}</div>
                if !answer.is_empty() {<section class="wiki-source-answer" aria-label={t("Answer")}><h2>{icon("spark")}{t("Answer")}</h2><div class="markdown" onclick={ctx.link().callback(Msg::ReaderClick)}>{crate::markdown(&answer)}</div></section>}
                <div class="wiki-source-foot">
                    <button type="button" class="text-button" onclick={ctx.link().batch_callback({let id=id.clone();move |_|vec![Msg::CloseDocument,Msg::Focus(id.clone())]})}>{icon("spark")}{t("Related")}</button>
                    <a class="text-button" href={format!("/memos/{id}")} data-testid="wiki-open-library">{icon("book")}{t("Open in library")}{icon("arrow")}</a>
                </div>
            </article>}
        </section>}
    }
    /// A picture or a clip over the whole page. Backdrop, the big Back and Esc close it.
    fn lightbox(&self, ctx: &Context<Self>, media: &Media) -> Html {
        let item = &media.items[media.index];
        let kind = item["kind"].as_str().unwrap_or("image");
        let url = item["url"].as_str().unwrap_or("").to_owned();
        let poster = item["poster"].as_str().unwrap_or("").to_owned();
        let name = item["name"].as_str().unwrap_or("").to_owned();
        let many = media.items.len() > 1;
        html! {<div class="wiki-lightbox" role="dialog" aria-modal="true" aria-label={media.title.clone()} data-testid="wiki-lightbox" onclick={ctx.link().callback(|_|Msg::CloseMedia)}>
            <button type="button" class="wiki-back wiki-lightbox-back" data-testid="wiki-media-close" aria-label={t("Back to Wiki")} onclick={ctx.link().callback(|e:MouseEvent|{e.stop_propagation();Msg::CloseMedia})}>{icon("arrow")}<span>{t("Back")}</span></button>
            <figure class="wiki-lightbox-body" onclick={Callback::from(|e:MouseEvent|e.stop_propagation())}>
                if kind=="video" {<video key={url.clone()} controls=true autoplay=true playsinline=true preload="metadata" src={url} poster={poster} aria-label={if name.is_empty(){media.title.clone()}else{name.clone()}} data-testid="wiki-lightbox-video"/>}
                else {<img key={url.clone()} src={url} alt={if name.is_empty(){media.title.clone()}else{name.clone()}} data-testid="wiki-lightbox-image"/>}
                <figcaption><strong>{&media.title}</strong>if !name.is_empty(){<span>{name}</span>}if many{<span class="wiki-lightbox-count">{format!("{} / {}",media.index+1,media.items.len())}</span>}
                    if !media.memo.is_empty() && media.memo!=self.source {<button type="button" class="wiki-lightbox-read" data-testid="wiki-lightbox-read" onclick={ctx.link().callback({let id=media.memo.clone();move |_|Msg::OpenSource(id.clone())})}>{icon("message")}{t("Read message")}</button>}
                </figcaption>
            </figure>
            if many {<button type="button" class="wiki-lightbox-step is-prev" aria-label={t("Previous")} onclick={ctx.link().callback(|e:MouseEvent|{e.stop_propagation();Msg::MediaStep(-1)})}>{icon("arrow")}</button><button type="button" class="wiki-lightbox-step is-next" aria-label={t("Next")} onclick={ctx.link().callback(|e:MouseEvent|{e.stop_propagation();Msg::MediaStep(1)})}>{icon("arrow")}</button>}
        </div>}
    }
    /// Cloud calls, tokens, and estimated cost behind the open page, summed over
    /// its sources; local-only pages say so.
    fn usage_summary(&self) -> Html {
        let sources = self.page["sources"].as_array().cloned().unwrap_or_default();
        let calls: u64 = sources
            .iter()
            .map(|s| s["cloud_calls"].as_u64().unwrap_or(0))
            .sum();
        if calls == 0 {
            return html! {<p class="wiki-usage is-local" data-testid="wiki-usage">{icon("shield")}{t("No cloud AI was used for this page.")}</p>};
        }
        // Providers that report no tokens leave the counts and cost unknown.
        let known = sources.iter().any(|s| s["input_tokens"].is_u64());
        let input: u64 = sources
            .iter()
            .map(|s| s["input_tokens"].as_u64().unwrap_or(0))
            .sum();
        let output: u64 = sources
            .iter()
            .map(|s| s["output_tokens"].as_u64().unwrap_or(0))
            .sum();
        let priced = sources.iter().any(|s| s["estimated_usd"].is_f64());
        let usd: f64 = sources
            .iter()
            .map(|s| s["estimated_usd"].as_f64().unwrap_or(0.0))
            .sum();
        let tokens = if known {
            format!("{input} / {output}")
        } else {
            "— / —".to_string()
        };
        let cost = if priced {
            format!("${usd:.4}")
        } else {
            t("Cost unknown")
        };
        html! {<p class="wiki-usage is-cloud" data-testid="wiki-usage">{icon("globe")}<span>{t("Cloud AI")}{" · "}{calls}{" "}{t("AI requests")}{" · "}{t("tokens")}{" "}{tokens}{" · "}{t("est.")}{" "}{cost}</span><small>{t("Not a provider bill.")}</small></p>}
    }
}
