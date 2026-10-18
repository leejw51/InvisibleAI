use super::*;
use crate::{AppState, credentials::Credentials};
use causewaybay_shared::ChatMessage;
use std::sync::Arc;

fn setup() -> (tempfile::TempDir, Arc<AppState>) {
    let dir = tempfile::tempdir().unwrap();
    let s = AppState::open_with_credentials(dir.path().into(), false, None, Credentials::default())
        .unwrap()
        .for_owner("alice");
    (dir, s)
}
fn memo(s: &AppState, text: &str) -> String {
    let m = s.db.create(text, "question", "apple", "system").unwrap();
    s.db.event(
        &m.id,
        "delta",
        "A generated explanation of decentralized money.",
    )
    .unwrap();
    s.db.finish(&m.id, "question").unwrap();
    // Only what the analysis judges worth remembering joins the Wiki, so the
    // helper judges at once (heuristically, as every test does).
    analyze_sync(&Arc::new(s.clone()), &m.id).unwrap();
    m.id
}
fn pages(s: &AppState) -> Vec<(String, String)> {
    s.db.0
        .lock()
        .unwrap()
        .prepare("SELECT id,markdown FROM wiki_pages WHERE owner=?1 ORDER BY id")
        .unwrap()
        .query_map([s.db.owner()], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
fn tags_for(s: &AppState, memo: &str) -> Vec<(String, String)> {
    s.db.0
        .lock()
        .unwrap()
        .prepare("SELECT tag,origin FROM wiki_tags WHERE memo_id=?1 AND owner=?2 ORDER BY tag")
        .unwrap()
        .query_map(params![memo, s.db.owner()], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
async fn feed(s: &Arc<AppState>, q: &str, tag: &str, offset: usize) -> Value {
    list(
        PrivateState(s.clone()),
        Query(Search {
            q: q.into(),
            offset,
            tag: tag.into(),
        }),
    )
    .await
    .unwrap()
    .0
}
fn item_tags(v: &Value) -> Vec<&str> {
    v["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect()
}
#[tokio::test]
async fn settings_are_partial_validated_wallet_scoped_and_durable() {
    let (dir, s) = setup();
    assert!(assistance::read(&s.db).unwrap().wiki_enabled);
    assistance::save(&s, json!({"wiki_enabled":false})).unwrap();
    assert!(!assistance::read(&s.db).unwrap().wiki_enabled);
    assistance::save(&s, json!({"wiki_enabled":true,"textedit_style":"detailed"})).unwrap();
    assistance::save(&s, json!({"textedit_settle_ms":2600})).unwrap();
    let prefs = assistance::read(&s.db).unwrap();
    assert!(prefs.wiki_enabled);
    assert_eq!(prefs.textedit_style, "detailed");
    for invalid in [
        json!({"textedit_settle_ms":499}),
        json!({"wiki_group_by":"../../etc"}),
        json!({"textedit_style":"shell"}),
        json!({"unknown":true}),
        json!({"wiki_enabled":null}),
    ] {
        assert!(assistance::save(&s, invalid).is_err());
        assert_eq!(assistance::read(&s.db).unwrap(), prefs);
    }
    // Another wallet keeps the defaults, untouched by alice's changes.
    assert_eq!(
        assistance::read(&s.for_owner("bob").db).unwrap(),
        causewaybay_shared::AssistancePreferences::default()
    );
    assert!(
        assistance::read(&s.for_owner("bob").db)
            .unwrap()
            .wiki_enabled
    );
    let reopened =
        AppState::open_with_credentials(dir.path().into(), false, None, Credentials::default())
            .unwrap()
            .for_owner("alice");
    assert_eq!(assistance::read(&reopened.db).unwrap(), prefs);
}
#[tokio::test]
async fn tag_documents_keep_provenance_and_never_mix_wallets() {
    let (dir, s) = setup();
    // The Wiki is on by default; this test starts with it off to watch it switch on.
    assistance::save(&s, json!({"wiki_enabled":false})).unwrap();
    let first = memo(&s, "Bitcoin #bitcoin #money");
    assert!(pages(&s).is_empty());
    assistance::save(&s, json!({"wiki_enabled":true})).unwrap();
    // Switching on judges what was kept while off.
    analyze(&s, &first).await.unwrap();
    memo(&s, "Mining #bitcoin");
    let other = s.for_owner("bob");
    // Bob's own Wiki stays off here, so his page ids cannot collide with alice's.
    assistance::save(&other, json!({"wiki_enabled":false})).unwrap();
    memo(&other, "Bob secret #bitcoin");
    let p = pages(&s);
    assert_eq!(p.len(), 3);
    let bitcoin = p
        .iter()
        .find(|(_, t)| t.starts_with("# bitcoin\n"))
        .unwrap();
    assert!(bitcoin.1.contains(&format!("/memos/{first}")));
    assert!(bitcoin.1.contains("#bitcoin #money"));
    assert!(bitcoin.1.contains("Mining"));
    assert!(!bitcoin.1.contains("Bob secret"));
    assert!(bitcoin.1.contains("AI answer · apple"));
    // Documents live in SQLite only: nothing is mirrored to disk.
    assert!(!dir.path().join("wiki").exists());
    // The tag's document id is what the feed hands the reader.
    assert_eq!(bitcoin.0, document_id("topic", "bitcoin"));
    let err = page(PrivateState(other.clone()), Path(bitcoin.0.clone()))
        .await
        .err()
        .unwrap();
    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert_eq!(feed(&s, "decentralized", "", 0).await["total"], 2);
    assert_eq!(feed(&other, "", "", 0).await["total"], 0);
    assistance::save(&s, json!({"wiki_include_answers":false})).unwrap();
    assert!(pages(&s).iter().all(|(_, t)| !t.contains("### AI answer")));
    assistance::save(&s, json!({"wiki_group_by":"day"})).unwrap();
    assert_eq!(pages(&s).len(), 1);
    assert!(!dir.path().join("wiki").exists());
    // Day grouping changes the documents only; the feed still carries the tags.
    let v = feed(&s, "", "", 0).await;
    assert_eq!(v["tags"].as_array().unwrap().len(), 3);
    assert!(v["tags"][0]["document"].is_null());
    assert_eq!(item_tags(&v["items"][1]), ["q&a", "bitcoin", "money"]);
}
#[tokio::test]
async fn manual_updates_deletions_and_crash_repair_preserve_the_source_boundary() {
    let (dir, s) = setup();
    assistance::save(&s, json!({"wiki_enabled":true,"wiki_auto_update":false})).unwrap();
    let id = memo(&s, "Private source #private");
    assert!(pages(&s).is_empty());
    assert_eq!(
        rebuild(PrivateState(s.clone())).await.unwrap().0["rebuilt"],
        true
    );
    let original = pages(&s);
    assert_eq!(original.len(), 2);
    // The #private document (the other one is the q&a category's).
    let private = original
        .iter()
        .find(|(_, t)| t.starts_with("# private\n"))
        .unwrap();
    assert!(private.1.contains("Private source"));
    // A legacy on-disk mirror left by an older version is not read: the page
    // in SQLite is the document, and a restart leaves the folder alone here
    // (startup migration removes it).
    restore(&s.db).unwrap();
    assert!(pages(&s).iter().any(|(_, t)| t.contains("Private source")));
    assert!(!dir.path().join("wiki").exists());
    let next = memo(&s, "Next source #next");
    assert_eq!(pages(&s).len(), 2);
    s.db.update(
        &id,
        Some("Renamed private source"),
        Some(&["renamed".into()]),
        None,
    )
    .unwrap();
    assert!(pages(&s)[0].1.contains("Renamed private source"));
    assert_eq!(
        tags_for(&s, &id),
        [
            ("q&a".to_string(), "type".to_string()),
            ("renamed".to_string(), "user".to_string())
        ]
    );
    s.db.delete(&id).unwrap();
    assert!(pages(&s).is_empty());
    assert!(tags_for(&s, &id).is_empty());
    assert_eq!(
        rebuild(PrivateState(s.clone())).await.unwrap().0["rebuilt"],
        true
    );
    assert_eq!(pages(&s).len(), 2);
    s.db.delete(&next).unwrap();
    restore(&s.db).unwrap();
    assert!(pages(&s).is_empty());
}
#[tokio::test]
async fn room_erasure_removes_wiki_content_from_sqlite_and_files() {
    let (dir, s) = setup();
    assistance::save(&s, json!({"wiki_enabled":true})).unwrap();
    let room = crate::chat::create(
        PrivateState(s.clone()),
        Json(crate::chat::NewRoom {
            roomid: None,
            title: Some("Wiki erasure".into()),
        }),
    )
    .await
    .unwrap()
    .0
    .roomid;
    let id = memo(&s, "Erase this #room");
    s.db.assign_room("memos", &id, &room).unwrap();
    assert_eq!(pages(&s).len(), 2);
    // The message links back to its room; the document picks the link up on
    // its next build.
    let _ = rebuild(PrivateState(s.clone())).await.unwrap();
    let v = feed(&s, "", "", 0).await;
    assert_eq!(v["items"][0]["room"]["title"], "Wiki erasure");
    assert_eq!(v["items"][0]["room"]["url"], format!("/chat?room={room}"));
    assert!(
        pages(&s)[0]
            .1
            .contains(&format!("in [Wiki erasure](/chat?room={room})"))
    );
    crate::chat::finish_purge(&s.db, &room, false).unwrap();
    assert!(pages(&s).is_empty());
    assert!(tags_for(&s, &id).is_empty());
    assert!(!dir.path().join("wiki").exists());
}

#[tokio::test]
async fn wiki_search_covers_full_sources_and_literal_search_operators() {
    let (_dir, s) = setup();
    assistance::save(&s, json!({"wiki_enabled":true})).unwrap();
    memo(
        &s,
        &format!("{} NeedleBeyondExcerpt #long", "context ".repeat(1000)),
    );
    let result = feed(&s, "NeedleBeyondExcerpt", "", 0).await;
    assert_eq!(result["total"], 1);
    assert!(!pages(&s)[0].1.contains("NeedleBeyondExcerpt"));
    assert_eq!(feed(&s, "%' OR 1=1 --", "", 0).await["total"], 0);
    assert_eq!(feed(&s, "", "%' OR 1=1 --", 0).await["total"], 0);
    assert_eq!(
        s.db.0
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM ai_usage", [], |r| r
                .get::<_, usize>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn tags_are_inferred_on_device_and_messages_link_to_tags_in_the_graph() {
    let (_dir, s) = setup();
    s.db.set_setting(
        "assistance.alice",
        &serde_json::json!({"wiki_enabled":true,"wiki_auto_update":true,"wiki_group_by":"topic","wiki_include_answers":true,"textedit_enabled":true,"textedit_settle_ms":1200,"textedit_style":"concise"}).to_string(),
    )
    .unwrap();
    // No hashtags: the on-device analysis (heuristic in tests) names the tag.
    let first = memo(
        &s,
        "Bitcoin mining difficulty adjusts every two weeks of bitcoin blocks",
    );
    let second = memo(
        &s,
        "Bitcoin halving cuts the bitcoin block reward for miners",
    );
    let third = memo(&s, "Sourdough starter needs flour and water daily feeding");
    for id in [&first, &second, &third] {
        analyze(&s, id).await.unwrap();
    }
    let (topics, keywords) =
        heuristic_topics("Bitcoin mining difficulty adjusts every two weeks of bitcoin blocks");
    assert_eq!(topics, ["bitcoin"]);
    assert!(keywords.contains(&"mining".to_string()));
    assert_eq!(titles(&s), ["bitcoin", "q&a", "sourdough"]);
    assert_eq!(
        tags_for(&s, &first),
        [
            ("bitcoin".to_string(), "heuristic".to_string()),
            ("q&a".to_string(), "type".to_string())
        ]
    );
    // Re-analyzing is a no-op and never calls a model.
    analyze(&s, &first).await.unwrap();
    // Handlers are called directly with the wallet-scoped state, as the router would.
    let graph = graph(PrivateState(s.clone())).await.unwrap().0;
    let nodes = graph["nodes"].as_array().unwrap();
    assert_eq!(nodes.iter().filter(|n| n["kind"] == "tag").count(), 2);
    assert_eq!(nodes.iter().filter(|n| n["kind"] != "tag").count(), 3);
    let bitcoin = nodes.iter().find(|n| n["id"] == "tag:bitcoin").unwrap();
    assert_eq!(bitcoin["size"], 2);
    assert_eq!(bitcoin["document"], document_id("topic", "bitcoin"));
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(edges.iter().filter(|e| e["kind"] == "tag").count(), 3);
    assert!(
        edges
            .iter()
            .any(|e| e["kind"] == "tag" && e["from"] == first && e["to"] == "tag:bitcoin")
    );
    assert!(
        parse_topics(
            "Sure! {\"topics\":[\"Rust Async\"],\"keywords\":[\"tokio\",\"futures\"]} done"
        )
        .is_some()
    );
    assert!(parse_topics("no json here").is_none());
}

#[tokio::test]
async fn feed_ranks_messages_by_bm25_and_filters_by_tags() {
    let (_dir, s) = setup();
    s.db.set_setting(
        "assistance.alice",
        &serde_json::json!({"wiki_enabled":true,"wiki_auto_update":true,"wiki_group_by":"topic","wiki_include_answers":true,"textedit_enabled":true,"textedit_settle_ms":1200,"textedit_style":"concise"}).to_string(),
    )
    .unwrap();
    memo(
        &s,
        "Lightning channels settle bitcoin payments off-chain #payments #bitcoin",
    );
    memo(&s, "Sourdough hydration ratios for a wetter crumb #baking");
    memo(&s, "Bitcoin fees spike when blocks fill up #bitcoin");
    for (query, tag, first, count) in [
        ("lightning", "", "payments", 1),
        ("sourdough crumb", "", "baking", 1),
        ("", "", "bitcoin", 3),
        ("#bitcoin", "", "bitcoin", 2),
        ("", "bitcoin", "bitcoin", 2),
        ("#bitcoin #payments", "", "payments", 1),
        ("lightning #bitcoin", "", "payments", 1),
        ("lightning #baking", "", "", 0),
    ] {
        let v = feed(&s, query, tag, 0).await;
        assert_eq!(v["total"], count, "{query} {tag}: {v}");
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), count, "{query} {tag}: {v}");
        if count > 0 {
            assert!(item_tags(&items[0]).contains(&first), "{query} {tag}: {v}");
        }
    }
    // The rail counts every tag; a filtered view also names the tags that
    // appear together with the active ones.
    let v = feed(&s, "#bitcoin", "", 0).await;
    assert_eq!(v["active_tags"], json!(["bitcoin"]));
    assert_eq!(v["query"], "");
    // Categories count like tags in the rail; q&a leads because every answered
    // question carries it.
    assert_eq!(v["tags"][0]["tag"], "q&a");
    let bitcoin = v["tags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["tag"] == "bitcoin")
        .unwrap();
    assert_eq!(bitcoin["count"], 2);
    assert_eq!(bitcoin["category"], false);
    assert_eq!(v["related_tags"], json!([{"tag":"payments","count":1}]));
    assert_eq!(feed(&s, "", "", 0).await["related_tags"], json!([]));
}

fn enable(s: &Arc<AppState>) {
    assistance::save(s, json!({"wiki_enabled":true})).unwrap();
}
fn titles(s: &AppState) -> Vec<String> {
    s.db.0
        .lock()
        .unwrap()
        .prepare("SELECT title FROM wiki_pages WHERE owner=?1 ORDER BY title")
        .unwrap()
        .query_map([s.db.owner()], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
fn topic_row(s: &AppState, memo: &str) -> Option<(String, String, String, String)> {
    s.db.0
        .lock()
        .unwrap()
        .query_row(
            "SELECT topics,keywords,provider,updated_at FROM wiki_topics WHERE memo_id=?1",
            [memo],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .unwrap()
}

#[test]
fn heuristic_topics_skip_stop_words_short_words_and_rank_by_frequency() {
    let (topics, keywords) =
        heuristic_topics("The the the bitcoin bitcoin mining of a big BIG network");
    assert_eq!(topics, ["bitcoin"]);
    assert_eq!(keywords, ["bitcoin", "mining", "network"]);
    assert_eq!(heuristic_topics("a an it the"), (vec![], vec![]));
    assert_eq!(heuristic_topics(""), (vec![], vec![]));
    // Ties keep first-seen order, so the title's words lead.
    let (topics, keywords) = heuristic_topics("Sourdough starter needs flour");
    assert_eq!(topics, ["sourdough"]);
    assert_eq!(keywords, ["sourdough", "starter", "needs", "flour"]);
    // Words are compared case-insensitively and punctuation never leaks in.
    let (_, keywords) = heuristic_topics("Rust, rust! RUST? tokio-futures");
    assert_eq!(keywords[0], "rust");
    assert!(
        keywords
            .iter()
            .all(|k| k.chars().all(char::is_alphanumeric))
    );
}

#[test]
fn parse_topics_is_lenient_and_bounded() {
    let (topics, keywords) = parse_topics(
        "Sure, here you go:\n{\"topics\":[\" #Rust Async \",\"Tokio\",\"Futures\",\"Fourth\"],\"keywords\":[\"a\",\"b\",\"c\",\"d\",\"e\",\"f\",\"g\"]}\nAnything else?",
    )
    .unwrap();
    assert_eq!(topics, ["rust async", "tokio", "futures"]);
    assert_eq!(keywords, ["a", "b", "c", "d", "e", "f"]);
    // The newer reply shape names tags and allows five.
    let (tags, _) = parse_topics(
        "{\"tags\":[\"#Bitcoin\",\"crypto\",\"mining\",\"energy\",\"policy\",\"sixth\"],\"keywords\":[]}",
    )
    .unwrap();
    assert_eq!(tags, ["bitcoin", "crypto", "mining", "energy", "policy"]);
    let long = "x".repeat(61);
    let (topics, keywords) = parse_topics(&format!(
        "{{\"topics\":[\"{long}\",\"ok\"],\"keywords\":[\"\",\"  \",\"fine\"]}}"
    ))
    .unwrap();
    assert_eq!(topics, ["ok"]);
    assert_eq!(keywords, ["fine"]);
    // A string stands in for a list (small models answer that way); a number is ignored.
    assert_eq!(
        parse_topics("{\"topics\":\"not a list\",\"keywords\":5}"),
        Some((vec!["not a list".to_string()], vec![]))
    );
    assert!(parse_topics("{\"topics\":7,\"keywords\":5}").is_none());
    assert!(parse_topics("{\"topics\":[],\"keywords\":[]}").is_none());
    assert!(parse_topics("no json at all").is_none());
    assert!(parse_topics("{broken").is_none());
}

#[test]
fn tags_normalize_to_one_spelling_and_resolve_in_order() {
    for (raw, expected) in [
        ("#Bitcoin", Some("bitcoin")),
        ("  Rust Async ", Some("rust-async")),
        ("machine learning", Some("machine-learning")),
        ("crypto/Bitcoin", Some("crypto/bitcoin")),
        ("BTC", Some("bitcoin")),
        ("---", None),
        ("#", None),
        ("", None),
        ("v1.2", Some("v1-2")),
        ("한국어 노트", Some("한국어-노트")),
    ] {
        assert_eq!(normalize_tag(raw).as_deref(), expected, "{raw:?}");
    }
    assert_eq!(normalize_tag(&"a".repeat(80)).unwrap().len(), 40);
    // User hashtags lead, model tags follow, five at most, no duplicates.
    let user = ["Bitcoin".to_string(), "money".to_string()];
    let model = [
        "bitcoin".to_string(),
        "crypto".to_string(),
        "mining".to_string(),
        "energy".to_string(),
        "policy".to_string(),
    ];
    let resolved = resolve_tags(&user, &model, "apple");
    assert_eq!(
        resolved,
        [
            ("bitcoin".to_string(), "user"),
            ("money".to_string(), "user"),
            ("crypto".to_string(), "model"),
            ("mining".to_string(), "model"),
            ("energy".to_string(), "model"),
        ]
    );
    // Heuristic (no-model) tags never dilute a hashtagged message.
    assert_eq!(
        resolve_tags(&user, &model, "heuristic"),
        [
            ("bitcoin".to_string(), "user"),
            ("money".to_string(), "user")
        ]
    );
    assert_eq!(
        resolve_tags(&[], &model[..2], "heuristic"),
        [
            ("bitcoin".to_string(), "heuristic"),
            ("crypto".to_string(), "heuristic")
        ]
    );
    // The search box separates words from `#tag` filters.
    assert_eq!(
        parse_query("lightning #Bitcoin fees #bitcoin #"),
        ("lightning fees".to_string(), vec!["bitcoin".to_string()])
    );
    assert_eq!(parse_query("   "), (String::new(), vec![]));
}

#[tokio::test]
async fn analysis_respects_settings_hashtags_grouping_and_wallets() {
    let (_dir, s) = setup();
    // Wiki off: nothing is analyzed or built.
    assistance::save(&s, json!({"wiki_enabled":false})).unwrap();
    let early = memo(&s, "Sourdough starter needs flour");
    analyze(&s, &early).await.unwrap();
    assert!(topic_row(&s, &early).is_none());
    assert!(pages(&s).is_empty());
    enable(&s);
    // A source without hashtags gets a tag named by the on-device analysis
    // (the heuristic stands in for the model in tests), never "inbox".
    analyze(&s, &early).await.unwrap();
    let (topics, keywords, provider, stamp) = topic_row(&s, &early).unwrap();
    assert_eq!(topics, "[\"sourdough\"]");
    assert!(keywords.contains("flour"));
    assert_eq!(provider, "heuristic");
    assert_eq!(titles(&s), ["q&a", "sourdough"]);
    // Repeating the analysis is a no-op: same row, same timestamp.
    analyze(&s, &early).await.unwrap();
    assert_eq!(topic_row(&s, &early).unwrap().3, stamp);
    // Hashtags the user wrote win over the inferred tag.
    let tagged = memo(&s, "Sourdough timing for weekends #baking");
    analyze(&s, &tagged).await.unwrap();
    assert_eq!(titles(&s), ["baking", "q&a", "sourdough"]);
    assert_eq!(
        tags_for(&s, &tagged),
        [
            ("baking".to_string(), "user".to_string()),
            ("q&a".to_string(), "type".to_string())
        ]
    );
    // A kept note with nothing to tag still has its category, so it never
    // vanishes from the Wiki.
    let bare =
        s.db.create("ok so be it", "memo", "apple", "system")
            .unwrap()
            .id;
    s.db.finish(&bare, "memo").unwrap();
    analyze(&s, &bare).await.unwrap();
    assert_eq!(
        tags_for(&s, &bare),
        [
            ("information".to_string(), "category".to_string()),
            ("note".to_string(), "type".to_string())
        ]
    );
    assert_eq!(
        titles(&s),
        ["baking", "information", "note", "q&a", "sourdough"]
    );
    s.db.delete(&bare).unwrap();
    // A source that is still running is not analyzed.
    let running =
        s.db.create("Half-written capture", "question", "apple", "system")
            .unwrap();
    analyze(&s, &running.id).await.unwrap();
    assert!(topic_row(&s, &running.id).is_none());
    // Day grouping changes the documents only.
    assistance::save(&s, json!({"wiki_group_by":"day"})).unwrap();
    assert_eq!(titles(&s).len(), 1);
    assert_eq!(titles(&s)[0].len(), 10);
    assistance::save(&s, json!({"wiki_group_by":"topic"})).unwrap();
    // Another wallet's analysis never changes this wallet's pages or tags.
    let bob = s.for_owner("bob");
    enable(&bob);
    let secret = memo(&bob, "Sourdough secrets of bob");
    analyze(&bob, &secret).await.unwrap();
    assert_eq!(titles(&bob), ["q&a", "sourdough"]);
    assert_eq!(titles(&s), ["baking", "q&a", "sourdough"]);
    assert_eq!(feed(&s, "", "sourdough", 0).await["total"], 1);
    let (alice_page, markdown) = pages(&s)
        .into_iter()
        .find(|(_, t)| t.starts_with("# sourdough\n"))
        .unwrap();
    assert!(!markdown.contains("secrets of bob"));
    assert!(alice_page.len() == 64);
}

#[tokio::test]
async fn graph_links_tags_by_shared_messages_and_related_follows_deletion() {
    let (dir, s) = setup();
    enable(&s);
    let bread = memo(&s, "Sourdough starter flour water #bread #baking");
    let pizza = memo(&s, "Pizza dough flour water yeast #pizza #baking");
    let poem = memo(&s, "Quiet evening verses about rain #poetry");
    for id in [&bread, &pizza, &poem] {
        analyze(&s, id).await.unwrap();
    }
    let first = graph(PrivateState(s.clone())).await.unwrap().0;
    let nodes = first["nodes"].as_array().unwrap();
    let edges = first["edges"].as_array().unwrap();
    assert_eq!(nodes.iter().filter(|n| n["kind"] == "tag").count(), 4);
    assert_eq!(nodes.iter().filter(|n| n["kind"] != "tag").count(), 3);
    assert!(
        nodes
            .iter()
            .filter(|n| n["kind"] != "tag")
            .all(|n| n["url"].as_str().unwrap().starts_with("/memos/"))
    );
    // Every message hangs off each of its tags.
    assert_eq!(edges.iter().filter(|e| e["kind"] == "tag").count(), 5);
    // Baking links to bread and pizza (one shared message each); poetry to nothing.
    let related_edges: Vec<&Value> = edges.iter().filter(|e| e["kind"] == "related").collect();
    assert_eq!(related_edges.len(), 2);
    assert!(
        related_edges
            .iter()
            .all(|e| e["from"] == "tag:baking" && e["weight"] == 1)
    );
    assert!(!edges.iter().any(|e| e["from"] == "tag:poetry" || e["to"] == "tag:poetry" && e["kind"] == "related"));
    // Related messages: bread and pizza share a tag; the poem relates to neither.
    let v = related(PrivateState(s.clone()), Path(bread.clone()))
        .await
        .unwrap()
        .0;
    assert_eq!(v["item"]["id"], bread);
    assert_eq!(v["related"].as_array().unwrap().len(), 1);
    assert_eq!(v["related"][0]["id"], pizza);
    assert_eq!(v["semantic"], false);
    let v = related(PrivateState(s.clone()), Path(poem.clone()))
        .await
        .unwrap()
        .0;
    assert!(v["related"].as_array().unwrap().is_empty());
    assert_eq!(
        related(PrivateState(s.for_owner("bob")), Path(bread.clone()))
            .await
            .err()
            .unwrap()
            .0,
        StatusCode::NOT_FOUND
    );
    // Deleting a message removes its node, its rows, and an emptied tag.
    s.db.delete(&pizza).unwrap();
    assert!(topic_row(&s, &pizza).is_none());
    let after = graph(PrivateState(s.clone())).await.unwrap().0;
    let nodes = after["nodes"].as_array().unwrap();
    assert_eq!(nodes.iter().filter(|n| n["kind"] == "tag").count(), 3);
    assert!(
        !nodes
            .iter()
            .any(|n| n["id"] == pizza || n["id"] == "tag:pizza")
    );
    assert_eq!(
        after["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "related")
            .count(),
        1
    );
    assert!(!dir.path().join("wiki").exists());
    // Another wallet sees an empty graph.
    let other = graph(PrivateState(s.for_owner("bob"))).await.unwrap().0;
    assert!(other["nodes"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn feed_is_newest_first_paginates_and_rejects_oversized_queries() {
    let (_dir, s) = setup();
    enable(&s);
    memo(
        &s,
        "Lightning channels settle bitcoin payments off-chain #payments",
    );
    memo(&s, "Sourdough hydration ratios for a wetter crumb #baking");
    memo(&s, "Quiet evening verses about rain #poetry");
    // No query: newest first, everything, embeddings not loaded in tests.
    let all = feed(&s, "", "", 0).await;
    assert_eq!(all["total"], 3);
    assert_eq!(all["semantic"], false);
    assert_eq!(all["page_size"], FEED_PAGE);
    let order: Vec<&str> = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["tags"][1].as_str().unwrap())
        .collect();
    assert_eq!(order, ["poetry", "baking", "payments"]);
    // A word from a message's text finds that message only.
    let hit = feed(&s, "channels", "", 0).await;
    assert_eq!(hit["total"], 1);
    assert_eq!(item_tags(&hit["items"][0]), ["q&a", "payments"]);
    // Prefix matching through the full-text index.
    let hit = feed(&s, "hydrat", "", 0).await;
    assert_eq!(item_tags(&hit["items"][0]), ["q&a", "baking"]);
    // A tag word matches as text too.
    let hit = feed(&s, "poetry", "", 0).await;
    assert_eq!(hit["total"], 1);
    // Offsets apply after ordering; past the end is empty but the total stays.
    let tail = feed(&s, "", "", 2).await;
    assert_eq!(tail["total"], 3);
    assert_eq!(tail["items"].as_array().unwrap().len(), 1);
    assert_eq!(item_tags(&tail["items"][0]), ["q&a", "payments"]);
    let beyond = feed(&s, "", "", 10).await;
    assert!(beyond["items"].as_array().unwrap().is_empty());
    // Nothing matches: no items, total zero, the rail still lists every tag.
    let none = feed(&s, "zzzz-nothing", "", 0).await;
    assert_eq!(none["total"], 0);
    assert_eq!(none["tags"].as_array().unwrap().len(), 4);
    // Oversized queries are rejected.
    assert!(
        list(
            PrivateState(s.clone()),
            Query(Search {
                q: "x".repeat(1001),
                offset: 0,
                tag: String::new(),
            })
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn manual_mode_keeps_pages_when_new_sources_are_analyzed() {
    let (_dir, s) = setup();
    assistance::save(&s, json!({"wiki_enabled":true,"wiki_auto_update":false})).unwrap();
    let first = memo(&s, "A first thought #ideas");
    analyze(&s, &first).await.unwrap();
    // Analysis alone never creates the space; the first build is explicit.
    assert!(pages(&s).is_empty());
    assert!(tags_for(&s, &first).is_empty());
    let _ = rebuild(PrivateState(s.clone())).await.unwrap();
    assert_eq!(titles(&s), ["ideas", "q&a"]);
    let second = memo(&s, "A second thought #new");
    analyze(&s, &second).await.unwrap();
    // With automatic updates off, the new source waits for the next build.
    assert_eq!(titles(&s), ["ideas", "q&a"]);
    assert_eq!(feed(&s, "", "", 0).await["total"], 1);
    let _ = rebuild(PrivateState(s.clone())).await.unwrap();
    assert_eq!(titles(&s), ["ideas", "new", "q&a"]);
    assert_eq!(feed(&s, "", "", 0).await["total"], 2);
}

#[tokio::test]
async fn messages_record_cloud_tokens_cost_and_show_media() {
    let (dir, s) = setup();
    enable(&s);
    let id = memo(&s, "Compare rollups and sidechains #bitcoin");
    // A cloud call and a local call were measured for this memo; only the cloud
    // one counts toward tokens and cost. The memo also has a screenshot and an
    // uploaded picture and video.
    {
        let conn = s.db.0.lock().unwrap();
        conn.execute("INSERT INTO ai_usage(id,memo_id,owner_wallet,provider,model,created_at,status,elapsed_ms,input_tokens,output_tokens) VALUES('u1',?1,'alice','grok','grok-4','2026-10-07T00:00:00Z','complete',900,1000,200)", [&id]).unwrap();
        conn.execute("INSERT INTO ai_usage(id,memo_id,owner_wallet,provider,model,created_at,status,elapsed_ms,input_tokens,output_tokens) VALUES('u2',?1,'alice','apple','system','2026-10-07T00:00:01Z','complete',300,NULL,NULL)", [&id]).unwrap();
        conn.execute(
            "UPDATE memos SET image_path='screenshots/x.png' WHERE id=?1",
            [&id],
        )
        .unwrap();
        conn.execute("INSERT INTO files(id,name,mime,bytes,created_at,owner_wallet,origin_memo) VALUES('f-img','diagram.png','image/png',x'00','2026-10-07T00:00:02Z','alice',?1)", [&id]).unwrap();
        conn.execute("INSERT INTO files(id,name,mime,bytes,created_at,owner_wallet,origin_memo) VALUES('f-vid','talk.mp4','video/mp4',x'00','2026-10-07T00:00:03Z','alice',?1)", [&id]).unwrap();
    }
    let _ = rebuild(PrivateState(s.clone())).await.unwrap();
    let (page_id, markdown) = pages(&s).into_iter().next().unwrap();
    assert!(
        markdown.contains(
            "Cloud AI used for this page: 1 call · tokens in/out 1000 / 200 · est. $0.0060"
        ),
        "{markdown}"
    );
    assert!(markdown.contains(
        "Cloud AI usage: grok · grok-4 · 1 call · tokens in/out 1000 / 200 · est. $0.0060"
    ));
    assert!(markdown.contains(&format!("![Captured screen](/api/memos/{id}/image)")));
    assert!(markdown.contains("![diagram.png](/api/files/f-img/image)"));
    assert!(markdown.contains(
        "[![talk.mp4 (video frame)](/api/files/f-vid/thumbnail)](/api/files/f-vid/stream)"
    ));
    assert!(!dir.path().join("wiki").exists());
    let v = page(PrivateState(s.clone()), Path(page_id))
        .await
        .unwrap()
        .0;
    let source = &v["sources"][0];
    assert_eq!(source["cloud_calls"], 1);
    assert_eq!(source["input_tokens"], 1000);
    assert_eq!(source["output_tokens"], 200);
    assert!((source["estimated_usd"].as_f64().unwrap() - 0.006).abs() < 1e-9);
    assert_eq!(source["has_image"], true);
    assert_eq!(source["attachments"].as_array().unwrap().len(), 2);
    assert_eq!(source["attachments"][1]["kind"], "video");
    // The feed item is a photo node with the capture as its thumbnail and the
    // same cost figures.
    let v = feed(&s, "", "bitcoin", 0).await;
    let item = &v["items"][0];
    assert_eq!(item["kind"], "photo");
    assert_eq!(item["thumbnail"], format!("/api/memos/{id}/thumbnail"));
    assert_eq!(item["cloud_calls"], 1);
    assert_eq!(item["cloud_model"], "grok · grok-4");
    assert!((item["estimated_usd"].as_f64().unwrap() - 0.006).abs() < 1e-9);
    assert_eq!(item["has_answer"], true);
    assert_eq!(item["attachments"].as_array().unwrap().len(), 2);
    let g = graph(PrivateState(s.clone())).await.unwrap().0;
    let node = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == id)
        .unwrap();
    assert_eq!(node["kind"], "photo");
    assert_eq!(node["thumbnail"], format!("/api/memos/{id}/thumbnail"));
    // A local-only message records no cloud usage; answered, it is a q&a node.
    let local = memo(&s, "Just a local thought #local");
    let v = feed(&s, "", "local", 0).await;
    assert_eq!(v["items"][0]["id"], local);
    assert_eq!(v["items"][0]["kind"], "q&a");
    assert_eq!(v["items"][0]["cloud_calls"], 0);
    assert!(v["items"][0]["thumbnail"].is_null());
    assert!(v["items"][0]["estimated_usd"].is_null());
}

fn message_doc(s: &AppState, messageid: &str) -> ChatMessage {
    let doc: String =
        s.db.0
            .lock()
            .unwrap()
            .query_row(
                "SELECT document FROM chat_messages WHERE messageid=?1",
                [messageid],
                |r| r.get(0),
            )
            .unwrap();
    serde_json::from_str(&doc).unwrap()
}
#[tokio::test]
async fn curated_tags_replace_inferred_ones_and_reach_the_chat_message() {
    let (_dir, s) = setup();
    enable(&s);
    let room = crate::chat::create(
        PrivateState(s.clone()),
        Json(crate::chat::NewRoom {
            roomid: None,
            title: Some("Tags".into()),
        }),
    )
    .await
    .unwrap()
    .0
    .roomid;
    let text = "Bitcoin mining difficulty adjusts every two weeks of bitcoin blocks";
    let id = memo(&s, text);
    s.db.assign_room("memos", &id, &room).unwrap();
    let mut message = ChatMessage {
        roomid: room.clone(),
        messageid: db::id(),
        role: "user".into(),
        text: text.into(),
        memo_id: id.clone(),
        provider: "apple".into(),
        status: "complete".into(),
        created_at: db::now(),
        ..Default::default()
    };
    {
        let mut conn = s.db.0.lock().unwrap();
        let tx = conn.transaction().unwrap();
        crate::chat::write(&tx, &mut message).unwrap();
        tx.commit().unwrap();
    }
    let mut reply = ChatMessage {
        roomid: room.clone(),
        messageid: db::id(),
        role: "assistant".into(),
        text: "A generated explanation of decentralized money.".into(),
        memo_id: id.clone(),
        provider: "apple".into(),
        status: "complete".into(),
        created_at: db::now(),
        ..Default::default()
    };
    {
        let mut conn = s.db.0.lock().unwrap();
        let tx = conn.transaction().unwrap();
        crate::chat::write(&tx, &mut reply).unwrap();
        tx.commit().unwrap();
    }
    let first_sync = message.messagesyncid;
    // The messages were written after the memo was judged; the next sync
    // carries the tags onto them.
    let _ = rebuild(PrivateState(s.clone())).await.unwrap();
    // The on-device analysis tags the memo, and both the post and its generated
    // response learn the tags as newer revisions, so every device picks them
    // up through room sync.
    analyze(&s, &id).await.unwrap();
    assert_eq!(
        tags_for(&s, &id),
        [
            ("bitcoin".to_string(), "heuristic".to_string()),
            ("q&a".to_string(), "type".to_string())
        ]
    );
    let doc = message_doc(&s, &message.messageid);
    assert_eq!(doc.tags, ["q&a", "bitcoin"]);
    assert!(doc.messagesyncid > first_sync);
    assert_eq!(message_doc(&s, &reply.messageid).tags, ["q&a", "bitcoin"]);
    // A sync that changes nothing writes no revision.
    let _ = rebuild(PrivateState(s.clone())).await.unwrap();
    assert_eq!(
        message_doc(&s, &message.messageid).messagesyncid,
        doc.messagesyncid
    );
    // Editing the tags replaces inferred ones; spelling is normalized and
    // duplicates and blanks are dropped.
    let tags = set_tags(
        &s,
        &id,
        &[
            "Energy".into(),
            "#bitcoin".into(),
            "energy".into(),
            " ".into(),
        ],
    )
    .unwrap();
    assert_eq!(tags, ["q&a", "bitcoin", "energy"]);
    assert_eq!(
        tags_for(&s, &id),
        [
            ("bitcoin".to_string(), "user".to_string()),
            ("energy".to_string(), "user".to_string()),
            ("q&a".to_string(), "type".to_string())
        ]
    );
    assert_eq!(titles(&s), ["bitcoin", "energy", "q&a"]);
    assert_eq!(
        message_doc(&s, &message.messageid).tags,
        ["q&a", "bitcoin", "energy"]
    );
    assert_eq!(
        message_doc(&s, &reply.messageid).tags,
        ["q&a", "bitcoin", "energy"]
    );
    assert_eq!(
        item_tags(&feed(&s, "", "energy", 0).await["items"][0]),
        ["q&a", "bitcoin", "energy"]
    );
    // A removed tag stays removed: the analysis, a late model result and a
    // rebuild never bring it back.
    assert_eq!(
        set_tags(&s, &id, &["energy".into()]).unwrap(),
        ["q&a", "energy"]
    );
    analyze(&s, &id).await.unwrap();
    {
        let mut conn = s.db.0.lock().unwrap();
        let tx = conn.transaction().unwrap();
        save_topics(
            &tx,
            "alice",
            &id,
            &["bitcoin".into()],
            &["mining".into()],
            "apple",
            "",
            true,
        )
        .unwrap();
        sync_locked(&tx, "alice", false).unwrap();
        tx.commit().unwrap();
    }
    let _ = rebuild(PrivateState(s.clone())).await.unwrap();
    assert_eq!(topic_row(&s, &id).unwrap().2, "user");
    assert_eq!(
        tags_for(&s, &id),
        [
            ("energy".to_string(), "user".to_string()),
            ("q&a".to_string(), "type".to_string())
        ]
    );
    assert_eq!(titles(&s), ["energy", "q&a"]);
    assert_eq!(message_doc(&s, &message.messageid).tags, ["q&a", "energy"]);
    // Editing the text keeps the curated tags and adds the hashtags written.
    let current = message_doc(&s, &message.messageid);
    let _ = crate::chat::edit_message(
        PrivateState(s.clone()),
        Path((room.clone(), message.messageid.clone())),
        Json(crate::chat::Edit {
            text: "Bitcoin mining uses a lot of power #Grid".into(),
            messagesyncid: current.messagesyncid,
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        message_doc(&s, &message.messageid).tags,
        ["q&a", "energy", "grid"]
    );
    assert_eq!(
        tags_for(&s, &id),
        [
            ("energy".to_string(), "user".to_string()),
            ("grid".to_string(), "user".to_string()),
            ("q&a".to_string(), "type".to_string())
        ]
    );
    // Without curation, a text edit replaces the hashtags as before.
    let plain = memo(&s, "Sourdough timing #baking");
    s.db.assign_room("memos", &plain, &room).unwrap();
    let mut other = ChatMessage {
        roomid: room.clone(),
        messageid: db::id(),
        role: "user".into(),
        text: "Sourdough timing #baking".into(),
        memo_id: plain.clone(),
        provider: "apple".into(),
        status: "complete".into(),
        created_at: db::now(),
        ..Default::default()
    };
    {
        let mut conn = s.db.0.lock().unwrap();
        let tx = conn.transaction().unwrap();
        crate::chat::write(&tx, &mut other).unwrap();
        tx.commit().unwrap();
    }
    // Written after the memo was judged: a sync carries the tags onto it.
    let _ = rebuild(PrivateState(s.clone())).await.unwrap();
    let current = message_doc(&s, &other.messageid);
    assert_eq!(current.tags, ["q&a", "baking"]);
    let _ = crate::chat::edit_message(
        PrivateState(s.clone()),
        Path((room.clone(), other.messageid.clone())),
        Json(crate::chat::Edit {
            text: "Sourdough timing #weekend".into(),
            messagesyncid: current.messagesyncid,
        }),
    )
    .await
    .unwrap();
    assert_eq!(message_doc(&s, &other.messageid).tags, ["q&a", "weekend"]);
    // An emptied list keeps the category; too many tags are refused; another
    // wallet cannot edit this message's tags.
    assert_eq!(set_tags(&s, &id, &[]).unwrap(), ["q&a"]);
    let many: Vec<String> = (0..13).map(|i| format!("tag{i}")).collect();
    assert!(
        set_tags(&s, &id, &many)
            .unwrap_err()
            .to_string()
            .contains("12 tags")
    );
    assert!(set_tags(&s.for_owner("bob"), &id, &["x".into()]).is_err());
    assert_eq!(tags_for(&s, &id), [("q&a".to_string(), "type".to_string())]);
    // Twelve user tags are kept in full; inferred ones never pad a curated list.
    let twelve: Vec<String> = (0..12).map(|i| format!("tag{i}")).collect();
    assert_eq!(set_tags(&s, &id, &twelve).unwrap().len(), 13);
    assert_eq!(resolve_tags(&twelve, &["extra".into()], "user").len(), 12);
    assert_eq!(
        resolve_tags(
            &["one".into()],
            &["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
            "apple"
        )
        .len(),
        5
    );
}

#[tokio::test]
async fn tags_set_while_the_answer_is_running_apply_when_it_completes() {
    let (_dir, s) = setup();
    enable(&s);
    let room = crate::chat::create(
        PrivateState(s.clone()),
        Json(crate::chat::NewRoom {
            roomid: None,
            title: Some("Early tags".into()),
        }),
    )
    .await
    .unwrap()
    .0
    .roomid;
    for manual in [false, true] {
        assistance::save(&s, json!({"wiki_auto_update":!manual})).unwrap();
        let m =
            s.db.create("Bitcoin fees tonight", "question", "apple", "system")
                .unwrap();
        s.db.assign_room("memos", &m.id, &room).unwrap();
        let mut message = ChatMessage {
            roomid: room.clone(),
            messageid: db::id(),
            role: "user".into(),
            text: "Bitcoin fees tonight".into(),
            memo_id: m.id.clone(),
            provider: "apple".into(),
            status: "running".into(),
            created_at: db::now(),
            ..Default::default()
        };
        {
            let mut conn = s.db.0.lock().unwrap();
            let tx = conn.transaction().unwrap();
            crate::chat::write(&tx, &mut message).unwrap();
            tx.commit().unwrap();
        }
        // The user tags the message before the answer exists: the list is kept,
        // but a running memo is not a Wiki source yet.
        assert!(set_tags(&s, &m.id, &["fees".into()]).unwrap().is_empty());
        assert!(tags_for(&s, &m.id).is_empty());
        s.db.event(&m.id, "delta", "Fees are low tonight.").unwrap();
        s.db.finish(&m.id, "question").unwrap();
        analyze(&s, &m.id).await.unwrap();
        // Complete: the curated tag applies, nothing is inferred over it, and the
        // chat message carries it; in manual mode the analysis brings the
        // curated message in without a rebuild.
        assert_eq!(
            tags_for(&s, &m.id),
            [
                ("fees".to_string(), "user".to_string()),
                ("q&a".to_string(), "type".to_string())
            ],
            "manual={manual}"
        );
        assert_eq!(message_doc(&s, &message.messageid).tags, ["q&a", "fees"]);
        assert_eq!(topic_row(&s, &m.id).unwrap().2, "user");
    }
}

#[test]
fn parse_topics_accepts_small_model_strings_and_falls_back_to_keywords() {
    // Arrays, as asked for.
    assert_eq!(
        parse_topics(
            "{\"tags\": [\"zig\", \"low-level\", \"compilers\"], \"keywords\": [\"zig\", \"c\"]}"
        ),
        Some((
            vec!["zig".into(), "low-level".into(), "compilers".into()],
            vec!["zig".into(), "c".into()]
        ))
    );
    // One hyphen-glued string for tags and a comma list for keywords, as the
    // Apple model answered the earlier prompt: the blob is dropped and the
    // keywords name the tags.
    assert_eq!(
        parse_topics(
            "{\"tags\":\"simple-explicit-interoperability-low-level-tooling-compilers\",\"keywords\":\"zig, c, tooling, compilers\"}"
        ),
        Some((
            vec!["zig".into(), "c".into(), "tooling".into()],
            vec![
                "zig".into(),
                "c".into(),
                "tooling".into(),
                "compilers".into()
            ]
        ))
    );
    // Alternate keys and comma-separated tags.
    assert_eq!(
        parse_topics("{\"labels\":\"Sourdough, Weekend Baking\",\"terms\":[\"starter\"]}")
            .unwrap()
            .0,
        vec!["sourdough".to_string(), "weekend baking".to_string()]
    );
}

#[tokio::test]
async fn messages_have_no_tags_until_analyzed_and_only_then_get_a_category() {
    let (_dir, s) = setup();
    enable(&s);
    // Complete and synced, but not yet judged: not in the Wiki at all.
    let bare =
        s.db.create("ok so be it", "memo", "apple", "system")
            .unwrap()
            .id;
    s.db.finish(&bare, "memo").unwrap();
    assert!(tags_for(&s, &bare).is_empty());
    assert!(titles(&s).is_empty());
    assert_eq!(feed(&s, "", "", 0).await["total"], 0);
    analyze(&s, &bare).await.unwrap();
    assert_eq!(
        tags_for(&s, &bare),
        [
            ("information".to_string(), "category".to_string()),
            ("note".to_string(), "type".to_string())
        ]
    );
}

#[test]
fn tag_model_follows_the_account_ai_mode() {
    use causewaybay_shared::UserAiPreferences;
    let prefs = |mode: &str| UserAiPreferences {
        mode: mode.into(),
        local_provider: "apple".into(),
        cloud_provider: "grok".into(),
    };
    let all = |_: &str| true;
    assert_eq!(
        tag_providers(Some(&prefs("ondevice")), false, all),
        ["apple"]
    );
    assert_eq!(tag_providers(Some(&prefs("hybrid")), false, all), ["apple"]);
    assert_eq!(
        tag_providers(Some(&prefs("cloudai")), false, all),
        ["grok", "apple"]
    );
    assert_eq!(tag_providers(Some(&prefs("codex")), false, all), ["apple"]);
    // A cloud provider without a key is skipped; so is a missing local model.
    assert_eq!(
        tag_providers(Some(&prefs("hybrid")), false, |id| id == "mlx"),
        ["mlx"]
    );
    assert!(tag_providers(Some(&prefs("hybrid")), false, |_| false).is_empty());
    // Demo mode never calls a model; no saved policy means local when present.
    assert!(tag_providers(Some(&prefs("cloudai")), true, all).is_empty());
    assert_eq!(tag_providers(None, false, all), ["apple"]);
}

#[tokio::test]
async fn repair_retags_messages_a_model_left_untagged_and_keeps_curated_ones() {
    let (_dir, s) = setup();
    enable(&s);
    let broken = memo(
        &s,
        "Bitcoin mining difficulty adjusts every two weeks of bitcoin blocks",
    );
    let curated = memo(&s, "Nothing here");
    {
        let mut conn = s.db.0.lock().unwrap();
        let tx = conn.transaction().unwrap();
        // What the old prompt produced: a model row with no tags, so inbox.
        save_topics(
            &tx,
            "alice",
            &broken,
            &[],
            &["mining".into()],
            "apple",
            "",
            true,
        )
        .unwrap();
        sync_locked(&tx, "alice", false).unwrap();
        tx.commit().unwrap();
    }
    set_tags(&s, &curated, &[]).unwrap();
    // Both carry only their category: the broken one because the model gave
    // no tags, the curated one because the user emptied its list.
    assert_eq!(
        tags_for(&s, &broken),
        [("q&a".to_string(), "type".to_string())]
    );
    assert_eq!(
        tags_for(&s, &curated),
        [("q&a".to_string(), "type".to_string())]
    );
    repair_later(&s);
    for _ in 0..50 {
        if tags_for(&s, &broken).len() > 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // The broken row was re-analyzed (heuristic in tests); the curated one stays.
    assert_eq!(
        tags_for(&s, &broken),
        [
            ("bitcoin".to_string(), "heuristic".to_string()),
            ("q&a".to_string(), "type".to_string())
        ]
    );
    assert_eq!(
        tags_for(&s, &curated),
        [("q&a".to_string(), "type".to_string())]
    );
    assert_eq!(topic_row(&s, &curated).unwrap().2, "user");
}

#[test]
fn categories_come_from_the_rule_the_model_or_the_heuristic() {
    // The category describes the subject, never whether it was answered.
    assert_eq!(
        heuristic_category("question", "Zig is a language.", "I wonder what zig is"),
        "personal"
    );
    assert_eq!(
        heuristic_category("question", "", "what is zig"),
        "information"
    );
    // Records: diary lines and to-dos in English and Korean, facts otherwise.
    assert_eq!(
        heuristic_category(
            "memo",
            "",
            "Had dinner with mom tonight, I should visit more."
        ),
        "personal"
    );
    assert_eq!(
        heuristic_category("memo", "", "오늘 아침에 한강에서 달리기 했다."),
        "personal"
    );
    assert_eq!(
        heuristic_category(
            "memo",
            "",
            "Renew the passport before March, need to book the dentist."
        ),
        "task"
    );
    assert_eq!(
        heuristic_category("memo", "", "여권 갱신 해야 함, 치과 예약"),
        "task"
    );
    assert_eq!(
        heuristic_category(
            "memo",
            "",
            "Zig is a general-purpose language created by Andrew Kelley."
        ),
        "information"
    );
    // The model's category is accepted under a few spellings, nothing else.
    assert_eq!(
        parse_category("{\"category\":\"Personal\",\"tags\":[]}").as_deref(),
        Some("personal")
    );
    assert_eq!(
        parse_category("{\"category\":\"todo\"}").as_deref(),
        Some("task")
    );
    assert_eq!(parse_category("{\"category\":\"qa\"}"), None);
    assert_eq!(parse_category("{\"category\":\"banana\"}"), None);
    assert_eq!(parse_category("{\"tags\":[\"x\"]}"), None);
    assert_eq!(normalize_tag("Q&A").as_deref(), Some("q&a"));
}

#[test]
fn a_typed_line_is_a_question_a_note_or_unsure() {
    for q in [
        "what is zig lang?",
        "Explain ownership in Rust",
        "번역해줘: good morning",
        "왜 하늘은 파란색이지",
        "Can you summarize this",
        "please list three options",
        "send 0xB32d16a925887f8636F76D84A7448F29b7c00aC2 0.1 tcro cronos testnet",
        "Transfer 2 USDC to 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "get balance",
        "balance tcro cronos testnet",
        "Balances on cronos mainnet",
        "내 잔액",
    ] {
        assert_eq!(intent(q), Intent::Question, "{q}");
    }
    for n in [
        "Had dinner with mom tonight, I should visit more often.",
        "오늘 아침에 한강에서 달리기 했다. 날씨가 좋아서 기분이 좋았음.",
        "Renew the passport before March and book the dentist appointment.",
        "https://ziglang.org/documentation/master/ is the reference",
        "Zig is a general-purpose programming language created by Andrew Kelley with a focus on simplicity and C interoperability.",
    ] {
        assert_eq!(intent(n), Intent::Note, "{n}");
    }
    for u in ["zig", "sourdough timing", "그 영화"] {
        assert_eq!(intent(u), Intent::Unsure, "{u}");
    }
    assert!(gate_says_note("NOTE"));
    assert!(gate_says_note("note."));
    assert!(!gate_says_note("QUESTION"));
    assert!(!gate_says_note("This is a QUESTION, not a note"));
}

#[tokio::test]
async fn questions_are_qa_and_curated_tags_keep_their_category() {
    let (_dir, s) = setup();
    enable(&s);
    // memo() finishes a question with an answer and judges it: q&a leads its
    // tags, the topic follows, and a second analysis changes nothing.
    let asked = memo(
        &s,
        "Bitcoin mining difficulty adjusts every two weeks of bitcoin blocks",
    );
    analyze(&s, &asked).await.unwrap();
    assert_eq!(
        tags_of(&s.db.0.lock().unwrap(), "alice", &asked).unwrap(),
        ["q&a", "bitcoin"]
    );
    // A kept diary line has no category until analyzed, then personal.
    let note =
        s.db.create(
            "Had dinner with mom tonight, I should visit more often.",
            "memo",
            "local",
            "",
        )
        .unwrap()
        .id;
    s.db.finish(&note, "memo").unwrap();
    assert!(tags_for(&s, &note).is_empty());
    analyze(&s, &note).await.unwrap();
    assert_eq!(
        tags_of(&s.db.0.lock().unwrap(), "alice", &note).unwrap()[..2],
        ["note", "personal"]
    );
    // Editing tags never removes the category, and typing it adds nothing twice.
    assert_eq!(
        set_tags(&s, &asked, &["energy".into(), "q&a".into()]).unwrap(),
        ["q&a", "energy"]
    );
    assert_eq!(set_tags(&s, &asked, &[]).unwrap(), ["q&a"]);
    // The feed carries the category and the answer with the question.
    let v = feed(&s, "", "q&a", 0).await;
    assert_eq!(v["items"][0]["type"], "q&a");
    assert_eq!(v["items"][0]["category"], "");
    assert!(
        v["items"][0]["answer_snippet"]
            .as_str()
            .unwrap()
            .contains("decentralized money")
    );
    assert!(
        v["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["tag"] == "q&a" && t["type"] == true)
    );
    assert!(
        v["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["tag"] == "personal" && t["category"] == true)
    );
    assert!(
        v["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["category"] == false)
    );
}

fn wiki_notifications(s: &AppState) -> i64 {
    s.db.0
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM notifications WHERE kind='wiki.updated' AND owner_wallet=?1",
            [s.db.owner()],
            |r| r.get(0),
        )
        .unwrap()
}
#[tokio::test]
async fn wiki_changes_notify_connected_clients_once_and_quiet_syncs_do_not() {
    let (_dir, s) = setup();
    enable(&s);
    let before = wiki_notifications(&s);
    let id = memo(&s, "Bitcoin basics #bitcoin");
    // Finishing the memo synced the Wiki (tags and a page): one notification.
    let after_memo = wiki_notifications(&s);
    assert!(after_memo > before);
    // A rebuild that changes nothing says nothing.
    let _ = rebuild(PrivateState(s.clone())).await.unwrap();
    assert_eq!(wiki_notifications(&s), after_memo);
    // A repeated analysis changes nothing and says nothing; editing tags does.
    analyze(&s, &id).await.unwrap();
    let after_analysis = wiki_notifications(&s);
    assert_eq!(after_analysis, after_memo);
    set_tags(&s, &id, &["energy".into()]).unwrap();
    assert!(wiki_notifications(&s) > after_analysis);
    // The row is a wallet notification the SSE stream delivers.
    let events = crate::notifications::changes(&s.db, None, 0).unwrap();
    assert!(events.iter().any(|e| e["kind"] == "wiki.updated"));
    // Another wallet hears nothing about it.
    assert_eq!(wiki_notifications(&s.for_owner("bob")), 0);
}

#[tokio::test]
async fn code_and_video_messages_get_content_tags_and_their_own_node_kind() {
    let (_dir, s) = setup();
    enable(&s);
    let code = memo(
        &s,
        "How do I add in Rust?\n```rust\nfn add(a: i32, b: i32) -> i32 { a + b }\n```",
    );
    let plain = memo(&s, "Bitcoin basics #bitcoin");
    assert!(tags_for(&s, &code).contains(&("code".to_string(), "type".to_string())));
    assert!(tags_for(&s, &plain).contains(&("q&a".to_string(), "type".to_string())));
    let graph = graph(PrivateState(s.clone())).await.unwrap().0;
    let nodes = graph["nodes"].as_array().unwrap();
    assert_eq!(
        nodes.iter().find(|n| n["id"] == code).unwrap()["kind"],
        "code"
    );
    assert_eq!(
        nodes.iter().find(|n| n["id"] == plain).unwrap()["kind"],
        "q&a"
    );
    assert_eq!(feed(&s, "", "code", 0).await["items"][0]["kind"], "code");
    assert_eq!(feed(&s, "", "code", 0).await["items"][0]["type"], "code");
    let clip = [("f".to_string(), "clip.mp4".to_string(), "video".to_string())];
    assert_eq!(content_type("memo", false, false, false, &clip), "video");
    assert_eq!(content_type("memo", false, true, false, &clip), "photo");
    assert_eq!(content_type("agent", false, false, false, &[]), "code");
    assert_eq!(content_type("question", false, false, true, &[]), "q&a");
    assert_eq!(content_type("question", false, false, false, &[]), "note");
    assert_eq!(content_type("note", false, false, true, &[]), "note");
    assert_eq!(content_type("memo", false, false, false, &[]), "note");
}

#[tokio::test]
async fn only_what_is_worth_remembering_joins_the_wiki_and_hands_can_override() {
    let (_dir, s) = setup();
    enable(&s);
    // Model replies carry the judgement; the heuristic stands in otherwise.
    assert_eq!(parse_keep("{\"keep\": false, \"tags\": []}"), Some(false));
    assert_eq!(parse_keep("{\"keep\": \"yes\"}"), Some(true));
    assert_eq!(parse_keep("{\"tags\": []}"), None);
    assert!(!heuristic_keep("question", false, false, &[], "ok thanks"));
    assert!(heuristic_keep(
        "question",
        false,
        false,
        &[],
        "what is zig lang anyway"
    ));
    assert!(!heuristic_keep(
        "ask",
        false,
        false,
        &[],
        "who created zig in detail"
    ));
    assert!(heuristic_keep(
        "note",
        false,
        false,
        &[],
        "dinner with mom tonight"
    ));
    assert!(heuristic_keep("question", true, false, &[], "hi"));
    assert!(heuristic_keep(
        "memo",
        false,
        false,
        &[("f".into(), "clip.mp4".into(), "video".into())],
        ""
    ));
    // The analysis judges each message; a two-word lookup stays out.
    let trivial = memo(&s, "ok thanks");
    let substantial = memo(
        &s,
        "Bitcoin mining difficulty adjusts every two weeks of bitcoin blocks",
    );
    assert!(tags_for(&s, &trivial).is_empty());
    assert_eq!(feed(&s, "", "", 0).await["total"], 1);
    assert!(tags_for(&s, &substantial).contains(&("q&a".to_string(), "type".to_string())));
    // An Ask result is derived from notes already kept: never in the Wiki.
    let asked =
        s.db.create("who created zig in detail", "ask", "apple", "system")
            .unwrap()
            .id;
    s.db.event(&asked, "delta", "Andrew Kelley, per your notes.")
        .unwrap();
    s.db.finish(&asked, "ask").unwrap();
    analyze(&s, &asked).await.unwrap();
    assert!(tags_for(&s, &asked).is_empty());
    // By hand: keep the trivial one, drop the substantial one, and back.
    set_kept(&s, &trivial, true).unwrap();
    assert!(tags_for(&s, &trivial).contains(&("q&a".to_string(), "type".to_string())));
    set_kept(&s, &substantial, false).unwrap();
    assert!(tags_for(&s, &substantial).is_empty());
    assert_eq!(feed(&s, "", "", 0).await["total"], 1);
    set_kept(&s, &substantial, true).unwrap();
    assert_eq!(feed(&s, "", "", 0).await["total"], 2);
    // Editing tags keeps a message too.
    set_kept(&s, &substantial, false).unwrap();
    set_tags(&s, &substantial, &["energy".into()]).unwrap();
    assert_eq!(feed(&s, "", "energy", 0).await["total"], 1);
}

#[tokio::test]
async fn deleting_a_tag_removes_it_everywhere_for_good_and_erasure_forgets_the_wiki() {
    let (dir, s) = setup();
    s.db.set_setting(
        "assistance.alice",
        &serde_json::json!({"wiki_enabled":true,"wiki_auto_update":true,"wiki_group_by":"topic","wiki_include_answers":true,"textedit_enabled":true,"textedit_settle_ms":1200,"textedit_style":"concise"}).to_string(),
    )
    .unwrap();
    let typed = memo(
        &s,
        "Mining groups transactions into blocks #bitcoin #mining",
    );
    let inferred = memo(
        &s,
        "Bitcoin mining difficulty adjusts every two weeks of bitcoin blocks",
    );
    let other = memo(&s, "Sourdough starter needs flour and water daily feeding");
    for id in [&typed, &inferred, &other] {
        analyze(&s, id).await.unwrap();
    }
    let names =
        |id: &str| -> Vec<String> { tags_for(&s, id).into_iter().map(|(t, _)| t).collect() };
    assert!(names(&typed).contains(&"mining".to_string()));
    assert!(names(&inferred).contains(&"bitcoin".to_string()));
    // Types and categories are part of what a message is.
    assert!(delete_tag(&s, "note").is_err());
    assert!(delete_tag(&s, "q&a").is_err());
    // The tag leaves every message that carried it; the other tags stay.
    let touched = delete_tag(&s, "#Bitcoin").unwrap();
    assert_eq!(touched.len(), 2);
    assert!(!names(&typed).contains(&"bitcoin".to_string()));
    assert!(names(&typed).contains(&"mining".to_string()));
    assert!(!names(&inferred).contains(&"bitcoin".to_string()));
    assert!(names(&other).contains(&"sourdough".to_string()));
    let feed = feed(&s, "", "", 0).await;
    assert!(
        feed["tags"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["tag"] != "bitcoin")
    );
    assert!(
        pages(&s)
            .iter()
            .all(|(id, _)| *id != document_id("topic", "bitcoin"))
    );
    // A rebuild and a fresh analysis do not bring it back: the messages are curated now.
    let _ = rebuild(PrivateState(s.clone())).await.unwrap();
    analyze(&s, &inferred).await.unwrap();
    assert!(!names(&inferred).contains(&"bitcoin".to_string()));
    assert_eq!(delete_tag(&s, "no-such-tag").unwrap().len(), 0);
    // Erasing the account forgets the Wiki in SQLite.
    let _ = crate::chat::delete_all(crate::PrivateState(s.clone()))
        .await
        .unwrap();
    let conn = s.db.0.lock().unwrap();
    for table in [
        "wiki_tags",
        "wiki_topics",
        "wiki_sources",
        "wiki_pages",
        "wiki_spaces",
        "code_notes",
    ] {
        let n: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE owner=?1"),
                ["alice"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    assert!(!dir.path().join("wiki").join(hash("alice")).exists());
}

#[tokio::test]
async fn a_message_left_out_of_the_wiki_still_shows_its_inferred_tags_in_the_chat() {
    let (_dir, s) = setup();
    enable(&s);
    let room = crate::chat::create(
        PrivateState(s.clone()),
        Json(crate::chat::NewRoom {
            roomid: None,
            title: Some("Rust".into()),
        }),
    )
    .await
    .unwrap()
    .0
    .roomid;
    let text = "What is Rust used for?";
    let id = memo(&s, text);
    s.db.assign_room("memos", &id, &room).unwrap();
    let mut reply = ChatMessage {
        roomid: room.clone(),
        messageid: db::id(),
        role: "assistant".into(),
        text: "Rust is a systems language.".into(),
        memo_id: id.clone(),
        provider: "mlx".into(),
        status: "complete".into(),
        created_at: db::now(),
        ..Default::default()
    };
    {
        let mut conn = s.db.0.lock().unwrap();
        let tx = conn.transaction().unwrap();
        crate::chat::write(&tx, &mut reply).unwrap();
        // What the on-device model returned for this message: tags and a
        // category, but "not worth keeping".
        save_topics(
            &tx,
            "alice",
            &id,
            &["rust".into(), "systems-programming".into()],
            &["memory".into()],
            "mlx",
            "information",
            false,
        )
        .unwrap();
        sync_locked(&tx, "alice", false).unwrap();
        tx.commit().unwrap();
    }
    let doc = message_doc(&s, &reply.messageid);
    assert_eq!(doc.wiki_state, "skipped");
    assert_eq!(doc.tags, ["information", "rust", "systems-programming"]);
    assert!(tags_for(&s, &id).is_empty(), "nothing joins the Wiki");
    // A sync that changes nothing writes no revision.
    let _ = rebuild(PrivateState(s.clone())).await.unwrap();
    assert_eq!(
        message_doc(&s, &reply.messageid).messagesyncid,
        doc.messagesyncid
    );
    // Keeping it moves it into the Wiki; the chat then shows the Wiki tags.
    set_kept(&s, &id, true).unwrap();
    let kept = message_doc(&s, &reply.messageid);
    assert_eq!(kept.wiki_state, "kept");
    assert!(kept.tags.contains(&"rust".to_string()), "{:?}", kept.tags);
    assert!(!tags_for(&s, &id).is_empty());
}

#[test]
fn media_notes_are_stripped_before_tagging_and_media_is_always_kept() {
    // What a picture kept from the Images page looks like: no words from the
    // user, the backend's notes, Apple Vision's labels.
    let text = analysis_text(
        "harbour.png",
        "Attached file: harbour.png (ID 0123456789abcdef0123456789abcdef)\n[Picture kept. No text was found in the picture]\nSeen in the picture: harbour, boats, water",
        "",
    );
    assert_eq!(
        text,
        "harbour.png\nFile: harbour.png\nharbour, boats, water"
    );
    // The frequency heuristic names the subject, not the note.
    let (topics, keywords) = heuristic_topics(&text);
    assert_eq!(topics, ["harbour"]);
    assert!(
        !keywords
            .iter()
            .any(|k| k == "attached" || k == "picture" || k == "kept")
    );
    // A clip with a transcript of its frame keeps the words, drops the labels.
    assert_eq!(
        analysis_text(
            "talk.mp4",
            "Attached file: talk.mp4 (ID f1)\nText visible in a frame of the video:\nQuarterly planning\n[Video kept; one frame is shown in the Wiki.]",
            ""
        ),
        "talk.mp4\nFile: talk.mp4\nQuarterly planning"
    );
    // Plain text is untouched.
    assert_eq!(analysis_text("A", "b c", "d"), "A\nb c\nd");
    // A picture or a clip is kept whatever the model said; text follows the
    // model, then the heuristic; an Ask result never joins.
    assert!(keep_decision(false, true, Some(false), false));
    assert!(keep_decision(false, true, None, false));
    assert!(!keep_decision(true, true, Some(true), true));
    assert!(!keep_decision(false, false, Some(false), true));
    assert!(keep_decision(false, false, Some(true), false));
    assert!(keep_decision(false, false, None, true));
    assert!(!keep_decision(false, false, None, false));
    // Save anything and filed notes are deliberate, like media.
    assert!(saved_on_purpose("memo") && saved_on_purpose("note"));
    assert!(!saved_on_purpose("question") && !saved_on_purpose("ask"));
}

/// A short personal line saved on purpose joins the Wiki with tags even when a
/// model judged it small talk (as Qwen 27B did with "my bike is a unicycle").
#[tokio::test]
async fn saved_notes_always_join_the_wiki_with_tags() {
    let (_dir, s) = setup();
    enable(&s);
    let id =
        s.db.create("내 자전거는 외발자전거", "memo", "local", "")
            .unwrap()
            .id;
    s.db.finish(&id, "memo").unwrap();
    // What the model said before the fix.
    {
        let conn = s.db.0.lock().unwrap();
        save_topics(
            &conn,
            "alice",
            &id,
            &["greeting".into(), "test".into()],
            &["bicycle".into()],
            "mlx",
            "other",
            false,
        )
        .unwrap();
    }
    assert!(tags_for(&s, &id).is_empty(), "not kept, so no tags");
    // The one-time repair analyzes it again (heuristically here) and keeps it.
    let rows = saved_to_reanalyze(&s).unwrap();
    assert_eq!(rows, [(id.clone(), "alice".to_string())]);
    analyze(&s, &id).await.unwrap();
    let tags = tags_for(&s, &id);
    assert!(
        tags.iter().any(|(t, o)| t == "note" && o == "type"),
        "{tags:?}"
    );
    assert!(tags.iter().any(|(_, o)| o == "heuristic"), "{tags:?}");
    assert_eq!(feed(&s, "", "", 0).await["total"], 1);
    // It runs once: a later manual removal from the Wiki is respected.
    set_kept(&s, &id, false).unwrap();
    assert!(saved_to_reanalyze(&s).unwrap().is_empty());
    assert!(tags_for(&s, &id).is_empty());
    // A question that a model judged not worth keeping stays out.
    let q = s.db.create("ok thanks", "question", "mlx", "m").unwrap().id;
    s.db.finish(&q, "question").unwrap();
    {
        let conn = s.db.0.lock().unwrap();
        save_topics(
            &conn,
            "alice",
            &q,
            &["thanks".into()],
            &[],
            "mlx",
            "other",
            false,
        )
        .unwrap();
    }
    s.db.0
        .lock()
        .unwrap()
        .execute("DELETE FROM settings WHERE key=?1", [SAVED_REPAIR])
        .unwrap();
    let again = saved_to_reanalyze(&s).unwrap();
    assert!(again.iter().all(|(memo, _)| memo != &q), "{again:?}");
}

#[tokio::test]
async fn repair_tags_messages_whose_analysis_never_ran() {
    let (_dir, s) = setup();
    enable(&s);
    // A kept picture whose background analysis died with the backend: complete,
    // but no analysis row, so the chat would say "Tagging…" forever.
    let m = s
        .db
        .create(
            "Attached file: harbour.png (ID f-pic)\n[Picture kept. No text was found in the picture]\nSeen in the picture: harbour, boats",
            "memo",
            "local",
            "",
        )
        .unwrap();
    s.db.finish(&m.id, "memo").unwrap();
    s.db.update(
        &m.id,
        Some("harbour.png"),
        Some(&["harbour".into(), "boats".into()]),
        None,
    )
    .unwrap();
    {
        let conn = s.db.0.lock().unwrap();
        conn.execute("INSERT INTO files(id,name,mime,bytes,created_at,owner_wallet,origin_memo,path) VALUES('f-pic','harbour.png','image/png',x'00','2026-10-09T00:00:02Z','alice',?1,'')", [&m.id]).unwrap();
    }
    // An Ask result is never repaired into the Wiki.
    let asked =
        s.db.create("What did I note?", "ask", "apple", "system")
            .unwrap();
    s.db.finish(&asked.id, "ask").unwrap();
    // Old history that was never analyzed is left alone: the repair is for
    // what a stopped backend left behind, not a retagging of everything.
    let old =
        s.db.create("Old untagged thought", "memo", "local", "")
            .unwrap();
    s.db.finish(&old.id, "memo").unwrap();
    s.db.0
        .lock()
        .unwrap()
        .execute(
            "UPDATE memos SET created_at='2026-01-01T00:00:00+00:00' WHERE id=?1",
            [&old.id],
        )
        .unwrap();
    assert!(tags_for(&s, &m.id).is_empty());
    repair_later(&s);
    for _ in 0..100 {
        if !tags_for(&s, &m.id).is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let tags = tags_for(&s, &m.id);
    assert!(
        tags.contains(&("photo".to_string(), "type".to_string())),
        "{tags:?}"
    );
    assert!(
        tags.contains(&("harbour".to_string(), "user".to_string())),
        "{tags:?}"
    );
    assert!(topic_row(&s, &asked.id).is_none());
    assert!(topic_row(&s, &old.id).is_none());
    // The picture is listed with its message and tags, so the Images page can
    // edit them like the chat does; a loose file has neither.
    let images = crate::library::images(PrivateState(s.clone()))
        .await
        .unwrap()
        .0;
    let pic = images.iter().find(|i| i.id == "f-pic").unwrap();
    assert_eq!(pic.memo_id, m.id);
    assert!(pic.tags.iter().any(|t| t == "harbour"));
    assert_eq!(pic.tags[0], "photo");
    {
        let conn = s.db.0.lock().unwrap();
        conn.execute("INSERT INTO files(id,name,mime,bytes,created_at,owner_wallet,path) VALUES('f-loose','loose.png','image/png',x'00','2026-10-09T00:00:03Z','alice','')", []).unwrap();
        conn.execute("INSERT INTO files(id,name,mime,bytes,created_at,owner_wallet,origin_memo,path) VALUES('f-clip','clip.mp4','video/mp4',x'00','2026-10-09T00:00:04Z','alice',?1,'')", [&m.id]).unwrap();
    }
    let images = crate::library::images(PrivateState(s.clone()))
        .await
        .unwrap()
        .0;
    let loose = images.iter().find(|i| i.id == "f-loose").unwrap();
    assert!(loose.memo_id.is_empty() && loose.tags.is_empty());
    let videos = crate::library::videos(PrivateState(s.clone()))
        .await
        .unwrap()
        .0;
    let clip = videos.iter().find(|v| v["id"] == "f-clip").unwrap();
    assert_eq!(clip["memo_id"], m.id);
    assert!(
        clip["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "harbour")
    );
    // Editing the tags from there is the same edit as in the chat: curated,
    // and the Wiki follows (a notification tells open pages).
    let before = wiki_notifications(&s);
    let saved = set_tags(&s, &m.id, &["sea".into()]).unwrap();
    assert_eq!(saved, ["photo", "information", "sea"]);
    assert!(wiki_notifications(&s) > before);
    // Another wallet's file is not found through this one.
    assert!(s.for_owner("bob").db.file_memo("f-pic").unwrap().is_none());
}
