#[cfg(feature = "semantic")]
fn model() -> anyhow::Result<fastembed::TextEmbedding> {
    use fastembed::{InitOptions, TextEmbedding};
    TextEmbedding::try_new(
        InitOptions::new(causewaybay_server::semantic::MODEL).with_cache_dir("data/models".into()),
    )
}

#[cfg(feature = "semantic")]
#[test]
#[ignore = "Downloads a sentence embedding model on first use; run make test-semantic"]
fn local_model_understands_related_meaning() -> anyhow::Result<()> {
    let vectors = model()?.embed(
        vec![
            "How do I fix a broken automobile?",
            "Repairing a car at the mechanic.",
            "Baking bread with flour and yeast.",
        ],
        None,
    )?;
    let related = causewaybay_server::db::cosine(&vectors[0], &vectors[1]);
    let unrelated = causewaybay_server::db::cosine(&vectors[0], &vectors[2]);
    assert!(
        related > unrelated + 0.15,
        "related={related}, unrelated={unrelated}"
    );
    Ok(())
}

/// A question in one language finds a note kept in another, and clears the
/// search threshold, so "what is my phone number" finds a Korean note.
#[cfg(feature = "semantic")]
#[test]
#[ignore = "Downloads a sentence embedding model on first use; run make test-semantic"]
fn local_model_matches_questions_across_languages() -> anyhow::Result<()> {
    let vectors = model()?.embed(
        vec![
            "내 전화번호 +82 10 1234 5678",
            "what is my phone number",
            "내 전화번호 뭐야?",
            "Baking bread with flour and yeast.",
            "What is my email address?",
        ],
        None,
    )?;
    let english = causewaybay_server::db::cosine(&vectors[0], &vectors[1]);
    let korean = causewaybay_server::db::cosine(&vectors[0], &vectors[2]);
    let unrelated = causewaybay_server::db::cosine(&vectors[0], &vectors[3]);
    let nearby = causewaybay_server::db::cosine(&vectors[0], &vectors[4]);
    eprintln!("english={english} korean={korean} unrelated={unrelated} nearby={nearby}");
    assert!(
        english > 0.4 && korean > 0.4 && english > unrelated + 0.2,
        "english={english}, korean={korean}, unrelated={unrelated}"
    );
    Ok(())
}

/// Prints how long one query and one long note take to embed on this Mac.
#[cfg(feature = "semantic")]
#[test]
#[ignore = "Timing only; run with --ignored --nocapture"]
fn embedding_latency() -> anyhow::Result<()> {
    let mut model = model()?;
    model.embed(vec!["warm up"], None)?;
    let start = std::time::Instant::now();
    for _ in 0..20 {
        model.embed(vec!["what is my phone number"], None)?;
    }
    let query = start.elapsed() / 20;
    let long = "내 전화번호와 주소, 그리고 오늘 회의에서 정한 일정을 기록해 둔다. ".repeat(20);
    let start = std::time::Instant::now();
    model.embed(vec![long.as_str()], None)?;
    eprintln!("query={query:?} long_note={:?}", start.elapsed());
    Ok(())
}
