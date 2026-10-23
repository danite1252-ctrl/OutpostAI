use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::error::{ErrorBody, ErrorDetail};
use crate::routes::{accounts, auth, engines, health, models, setup, SignInResponse};

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
        models::hardware,
        models::overview,
        models::pull,
        models::remove,
        engines::status,
        engines::install,
        engines::start,
        engines::stop,
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
        (name = "system", description = "Health"),
        (name = "engines", description = "Ollama and ComfyUI: install, start, stop"),
        (name = "models", description = "Hardware, model catalogue and downloads"),
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
