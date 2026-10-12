use axum::{body::Body, http::Request};
use causewaybay_server::{AppState, app, capture};
use http_body_util::BodyExt;
use image::{GenericImageView, ImageFormat};
use std::io::Cursor;
use tower::ServiceExt;

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8])
    });
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, ImageFormat::Png).unwrap();
    output.into_inner()
}

#[tokio::test]
async fn thumbnails_bound_dimensions_without_upscaling_and_flatten_transparency()
-> anyhow::Result<()> {
    for (width, height, expected) in [(2624, 1696, (960, 620)), (800, 2400, (320, 960))] {
        let original = png(width, height);
        let jpeg = capture::thumbnail(original.clone()).await?;
        assert_eq!(capture::image_mime(&jpeg), Some("image/jpeg"));
        assert_eq!(image::load_from_memory(&jpeg)?.dimensions(), expected);
        assert!(jpeg.len() < original.len());
    }
    let mut transparent = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(24, 16, image::Rgba([0, 0, 0, 0]))
        .write_to(&mut transparent, ImageFormat::Png)?;
    let image = image::load_from_memory(&capture::thumbnail(transparent.into_inner()).await?)?;
    assert_eq!(image.dimensions(), (24, 16));
    assert!(
        image
            .to_rgb8()
            .pixels()
            .all(|p| p.0.iter().all(|v| *v >= 250))
    );
    assert!(capture::thumbnail(b"not an image".to_vec()).await.is_err());
    // The decoder refuses dimensions above the allocation limits before resizing.
    assert!(capture::thumbnail(png(16001, 1)).await.is_err());
    Ok(())
}

#[tokio::test]
async fn private_thumbnail_routes_preserve_originals_and_follow_deletion() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let state = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let original = png(1800, 900);
    let memo = state.db.create("test", "capture", "test", "test")?;
    state.db.set_image(&memo.id, &original)?;
    let file = state.db.put_file("photo.png", "image/png", &original)?;
    let router = app(state.clone(), "dist".into());
    for (thumbnail, full) in [
        (
            format!("/api/memos/{}/thumbnail", memo.id),
            format!("/api/memos/{}/image", memo.id),
        ),
        (
            format!("/api/files/{}/thumbnail", file.id),
            format!("/api/files/{}/download", file.id),
        ),
    ] {
        let res = router
            .clone()
            .oneshot(Request::get(&thumbnail).body(Body::empty())?)
            .await?;
        assert_eq!(res.status(), 200);
        assert_eq!(res.headers()["content-type"], "image/jpeg");
        assert_eq!(res.headers()["cache-control"], "no-store");
        let jpeg = res.into_body().collect().await?.to_bytes();
        assert_eq!(image::load_from_memory(&jpeg)?.dimensions(), (960, 480));
        let res = router
            .clone()
            .oneshot(Request::get(&full).body(Body::empty())?)
            .await?;
        assert_eq!(res.status(), 200);
        assert_eq!(
            res.into_body().collect().await?.to_bytes().as_ref(),
            original
        );
    }
    assert_eq!(state.db.image(&memo.id)?.unwrap(), original);
    assert_eq!(state.db.file(&file.id)?.1, original);
    let response = router
        .clone()
        .oneshot(Request::get(format!("/api/files/{}/image", file.id)).body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/png");
    assert!(response.headers().get("content-disposition").is_none());
    assert_eq!(
        response.into_body().collect().await?.to_bytes().as_ref(),
        original
    );
    let text = state
        .db
        .put_file("fake.png", "text/plain", b"not an image")?;
    for (suffix, status) in [("image", 415), ("thumbnail", 422)] {
        assert_eq!(
            router
                .clone()
                .oneshot(
                    Request::get(format!("/api/files/{}/{suffix}", text.id)).body(Body::empty())?
                )
                .await?
                .status(),
            status
        );
    }
    for (delete, thumbnail) in [
        (
            format!("/api/memos/{}/image", memo.id),
            format!("/api/memos/{}/thumbnail", memo.id),
        ),
        (
            format!("/api/files/{}", file.id),
            format!("/api/files/{}/thumbnail", file.id),
        ),
    ] {
        assert_eq!(
            router
                .clone()
                .oneshot(Request::delete(delete).body(Body::empty())?)
                .await?
                .status(),
            200
        );
        assert_eq!(
            router
                .clone()
                .oneshot(Request::get(thumbnail).body(Body::empty())?)
                .await?
                .status(),
            404
        );
    }
    Ok(())
}
