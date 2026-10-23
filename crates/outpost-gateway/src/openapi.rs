use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::error::{ErrorBody, ErrorDetail};
use crate::routes::{accounts, auth, chat, engines, health, keys, models, network, openai, setup, SignInResponse};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Outpost API",
        description = "Local API for Outpost. Every feature in the desktop app is available here.",
        license(name = "MIT")
    ),
    paths(
        health::health,
        setup::create_owner,
        auth::directory,
        auth::sign_in,
        auth::sign_out,
        auth::me,
        auth::change_my_secret,
        accounts::list,
        accounts::create,
        accounts::delete,
        accounts::reset_secret,
        models::list_models,
        openai::chat_completions,
        chat::list_personas,
        chat::create_persona,
        chat::update_persona,
        chat::delete_persona,
        chat::list_chats,
        chat::create_chat,
        chat::get_chat,
        chat::update_chat,
        chat::delete_chat,
        chat::send_message,
        chat::list_memory,
        chat::add_memory,
        chat::delete_memory,
        keys::list,
        keys::create,
        keys::revoke,
        models::hardware,
        models::overview,
        models::pull,
        models::remove,
        engines::status,
        engines::install,
        engines::start,
        engines::stop,
        network::get_network,
        network::set_network,
    ),
    components(schemas(
        ErrorBody,
        ErrorDetail,
        SignInResponse,
        outpost_core::accounts::Account,
        outpost_core::accounts::Role,
        outpost_core::sessions::Session,
        outpost_core::sessions::SessionKind,
    )),
    modifiers(&BearerAuth),
    tags(
        (name = "system", description = "Health and settings"),
        (name = "engines", description = "Ollama and ComfyUI: install, start, stop"),
        (name = "models", description = "Hardware, model catalogue and downloads"),
        (name = "chat", description = "Personas, chats with streamed replies, memory"),
        (name = "keys", description = "App keys for Hermes and other apps"),
        (name = "setup", description = "First run"),
        (name = "auth", description = "Signing in and out"),
        (name = "accounts", description = "Managing accounts (owner)"),
        (name = "openai", description = "OpenAI-compatible endpoints"),
    )
)]
pub struct ApiDoc;

struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
        );
    }
}

/// The full OpenAPI document as pretty JSON.
pub fn spec_json() -> String {
    ApiDoc::openapi().to_pretty_json().unwrap_or_else(|_| "{}".into())
}
