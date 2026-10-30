//! LoRAs are per account. LoRAs from the shared-folder days move to their owners.

use outpost_core::accounts::{self, NewAccount, Role};
use outpost_core::training::{self, NewCharacter};
use outpost_gateway::{AppState, Config};

#[tokio::test]
async fn shared_loras_with_a_known_owner_become_theirs() {
    let pool = outpost_core::db::open_in_memory().await.unwrap();
    let mut config = Config::local_default();
    config.data_dir = std::env::temp_dir().join(format!("outpost-loras-{}", std::process::id()));
    let state = AppState::with_pool(pool.clone(), config);
    let new = |name: &str, role| NewAccount { name: name.into(), role, secret: Some("4821".into()) };
    let dan = accounts::create(&pool, new("Dan", Role::Owner)).await.unwrap();
    let sam = accounts::create(&pool, new("Sam", Role::Member)).await.unwrap();

    let root = state.assets.loras_dir();
    std::fs::create_dir_all(&root).unwrap();
    for f in ["uploaded.safetensors", "kat.safetensors", "style.safetensors"] {
        std::fs::write(root.join(f), b"x").unwrap();
    }
    // Dan uploaded one; Sam trained Kat; nobody owns the style.
    outpost_core::audit::log(&pool, Some(&dan.id), None, "lora_upload", Some("uploaded.safetensors")).await.unwrap();
    training::save_character(
        &pool,
        &sam.id,
        NewCharacter {
            name: "Kat",
            trigger_word: "k4tt",
            lora_file: "kat.safetensors",
            training_id: None,
            step: None,
            subject: None,
            style: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(outpost_gateway::routes::create::claim_shared_loras(&state).await, 2);
    assert_eq!(state.assets.loras(), vec!["style.safetensors"]);
    assert_eq!(state.assets.account_loras(&dan.id), vec![format!("{}/uploaded.safetensors", dan.id)]);
    let kat = format!("{}/kat.safetensors", sam.id);
    assert_eq!(state.assets.account_loras(&sam.id), vec![kat.clone()]);
    // The character follows its file.
    assert_eq!(training::characters(&pool, Some(&sam.id)).await.unwrap()[0].lora_file, kat);
    assert!(state.assets.lora_info(&kat).is_none(), "not a real LoRA, but the path resolves");

    // Each sees their own and the shared style; nobody sees the other's.
    let dans = outpost_gateway::routes::create::visible_loras(&state, &dan.id).await;
    assert_eq!(dans, vec![format!("{}/uploaded.safetensors", dan.id), "style.safetensors".to_string()]);
    let sams = outpost_gateway::routes::create::visible_loras(&state, &sam.id).await;
    assert_eq!(sams, vec![kat, "style.safetensors".to_string()]);

    // Running it again changes nothing.
    assert_eq!(outpost_gateway::routes::create::claim_shared_loras(&state).await, 0);
    let _ = std::fs::remove_dir_all(&state.config.data_dir);
}
