use causewaybay_server::providers::{self, Config, Message};

fn config() -> anyhow::Result<Config> {
    let key = std::env::var("XAI_API_KEY")
        .map_err(|_| anyhow::anyhow!("Set XAI_API_KEY in your environment to run the live test"))?;
    anyhow::ensure!(!key.is_empty(), "XAI_API_KEY is empty");
    Ok(Config {
        owner: "local".into(),
        local: Default::default(),
        runtime: Default::default(),
        provider: "grok".into(),
        model: std::env::var("XAI_MODEL").unwrap_or_else(|_| "grok-4".into()),
        key,
        progress: None,
        max_tokens: None,
    })
}

#[tokio::test]
#[ignore = "Makes a paid live call using XAI_API_KEY; run make test-cloud"]
async fn grok_streams_real_text_without_persisting_credentials() -> anyhow::Result<()> {
    let config = config()?;
    let mut streamed = String::new();
    let result = providers::stream(
        &config,
        "Answer exactly as requested.",
        &[Message::User(
            "What is 7 + 5? Reply with only the number.".into(),
        )],
        &[],
        |part| {
            streamed.push_str(part);
            Ok(())
        },
    )
    .await?;
    assert_eq!(result.text.trim(), "12");
    assert_eq!(streamed, result.text);
    Ok(())
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "Makes a paid live image call using XAI_API_KEY; run make test-cloud"]
async fn grok_reads_a_real_image() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("image.png");
    apple_vision::recognize_text::_test_helper_render_text_png("What is 7 + 5?", 900, 180, &path)?;
    let (mime, data) = causewaybay_server::capture::cloud_image(std::fs::read(path)?).await?;
    let result = providers::stream(
        &config()?,
        "Answer exactly as requested.",
        &[Message::Image {
            text: "Solve the arithmetic question in the image. Reply with only the number.".into(),
            images: vec![(mime, data)],
        }],
        &[],
        |_| Ok(()),
    )
    .await?;
    assert_eq!(result.text.trim(), "12");
    Ok(())
}
