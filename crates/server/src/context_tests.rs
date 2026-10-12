use super::*;

#[tokio::test]
async fn text_context_isolated_after_new_thought_or_chat() -> Result<()> {
    check_context(0).await
}

#[tokio::test]
async fn single_capture_context_isolated_after_new_thought_or_chat() -> Result<()> {
    check_context(1).await
}

#[tokio::test]
async fn multi_capture_context_isolated_after_new_thought_or_chat() -> Result<()> {
    check_context(8).await
}

async fn check_context(photo_count: usize) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let old = chat::thought(
        PrivateState(s.clone()),
        Json(chat::ThoughtRequest {
            fresh: true,
            ..Default::default()
        }),
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.1))?
    .0;
    let memo = s.db.create("isolationneedle", "question", "demo", "test")?;
    s.db.assign_room("memos", &memo.id, &old.roomid)?;
    let file =
        s.db.put_file("old-problem.txt", "text/plain", b"OLD_ATTACHMENT")?;
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(8, 8, image::Rgb([255, 0, 0]))
        .write_to(&mut encoded, image::ImageFormat::Png)?;
    let old_photo =
        s.db.put_file("old-photo.png", "image/png", &encoded.into_inner())?;
    chat::attach(
        &s.db,
        &old.roomid,
        &db::id(),
        &memo,
        &ThinkRequest {
            prompt: "isolationneedle OLD_PROMPT".into(),
            file_ids: vec![file.id, old_photo.id],
            input_mode: InputMode::Image,
            ..Default::default()
        },
    )?;
    s.db.event(&memo.id, "delta", "isolationneedle OLD_ANSWER")?;
    s.db.finish(&memo.id, "question")?;
    // Prove this answer would be retrieved by the old implicit library search.
    assert!(
        search(&s, "isolationneedle", "all", 0, 4)
            .await?
            .memos
            .iter()
            .any(|m| m.id == memo.id)
    );

    let fresh = chat::thought(
        PrivateState(s.clone()),
        Json(chat::ThoughtRequest {
            fresh: true,
            ..Default::default()
        }),
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.1))?
    .0;
    let new = chat::create(
        PrivateState(s.clone()),
        Json(chat::NewRoom {
            roomid: None,
            title: None,
        }),
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.1))?
    .0;
    assert_ne!(old.roomid, fresh.roomid);
    assert_ne!(fresh.roomid, new.roomid);
    assert_eq!(
        chat::thought(
            PrivateState(s.clone()),
            Json(chat::ThoughtRequest::default())
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.1))?
        .0
        .roomid,
        fresh.roomid
    );

    for room in [&fresh, &new, &old] {
        let current = s.db.create("isolationneedle", "question", "demo", "test")?;
        s.db.assign_room("memos", &current.id, &room.roomid)?;
        let req = ThinkRequest {
            room_id: Some(room.roomid.clone()),
            input_mode: if photo_count == 0 {
                InputMode::Text
            } else {
                InputMode::Image
            },
            // A stale parent must not override the explicit fresh room either.
            parent_id: Some(memo.id.clone()),
            ..Default::default()
        };
        chat::attach(&s.db, &room.roomid, &db::id(), &current, &req)?;
        let (messages, _) = answer_messages(
            &s,
            &current.id,
            &req,
            "isolationneedle".into(),
            (0..photo_count)
                .map(|i| ("image/png".into(), format!("CURRENT_PHOTO_{i}")))
                .collect(),
        )
        .await?;
        let image_count: usize = messages
            .iter()
            .map(|message| match message {
                providers::Message::Image { images, .. } => images.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(
            image_count,
            photo_count + usize::from(photo_count > 0 && room.roomid == old.roomid)
        );
        for provider in ["openai", "anthropic", "grok", "gemini"] {
            let body = providers::request_body(
                &providers::Config {
                    provider: provider.into(),
                    model: "test".into(),
                    key: String::new(),
                    ..Default::default()
                },
                "system",
                &messages,
                &[],
            )
            .to_string();
            assert_eq!(body.contains("CURRENT_PHOTO"), photo_count > 0);
            for index in 0..photo_count {
                assert!(
                    body.contains(&format!("CURRENT_PHOTO_{index}")),
                    "Missing current image {index} for {provider}"
                );
            }
            assert!(!body.contains("reference_memos"));
            for previous in ["OLD_PROMPT", "OLD_ANSWER", "OLD_ATTACHMENT"] {
                assert_eq!(
                    body.contains(previous),
                    room.roomid == old.roomid,
                    "{previous}: {body}"
                );
            }
        }
        if room.roomid != old.roomid {
            assert_eq!(messages.len(), 1);
        }
    }
    // Starting fresh preserves saved work; returning to its room can continue it.
    assert!(s.db.get(&memo.id)?.is_some());
    Ok(())
}
