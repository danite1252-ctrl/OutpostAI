//! Personas: a name, a system prompt, a model and a few settings. Each account has
//! its own; apps address them as `persona:<slug>`.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::{now, CoreError, CoreResult};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Persona {
    pub id: String,
    pub account_id: String,
    /// Stable short name; use `persona:<slug>` as the model in the API.
    pub slug: String,
    pub name: String,
    pub avatar_color: String,
    pub system_prompt: String,
    /// Ollama model; `None` means the default chat model.
    pub model: Option<String>,
    /// 0–2; `None` means the model's default.
    pub temperature: Option<f64>,
    /// Ask thinking models to show their reasoning.
    pub think: bool,
    /// Include the account's memories in the prompt.
    pub use_memory: bool,
    /// Speaking voice (Kokoro name, e.g. `af_heart`); `None` = Outpost's default.
    pub voice: Option<String>,
    /// Read replies aloud in chat.
    pub speak_replies: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Fields to create or update a persona. Missing fields keep their current (or default) value.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct PersonaInput {
    pub name: Option<String>,
    pub system_prompt: Option<String>,
    /// Ollama model name. Send an empty string to go back to the default model.
    pub model: Option<String>,
    pub temperature: Option<f64>,
    pub think: Option<bool>,
    pub use_memory: Option<bool>,
    /// Speaking voice; send an empty string for the default.
    pub voice: Option<String>,
    pub speak_replies: Option<bool>,
    pub avatar_color: Option<String>,
    /// The id apps use, as in `persona:<slug>`. Normally made from the name; set it to
    /// rename the id (apps using the old one then need updating).
    pub slug: Option<String>,
}

/// The id a persona gets while it still has the "New persona" placeholder name.
const PLACEHOLDER_SLUG: &str = "new-persona";

