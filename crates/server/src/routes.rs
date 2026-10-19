//! Router construction: the JSON API, WebSocket upgrades, single-page app
//! paths and the security headers applied to every response.
use super::*;

pub fn api_routes() -> Router<Arc<AppState>> {
    Router::new()
        .merge(upload_routes())
        .route("/admin/users", get(users::list))
        .route(
            "/admin/users/{address}",
            axum::routing::patch(users::update),
        )
        .route("/notifications", get(notifications::events))
        .route("/settings", get(get_settings).patch(update_settings))
        .route("/memos", get(list_memos).post(save_memo))
        .route(
            "/memos/{id}",
            get(get_memo).patch(update_memo).delete(delete_memo),
        )
        .route(
            "/memos/{id}/image",
            get(memo_image).delete(library::delete_image),
        )
        .route("/memos/{id}/export", get(export_memo))
        .route("/memos/{id}/thumbnail", get(memo_thumbnail))
        .route("/memos/{id}/events", get(events))
        .route("/memos/{id}/cancel", post(cancel))
        .route("/think", post(think))
        .route("/rooms", get(chat::rooms).post(chat::create))
        .route(
            "/rooms/thought",
            get(chat::current_thought).post(chat::thought),
        )
        .route("/rooms/{id}/clear", post(chat::clear_room))
        .route("/user/data", axum::routing::delete(chat::delete_all))
        .route("/rooms/{id}/snapshot", post(chat::snapshot))
        .route("/rooms/{id}/capture/now", post(continuous::capture_now))
        .route(
            "/rooms/{id}",
            axum::routing::patch(chat::rename).delete(chat::delete_room),
        )
        .route(
            "/rooms/{room}/messages/{id}",
            axum::routing::patch(chat::edit_message).delete(chat::delete_message),
        )
        .route("/rooms/{id}/sync", get(chat::sync))
        .route("/rooms/{id}/events", get(chat::events))
        .route(
            "/rooms/{id}/capture",
            get(continuous::status)
                .post(continuous::start)
                .delete(continuous::stop),
        )
        .route(
            "/ai/preferences",
            get(ai_preferences::get).patch(ai_preferences::update),
        )
        .route(
            "/capture/choices",
            get(capture_choices::get).patch(capture_choices::update),
        )
        .route(
            "/ai/models",
            axum::routing::patch(ai_preferences::update_model),
        )
        .route(
            "/rooms/{id}/notes",
            get(notes::status).post(notes::start).delete(notes::stop),
        )
        .route("/rooms/{id}/notes/approval", post(notes::decide))
        .route(
            "/audio",
            get(audio::status).post(audio::start).delete(audio::stop),
        )
        .route("/audio/solve", post(audio::solve))
        .route("/audio/events", get(audio::events))
        .route("/audio/note", post(audio::note))
        .route("/audio/room", post(audio::choose_room))
        .route(
            "/invisible/sessions",
            get(invisible_sessions::list).post(invisible_sessions::create),
        )
        .route(
            "/invisible/sessions/{id}",
            get(invisible_sessions::get).patch(invisible_sessions::update),
        )
        .route(
            "/invisible/sessions/{id}/start",
            post(invisible_sessions::start),
        )
        .route(
            "/invisible/sessions/{id}/stop",
            post(invisible_sessions::stop),
        )
        .route(
            "/invisible/sessions/{id}/approval",
            post(invisible_sessions::decide),
        )
        .route(
            "/invisible/sessions/{id}/prompt",
            post(invisible_sessions::prompt),
        )
        .route("/usage", get(usage::get))
        .route("/activity/summary", get(activity::summary_route))
        .route("/activity/events", get(crate::ai_activity::events))
        .route(
            "/assistance/preferences",
            get(assistance::get).patch(assistance::update),
        )
        .route("/wiki", get(wiki::list))
        .route("/wiki/rebuild", post(wiki::rebuild))
        .route("/wiki/graph", get(wiki::graph))
        .route("/wiki/related/{id}", get(wiki::related))
        .route("/wiki/memo/{id}", get(wiki::memo))
        .route("/wiki/tag/{tag}", axum::routing::delete(wiki::remove_tag))
        .route("/wallet", get(wallet::get).patch(wallet::update))
        .route("/wallet/tokens", post(wallet::add_token))
        .route("/wallet/cardano", post(wallet::cardano))
        .route(
            "/wallet/requests/{id}",
            axum::routing::patch(wallet::resolve_request),
        )
        .route(
            "/wallet/balances/{id}",
            axum::routing::patch(wallet::report_balance),
        )
        .route(
            "/wallet/tokens/{network}/{address}",
            axum::routing::delete(wallet::remove_token),
        )
        .route("/code", get(code::list))
        .route("/code/{id}", axum::routing::patch(code::update))
        .route("/coding", get(coding::overview))
        .route("/coding/projects", post(coding::create))
        .route(
            "/coding/projects/{id}",
            get(coding::get)
                .patch(coding::update)
                .delete(coding::remove),
        )
        .route(
            "/coding/projects/{id}/file",
            get(coding::file_get)
                .post(coding::file_put)
                .delete(coding::file_delete),
        )
        .route("/coding/projects/{id}/run", post(coding::run))
        .route("/coding/projects/{id}/test", post(coding::test))
        .route("/coding/projects/{id}/history", get(coding::history_route))
        .route("/coding/projects/{id}/example", post(coding::example_route))
        .route("/coding/projects/{id}/revert", post(coding::revert_route))
        .route("/coding/projects/{id}/ask", post(coding::ask))
        .route(
            "/skills",
            get(coding::skills_route).post(coding::create_skill),
        )
        .route("/skills/{id}/run", post(coding::skill_run))
        .route("/wiki/{id}", get(wiki::page))
        .route("/ai", get(ai_status::get))
        .route("/ai/check", post(ai_status::check))
        .route("/local-ai/models", get(local_models::get))
        .route("/local-ai/models/events", get(local_models::events))
        .route("/admin/setup/check", get(local_models::setup_check))
        .route(
            "/admin/local-ai/models/fetch",
            post(local_models::download).delete(local_models::cancel),
        )
        .route("/capture/batch", post(capture_batch))
        .route("/files", get(files).post(upload))
        .route("/images", get(library::images))
        .route("/videos", get(library::videos))
        .route("/files/{id}/stream", get(library::stream))
        .route("/files/{id}", axum::routing::delete(library::delete_file))
        .route("/files/{id}/process", post(library::process))
        .route("/files/{id}/download", get(download))
        .route("/files/{id}/image", get(file_image))
        .route("/files/{id}/thumbnail", get(file_thumbnail))
        .route("/files/{id}/text", get(file_text))
        .route("/files/{id}/compile", post(compile_file))
        .route("/clients", get(transport::clients))
        .route("/clients/{id}", axum::routing::patch(nodes::rename))
        .route("/dashboard", get(connections::dashboard))
        .route("/devices", get(connections::dashboard))
        .route("/status", get(status::get))
        .route("/download/client.zip", get(client_package::download))
        .layer(DefaultBodyLimit::max(36 * 1024 * 1024))
}
/// Chunked video uploads, with their own body limit: one chunk, never a
/// whole video.
fn upload_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/uploads", post(uploads::begin))
        .route(
            "/uploads/{id}",
            get(uploads::status)
                .patch(uploads::append)
                .delete(uploads::abort),
        )
        .route("/uploads/{id}/finish", post(uploads::finish))
        .layer(DefaultBodyLimit::max(uploads::MAX_CHUNK_BYTES))
}
pub fn app(state: Arc<AppState>, dist: PathBuf) -> Router {
    let api = api_routes()
        .route("/ws", get(transport::browser_upgrade))
        .route("/client/ws", get(transport::client_upgrade))
        .route("/client/events", get(transport::client_events))
        .route_layer(middleware::from_fn_with_state(state.clone(), guard));
    // Browser pages served by the single-page app. `/cloud` and `/status` are
    // kept as aliases of `/autonomous` and `/dashboard` for saved links.
    let pages = [
        "/autonomous",
        "/audio",
        "/cloud",
        "/wiki",
        "/wiki/map",
        "/wiki/tag/{tag}",
        "/wiki/memo/{id}",
        "/wiki/{id}",
        "/code",
        "/coding",
        "/coding/{id}",
        "/skills",
        "/ai",
        "/assistance",
        "/overview",
        "/chat",
        "/library",
        "/files",
        "/images",
        "/videos",
        "/dashboard",
        "/devices",
        "/status",
        "/admin",
        "/quiz",
        "/memos/{id}",
    ]
    .into_iter()
    .fold(Router::new(), |routes, path| {
        routes.route_service(path, ServeFile::new(dist.join("index.html")))
    });
    pages
        .nest("/api", api)
        .route("/api/session", post(login))
        .route("/api/auth/info", get(auth::info))
        .route("/api/auth/challenge", post(auth::challenge))
        .route("/api/auth/verify", post(auth::verify))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/apply", post(users::apply))
        .route("/api/auth/profile", axum::routing::patch(users::profile))
        .route(
            "/api/health",
            get(|| async { Json(json!({"ok":true,"app":"Invisible AI"})) }),
        )
        .fallback_service(
            ServeDir::new(&dist).not_found_service(ServeFile::new(dist.join("index.html"))),
        )
        .layer(DefaultBodyLimit::max(36 * 1024 * 1024))
        .layer(middleware::from_fn(security_headers))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    h.insert("referrer-policy", "no-referrer".parse().unwrap());
    h.insert("x-frame-options", "DENY".parse().unwrap());
    h.insert("content-security-policy", "default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self' https://evm.cronos.org https://evm-t3.cronos.org https://arb1.arbitrum.io https://sepolia-rollup.arbitrum.io https://solana-rpc.publicnode.com https://api.devnet.solana.com https://api.testnet.solana.com wss://indexer.preview.midnight.network wss://indexer.devnet.midnight.network https://chronik.e.cash https://chronik-testnet.fabien.cash; object-src 'none'; base-uri 'self'; frame-ancestors 'none'".parse().unwrap());
    h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    res
}

pub(crate) async fn guard(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    if let Err(e) = auth::authorize(
        &state,
        req.headers(),
        req.method().as_str(),
        req.uri().path(),
    ) {
        return e.into_response();
    }
    next.run(req).await
}
