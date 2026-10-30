//! ComfyUI's own copies of results and input pictures are cleared between jobs, so a
//! server's disk only holds what's in the gallery.

use outpost_gateway::{AppState, Config};

#[tokio::test]
async fn comfyui_copies_are_cleared_but_nothing_else() {
    let pool = outpost_core::db::open_in_memory().await.unwrap();
    let mut config = Config::local_default();
    config.data_dir = std::env::temp_dir().join(format!("outpost-comfy-copies-{}", std::process::id()));
    let state = AppState::with_pool(pool, config);
    let app = state.engines.comfyui_app_dir();
    let write = |rel: &str| {
        let path = app.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"x").unwrap();
        path
    };
    let result = write("output/outpost/z_image_turbo_00001_.png");
    let video = write("output/outpost/wan/clip_00001.mp4");
    let upload = write("input/outpost/photo.png");
    // Someone's own ComfyUI work, outside Outpost's folders, stays.
    let theirs = write("output/my_render.png");

    outpost_gateway::create::clear_comfy_copies(&state).await;
    assert!(!result.exists() && !video.exists() && !upload.exists());
    assert!(theirs.exists());
    // Nothing there: still fine.
    outpost_gateway::create::clear_comfy_copies(&state).await;
    let _ = std::fs::remove_dir_all(&state.config.data_dir);
}