fn is_placeholder(slug: &str) -> bool {
    slug == PLACEHOLDER_SLUG
        || slug
            .strip_prefix(PLACEHOLDER_SLUG)
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// A slug unique within the account: `base`, `base-2`, ... `except` is the persona
/// being renamed, which may keep its own slug.
async fn unique_slug(pool: &SqlitePool, account_id: &str, base: &str, except: Option<&str>) -> CoreResult<String> {
    let mut slug = base.to_string();
    let mut i = 2;
    loop {
        let taken: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM personas WHERE account_id = ? AND slug = ? AND id != ?")
                .bind(account_id)
                .bind(&slug)
                .bind(except.unwrap_or(""))
                .fetch_one(pool)
                .await?;
        if taken == 0 {
            return Ok(slug);
        }
        slug = format!("{base}-{i}");
        i += 1;
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    account_id: String,
    slug: String,
    name: String,
    avatar_color: String,
    system_prompt: String,
    model: Option<String>,
    temperature: Option<f64>,
    think: bool,
    use_memory: bool,
    voice: Option<String>,
    speak_replies: bool,
    created_at: i64,
    updated_at: i64,
}

impl From<Row> for Persona {
    fn from(r: Row) -> Self {
        Persona {
            id: r.id,
            account_id: r.account_id,
            slug: r.slug,
            name: r.name,
            avatar_color: r.avatar_color,
            system_prompt: r.system_prompt,
            model: r.model,
            temperature: r.temperature,
            think: r.think,
            use_memory: r.use_memory,
            voice: r.voice,
            speak_replies: r.speak_replies,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

const SELECT: &str = "SELECT id, account_id, slug, name, avatar_color, system_prompt, model, temperature, think, use_memory, voice, speak_replies, created_at, updated_at FROM personas";

const COLORS: [&str; 6] = ["#FFB547", "#FF6A3D", "#9CB4FF", "#7BD88F", "#C9B8FF", "#6FD3FF"];

pub const DEFAULT_PROMPT: &str =
    "You are a helpful, direct assistant. Answer clearly and concisely. If you are not sure, say so.";

/// `My Art Director!` -> `my-art-director`.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-').chars().take(40).collect::<String>();
    if slug.is_empty() {
        "persona".into()
    } else {
        slug
    }
}

/// Model names Ollama accepts: `name`, `name:tag`, `user/name:tag`.
pub fn valid_model_name(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 200
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '/'))
        && !model.contains("..")
        && !model.starts_with('/')
}

fn check(input: &PersonaInput) -> CoreResult<()> {
    if let Some(name) = &input.name {
        let n = name.trim().chars().count();
        if n == 0 || n > 40 {
            return Err(CoreError::Invalid("A persona name must be 1 to 40 characters.".into()));
        }
    }
    if input.system_prompt.as_ref().is_some_and(|p| p.chars().count() > 20_000) {
        return Err(CoreError::Invalid(
            "The system prompt is too long (20,000 characters max).".into(),
        ));
    }
    if let Some(m) = &input.model {
        if !m.is_empty() && !valid_model_name(m) {
            return Err(CoreError::Invalid(format!("\"{m}\" is not a valid model name.")));
        }
    }
    if input.temperature.is_some_and(|t| !(0.0..=2.0).contains(&t)) {
        return Err(CoreError::Invalid("Temperature must be between 0 and 2.".into()));
    }
    if let Some(v) = &input.voice {
        let ok = v.is_empty() || (v.len() <= 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        if !ok {
            return Err(CoreError::Invalid("That isn't a voice name.".into()));
        }
    }
    if let Some(c) = &input.avatar_color {
        let ok = c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|h| h.is_ascii_hexdigit());
        if !ok {
            return Err(CoreError::Invalid("Colours look like #FF6A3D.".into()));
        }
    }
    Ok(())
}

pub async fn list(pool: &SqlitePool, account_id: &str) -> CoreResult<Vec<Persona>> {
    ensure_default(pool, account_id).await?;
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE account_id = ? ORDER BY created_at, name"))
        .bind(account_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Persona::from).collect())
}

/// Every account starts with an "Assistant" persona so chat works straight away.
pub async fn ensure_default(pool: &SqlitePool, account_id: &str) -> CoreResult<()> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM personas WHERE account_id = ?")
        .bind(account_id)
        .fetch_one(pool)
        .await?;
    if n == 0 {
        create(
            pool,
            account_id,
            PersonaInput {
                name: Some("Assistant".into()),
                system_prompt: Some(DEFAULT_PROMPT.into()),
                ..Default::default()
            },
        )
        .await?;
    }
    Ok(())
}

