use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::ApiResult;
use crate::AppState;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Health {
    /// Always `ok` when the gateway answers.
    pub status: String,
    /// Outpost version.
    pub version: String,
    /// This Outpost's name (the PC name), shown to phones later.
    pub name: String,
    /// True until the first owner account exists.
    pub setup_required: bool,
}

/// Is the gateway up, and does it still need its first owner account?
#[utoipa::path(get, path = "/api/health", tag = "system",
    responses((status = 200, description = "Gateway is up", body = Health)))]
pub async fn health(State(state): State<AppState>) -> ApiResult<Json<Health>> {
    let setup_required = !outpost_core::accounts::owner_exists(&state.pool).await?;
    Ok(Json(Health {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        name: machine_name(),
        setup_required,
    }))
}

fn machine_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Outpost".into())
}