pub async fn get(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<Persona> {
    let row: Option<Row> = sqlx::query_as(&format!("{SELECT} WHERE id = ? AND account_id = ?"))
        .bind(id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?;
    row.map(Persona::from).ok_or(CoreError::NotFound)
}

pub async fn get_by_slug(pool: &SqlitePool, account_id: &str, slug: &str) -> CoreResult<Persona> {
    ensure_default(pool, account_id).await?;
    let row: Option<Row> = sqlx::query_as(&format!("{SELECT} WHERE slug = ? AND account_id = ?"))
        .bind(slug)
        .bind(account_id)
        .fetch_optional(pool)
        .await?;
    row.map(Persona::from).ok_or(CoreError::NotFound)
}

pub async fn create(pool: &SqlitePool, account_id: &str, input: PersonaInput) -> CoreResult<Persona> {
    check(&input)?;
    let name = input
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| CoreError::Invalid("Give the persona a name.".into()))?
        .to_string();

    let base = slugify(input.slug.as_deref().filter(|s| !s.trim().is_empty()).unwrap_or(&name));
    let slug = unique_slug(pool, account_id, &base, None).await?;

    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM personas WHERE account_id = ?")
        .bind(account_id)
        .fetch_one(pool)
        .await?;
    let t = now();
    let persona = Persona {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.to_string(),
        slug,
        name,
        avatar_color: input
            .avatar_color
            .unwrap_or_else(|| COLORS[n as usize % COLORS.len()].to_string()),
        system_prompt: input.system_prompt.unwrap_or_default(),
        model: input.model.filter(|m| !m.is_empty()),
        temperature: input.temperature,
        think: input.think.unwrap_or(false),
        use_memory: input.use_memory.unwrap_or(true),
        voice: input.voice.filter(|v| !v.is_empty()),
        speak_replies: input.speak_replies.unwrap_or(false),
        created_at: t,
        updated_at: t,
    };
    sqlx::query(
        "INSERT INTO personas (id, account_id, slug, name, avatar_color, system_prompt, model, temperature, think, use_memory, voice, speak_replies, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&persona.id)
    .bind(&persona.account_id)
    .bind(&persona.slug)
    .bind(&persona.name)
    .bind(&persona.avatar_color)
    .bind(&persona.system_prompt)
    .bind(&persona.model)
    .bind(persona.temperature)
    .bind(persona.think)
    .bind(persona.use_memory)
    .bind(&persona.voice)
    .bind(persona.speak_replies)
    .bind(t)
    .bind(t)
    .execute(pool)
    .await?;
    Ok(persona)
}

/// Change the given fields. Renaming keeps the slug, so apps keep working, with one
/// exception: a persona still on its "New persona" placeholder id takes its id from
/// the first real name it gets. Send `slug` to change the id on purpose.
pub async fn update(pool: &SqlitePool, account_id: &str, id: &str, input: PersonaInput) -> CoreResult<Persona> {
    check(&input)?;
    let mut p = get(pool, account_id, id).await?;
    if let Some(name) = input.name {
        p.name = name.trim().to_string();
    }
    let wanted = match input.slug.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(slugify(s)),
        None if is_placeholder(&p.slug) && slugify(&p.name) != PLACEHOLDER_SLUG => Some(slugify(&p.name)),
        None => None,
    };
    if let Some(base) = wanted.filter(|b| *b != p.slug) {
        if input.slug.is_some() {
            let taken: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM personas WHERE account_id = ? AND slug = ? AND id != ?")
                    .bind(account_id)
                    .bind(&base)
                    .bind(id)
                    .fetch_one(pool)
                    .await?;
            if taken > 0 {
                return Err(CoreError::Conflict(format!(
                    "Another persona already uses persona:{base}."
                )));
            }
            p.slug = base;
        } else {
            p.slug = unique_slug(pool, account_id, &base, Some(id)).await?;
        }
    }
    if let Some(prompt) = input.system_prompt {
        p.system_prompt = prompt;
    }
    if let Some(model) = input.model {
        p.model = Some(model).filter(|m| !m.is_empty());
    }
    if input.temperature.is_some() {
        p.temperature = input.temperature;
    }
    if let Some(t) = input.think {
        p.think = t;
    }
    if let Some(m) = input.use_memory {
        p.use_memory = m;
    }
    if let Some(v) = input.voice {
        p.voice = Some(v).filter(|v| !v.is_empty());
    }
    if let Some(s) = input.speak_replies {
        p.speak_replies = s;
    }
    if let Some(c) = input.avatar_color {
        p.avatar_color = c;
    }
    p.updated_at = now();
    sqlx::query(
        "UPDATE personas SET name = ?, slug = ?, system_prompt = ?, model = ?, temperature = ?, think = ?, use_memory = ?, avatar_color = ?, voice = ?, speak_replies = ?, updated_at = ?
         WHERE id = ? AND account_id = ?",
    )
    .bind(&p.name)
    .bind(&p.slug)
    .bind(&p.system_prompt)
    .bind(&p.model)
    .bind(p.temperature)
    .bind(p.think)
    .bind(p.use_memory)
    .bind(&p.avatar_color)
    .bind(&p.voice)
    .bind(p.speak_replies)
    .bind(p.updated_at)
    .bind(id)
    .bind(account_id)
    .execute(pool)
    .await?;
    Ok(p)
}

/// Delete a persona. Its chats stay and fall back to the default model and no prompt.
/// The last persona can't be deleted.
pub async fn delete(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<()> {
    get(pool, account_id, id).await?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM personas WHERE account_id = ?")
        .bind(account_id)
        .fetch_one(pool)
        .await?;
    if n <= 1 {
        return Err(CoreError::Forbidden("Keep at least one persona.".into()));
    }
    sqlx::query("DELETE FROM personas WHERE id = ? AND account_id = ?")
        .bind(id)
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::db::open_in_memory;

    async fn account(pool: &SqlitePool, name: &str) -> String {
        accounts::create(
            pool,
            NewAccount {
                name: name.into(),
                role: Role::Owner,
                secret: Some("4821".into()),
            },
        )
        .await
        .unwrap()
        .id
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("My Art Director!"), "my-art-director");
        assert_eq!(slugify("  Dungeon   Master "), "dungeon-master");
        assert_eq!(slugify("!!!"), "persona");
    }

    #[tokio::test]
    async fn default_persona_and_unique_slugs() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan").await;
        let list = list(&pool, &dan).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].slug, "assistant");

        let again = create(
            &pool,
            &dan,
            PersonaInput {
                name: Some("Assistant".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(again.slug, "assistant-2");
        assert!(get_by_slug(&pool, &dan, "assistant-2").await.is_ok());
    }

    #[tokio::test]
    async fn personas_are_private_to_their_account() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan").await;
        let sarah = account(&pool, "Sarah").await;
        let p = create(
            &pool,
            &dan,
            PersonaInput {
                name: Some("Coder".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(matches!(get(&pool, &sarah, &p.id).await, Err(CoreError::NotFound)));
        assert!(matches!(
            update(&pool, &sarah, &p.id, PersonaInput::default()).await,
            Err(CoreError::NotFound)
        ));
    }

    #[tokio::test]
    async fn update_validates_and_keeps_slug() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan").await;
        let p = create(
            &pool,
            &dan,
            PersonaInput {
                name: Some("Coder".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let bad = update(
            &pool,
            &dan,
            &p.id,
            PersonaInput {
                temperature: Some(3.0),
                ..Default::default()
            },
        )
        .await;
        assert!(matches!(bad, Err(CoreError::Invalid(_))));
        let p2 = update(
            &pool,
            &dan,
            &p.id,
            PersonaInput {
                name: Some("Senior Coder".into()),
                model: Some("huihui_ai/qwen3.5-abliterated:9b".into()),
                think: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(p2.slug, "coder");
        assert_eq!(p2.name, "Senior Coder");
        assert!(p2.think);
        assert_eq!(p2.model.as_deref(), Some("huihui_ai/qwen3.5-abliterated:9b"));
    }

    #[tokio::test]
    async fn new_persona_takes_its_id_from_its_first_name() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan").await;
        let new = |name: &str| PersonaInput {
            name: Some(name.into()),
            ..Default::default()
        };
        let a = create(&pool, &dan, new("New persona")).await.unwrap();
        let b = create(&pool, &dan, new("New persona")).await.unwrap();
        assert_eq!(a.slug, "new-persona");
        assert_eq!(b.slug, "new-persona-2");

        let a = update(&pool, &dan, &a.id, new("Anna")).await.unwrap();
        assert_eq!(a.slug, "anna");
        // Once it has a real id, renaming keeps it.
        let a = update(&pool, &dan, &a.id, new("Anna Smith")).await.unwrap();
        assert_eq!(a.slug, "anna");
        // A second "Anna" gets anna-2 rather than clashing.
        let b = update(&pool, &dan, &b.id, new("Anna")).await.unwrap();
        assert_eq!(b.slug, "anna-2");

        // Changing the id on purpose: taken ids are refused, free ones are tidied.
        let clash = update(
            &pool,
            &dan,
            &b.id,
            PersonaInput {
                slug: Some("anna".into()),
                ..Default::default()
            },
        )
        .await;
        assert!(matches!(clash, Err(CoreError::Conflict(_))));
        let b = update(
            &pool,
            &dan,
            &b.id,
            PersonaInput {
                slug: Some("Anna Two!".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(b.slug, "anna-two");
        assert!(get_by_slug(&pool, &dan, "anna-two").await.is_ok());
    }
}
