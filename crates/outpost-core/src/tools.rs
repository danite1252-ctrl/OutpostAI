//! Tools on the Outpost server, per account (see migration 0023). A tool belongs to whoever
//! added it; shared, it works in everyone's chats (restricted accounts only when allowed). A
//! persona has a default set; a chat adds to or takes from its persona's.
//!
//! Keys and sign-ins arrive here sealed (the gateway encrypts them) and are stored as given.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::{now, CoreError, CoreResult};

/// Most tools one chat may use (each adds its tools to every request).
pub const MAX_PER_CHAT: usize = 10;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Tool {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub catalog_id: Option<String>,
    /// `stdio`, `http` or `builtin`.
    pub transport: String,
    pub command: String,
    pub args: String,
    pub url: String,
    /// Sealed `{"env":{},"headers":{}}`.
    pub secrets: Option<String>,
    pub secret_names: String,
    /// Sealed sign-in.
    pub oauth: Option<String>,
    pub signed_in_as: Option<String>,
    pub enabled: bool,
    pub shared: bool,
    pub restricted_ok: bool,
    pub disabled_tools: String,
    /// Actions that ask the person first, in every chat (JSON list).
    pub ask_tools: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Tool {
    pub fn args(&self) -> Vec<String> {
        serde_json::from_str(&self.args).unwrap_or_default()
    }
    pub fn disabled_tools(&self) -> Vec<String> {
        serde_json::from_str(&self.disabled_tools).unwrap_or_default()
    }
    pub fn ask_tools(&self) -> Vec<String> {
        serde_json::from_str(&self.ask_tools).unwrap_or_default()
    }
    pub fn secret_names(&self) -> Vec<String> {
        serde_json::from_str(&self.secret_names).unwrap_or_default()
    }
}

/// What the apps may change about a tool (`None`: unchanged). Keys go through the gateway.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct ToolFields {
    pub name: Option<String>,
    pub catalog_id: Option<String>,
    pub transport: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub url: Option<String>,
    pub enabled: Option<bool>,
    pub shared: Option<bool>,
    pub restricted_ok: Option<bool>,
    pub disabled_tools: Option<Vec<String>>,
    /// Actions that ask first, in every chat.
    pub ask_tools: Option<Vec<String>>,
}

const COLS: &str = "id, account_id, name, catalog_id, transport, command, args, url, secrets, secret_names, oauth, signed_in_as, \
                    enabled, shared, restricted_ok, disabled_tools, ask_tools, created_at, updated_at";

fn clean_name(name: &str) -> CoreResult<String> {
    let n: String = name.trim().chars().filter(|c| !c.is_control()).take(60).collect();
    if n.is_empty() {
        return Err(CoreError::Invalid("Give it a name.".into()));
    }
    Ok(n)
}

/// Apply changes and check the result.
pub fn apply(t: &mut Tool, f: &ToolFields) -> CoreResult<()> {
    if let Some(n) = &f.name {
        t.name = clean_name(n)?;
    }
    if let Some(c) = &f.catalog_id {
        t.catalog_id = Some(c.clone()).filter(|c| !c.is_empty());
    }
    if let Some(x) = &f.transport {
        t.transport = x.clone();
    }
    if let Some(c) = &f.command {
        t.command = c.trim().to_string();
    }
    if let Some(a) = &f.args {
        t.args = serde_json::to_string(a).unwrap_or_else(|_| "[]".into());
    }
    if let Some(u) = &f.url {
        t.url = u.trim().to_string();
    }
    if let Some(e) = f.enabled {
        t.enabled = e;
    }
    if let Some(s) = f.shared {
        t.shared = s;
    }
    if let Some(r) = f.restricted_ok {
        t.restricted_ok = r;
    }
    if let Some(d) = &f.disabled_tools {
        t.disabled_tools = serde_json::to_string(d).unwrap_or_else(|_| "[]".into());
    }
    if let Some(a) = &f.ask_tools {
        t.ask_tools = serde_json::to_string(a).unwrap_or_else(|_| "[]".into());
    }
    match t.transport.as_str() {
        "stdio" if t.command.is_empty() => Err(CoreError::Invalid("Give the command that starts it (e.g. npx).".into())),
        "http" if !(t.url.starts_with("https://") || t.url.starts_with("http://")) => {
            Err(CoreError::Invalid("Give its address (https://...).".into()))
        }
        "builtin" if t.command != "google" && t.command != "text" => Err(CoreError::Invalid("There's no built-in tool called that.".into())),
        "stdio" | "http" | "builtin" => Ok(()),
        _ => Err(CoreError::Invalid("A tool runs as a program (stdio), at an address (http), or built in.".into())),
    }
}

/// A new tool from these fields (not yet stored).
pub fn new_tool(account_id: &str, f: &ToolFields) -> CoreResult<Tool> {
    let t0 = now();
    let mut t = Tool {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.into(),
        name: clean_name(f.name.as_deref().unwrap_or(""))?,
        catalog_id: None,
        transport: String::new(),
        command: String::new(),
        args: "[]".into(),
        url: String::new(),
        secrets: None,
        secret_names: "[]".into(),
        oauth: None,
        signed_in_as: None,
        enabled: true,
        shared: false,
        restricted_ok: false,
        disabled_tools: "[]".into(),
        ask_tools: "[]".into(),
        created_at: t0,
        updated_at: t0,
    };
    apply(&mut t, f)?;
    Ok(t)
}

pub async fn insert(pool: &SqlitePool, t: &Tool) -> CoreResult<()> {
    sqlx::query(&format!("INSERT INTO tools ({COLS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"))
        .bind(&t.id)
        .bind(&t.account_id)
        .bind(&t.name)
        .bind(&t.catalog_id)
        .bind(&t.transport)
        .bind(&t.command)
        .bind(&t.args)
        .bind(&t.url)
        .bind(&t.secrets)
        .bind(&t.secret_names)
        .bind(&t.oauth)
        .bind(&t.signed_in_as)
        .bind(t.enabled)
        .bind(t.shared)
        .bind(t.restricted_ok)
        .bind(&t.disabled_tools)
        .bind(&t.ask_tools)
        .bind(t.created_at)
        .bind(t.updated_at)
        .execute(pool)
        .await?;
    Ok(())
}

/// Store a tool's current fields (keys and sign-in included).
pub async fn save(pool: &SqlitePool, t: &Tool) -> CoreResult<()> {
    sqlx::query(
        "UPDATE tools SET name = ?, catalog_id = ?, transport = ?, command = ?, args = ?, url = ?, secrets = ?, secret_names = ?, \
         oauth = ?, signed_in_as = ?, enabled = ?, shared = ?, restricted_ok = ?, disabled_tools = ?, ask_tools = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&t.name)
    .bind(&t.catalog_id)
    .bind(&t.transport)
    .bind(&t.command)
    .bind(&t.args)
    .bind(&t.url)
    .bind(&t.secrets)
    .bind(&t.secret_names)
    .bind(&t.oauth)
    .bind(&t.signed_in_as)
    .bind(t.enabled)
    .bind(t.shared)
    .bind(t.restricted_ok)
    .bind(&t.disabled_tools)
    .bind(&t.ask_tools)
    .bind(now())
    .bind(&t.id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Only a sign-in changed (token renewals: nothing else is touched).
pub async fn save_oauth(pool: &SqlitePool, id: &str, sealed: Option<&str>, signed_in_as: Option<&str>) -> CoreResult<()> {
    sqlx::query("UPDATE tools SET oauth = ?, signed_in_as = ?, updated_at = ? WHERE id = ?")
        .bind(sealed)
        .bind(signed_in_as)
        .bind(now())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// A tool `t` I can use: mine, or shared and not taken away from me (restricted accounts only
/// when the tool allows them, or its owner gave them some of it). Binds: account, account,
/// restricted, account.
fn usable_sql(t: &str) -> String {
    format!(
        "({t}.account_id = ? OR ({t}.shared = 1 \
         AND NOT EXISTS (SELECT 1 FROM tool_people p WHERE p.tool_id = {t}.id AND p.account_id = ? AND p.access = 'none') \
         AND (? = 0 OR {t}.restricted_ok = 1 \
              OR EXISTS (SELECT 1 FROM tool_people p WHERE p.tool_id = {t}.id AND p.account_id = ? AND p.access <> 'none'))))"
    )
}

/// Every tool I can use: mine, and others' shared ones.
pub async fn usable(pool: &SqlitePool, account_id: &str, restricted: bool) -> CoreResult<Vec<Tool>> {
    Ok(sqlx::query_as::<_, Tool>(&format!("SELECT {COLS} FROM tools WHERE {} ORDER BY name COLLATE NOCASE", usable_sql("tools")))
        .bind(account_id)
        .bind(account_id)
        .bind(restricted)
        .bind(account_id)
        .fetch_all(pool)
        .await?)
}

pub async fn get_usable(pool: &SqlitePool, account_id: &str, restricted: bool, id: &str) -> CoreResult<Tool> {
    sqlx::query_as::<_, Tool>(&format!("SELECT {COLS} FROM tools WHERE id = ? AND {}", usable_sql("tools")))
        .bind(id)
        .bind(account_id)
        .bind(account_id)
        .bind(restricted)
        .bind(account_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

/// A tool by id, whoever's (the server's own use: renewing sign-ins).
pub async fn get_any(pool: &SqlitePool, id: &str) -> CoreResult<Tool> {
    sqlx::query_as::<_, Tool>(&format!("SELECT {COLS} FROM tools WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

/// One of my tools (to change or remove).
pub async fn get_own(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<Tool> {
    sqlx::query_as::<_, Tool>(&format!("SELECT {COLS} FROM tools WHERE id = ? AND account_id = ?"))
        .bind(id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

pub async fn delete(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<()> {
    let done = sqlx::query("DELETE FROM tools WHERE id = ? AND account_id = ?").bind(id).bind(account_id).execute(pool).await?;
    if done.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

/// A persona's tools that I can use (a shared persona's personal tools aren't mine).
pub async fn persona_tools(pool: &SqlitePool, account_id: &str, restricted: bool, persona_id: &str) -> CoreResult<Vec<String>> {
    let ids: Vec<(String,)> = sqlx::query_as(&format!(
        "SELECT pt.tool_id FROM persona_tools pt JOIN tools t ON t.id = pt.tool_id
         WHERE pt.persona_id = ? AND {}
         ORDER BY t.name COLLATE NOCASE",
        usable_sql("t")
    ))
    .bind(persona_id)
    .bind(account_id)
    .bind(account_id)
    .bind(restricted)
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(ids.into_iter().map(|(i,)| i).collect())
}

/// Set your persona's tools.
pub async fn set_persona_tools(pool: &SqlitePool, account_id: &str, restricted: bool, persona_id: &str, ids: &[String]) -> CoreResult<Vec<String>> {
    crate::personas::get(pool, account_id, persona_id).await?;
    if ids.len() > MAX_PER_CHAT {
        return Err(CoreError::Invalid(format!("A persona can use up to {MAX_PER_CHAT} tools.")));
    }
    for id in ids {
        get_usable(pool, account_id, restricted, id).await?;
    }
    let mut tx = pool.begin().await?;
    // Others' tools stay (a shared tool someone else can't see is theirs to keep).
    sqlx::query("DELETE FROM persona_tools WHERE persona_id = ? AND tool_id IN (SELECT id FROM tools WHERE account_id = ? OR shared = 1)")
        .bind(persona_id)
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    for id in ids {
        sqlx::query("INSERT OR IGNORE INTO persona_tools (persona_id, tool_id) VALUES (?, ?)").bind(persona_id).bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    persona_tools(pool, account_id, restricted, persona_id).await
}

/// The tools a chat uses: its persona's, plus what the chat added, less what it took away.
pub async fn chat_tools(pool: &SqlitePool, account_id: &str, restricted: bool, chat_id: &str) -> CoreResult<Vec<String>> {
    let chat = crate::chats::get(pool, account_id, chat_id).await?;
    let mut ids = match &chat.persona_id {
        Some(p) => persona_tools(pool, account_id, restricted, p).await?,
        None => Vec::new(),
    };
    let own: Vec<(String, bool)> = sqlx::query_as(&format!(
        "SELECT c.tool_id, c.is_on FROM chat_tools c JOIN tools t ON t.id = c.tool_id WHERE c.chat_id = ? AND {}",
        usable_sql("t")
    ))
    .bind(chat_id)
    .bind(account_id)
    .bind(account_id)
    .bind(restricted)
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    for (id, on) in own {
        if on && !ids.contains(&id) {
            ids.push(id);
        } else if !on {
            ids.retain(|x| x != &id);
        }
    }
    Ok(ids)
}

/// Set exactly which tools a chat uses (kept as the difference from its persona's).
pub async fn set_chat_tools(pool: &SqlitePool, account_id: &str, restricted: bool, chat_id: &str, ids: &[String]) -> CoreResult<Vec<String>> {
    let chat = crate::chats::get(pool, account_id, chat_id).await?;
    if ids.len() > MAX_PER_CHAT {
        return Err(CoreError::Invalid(format!("A chat can use up to {MAX_PER_CHAT} tools.")));
    }
    for id in ids {
        get_usable(pool, account_id, restricted, id).await?;
    }
    let base = match &chat.persona_id {
        Some(p) => persona_tools(pool, account_id, restricted, p).await?,
        None => Vec::new(),
    };
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM chat_tools WHERE chat_id = ?").bind(chat_id).execute(&mut *tx).await?;
    for id in ids.iter().filter(|i| !base.contains(i)) {
        sqlx::query("INSERT OR IGNORE INTO chat_tools (chat_id, tool_id, is_on) VALUES (?, ?, 1)").bind(chat_id).bind(id).execute(&mut *tx).await?;
    }
    for id in base.iter().filter(|b| !ids.contains(b)) {
        sqlx::query("INSERT OR IGNORE INTO chat_tools (chat_id, tool_id, is_on) VALUES (?, ?, 0)").bind(chat_id).bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    chat_tools(pool, account_id, restricted, chat_id).await
}

/// Go back to the persona's tools.
pub async fn reset_chat_tools(pool: &SqlitePool, account_id: &str, chat_id: &str) -> CoreResult<()> {
    crate::chats::get(pool, account_id, chat_id).await?;
    sqlx::query("DELETE FROM chat_tools WHERE chat_id = ?").bind(chat_id).execute(pool).await?;
    Ok(())
}

/// How much of a shared tool someone may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// All its actions (the ones switched on).
    All,
    /// Only the actions listed.
    Some,
    /// None of it.
    None,
}

impl Access {
    fn as_str(self) -> &'static str {
        match self {
            Access::All => "all",
            Access::Some => "some",
            Access::None => "none",
        }
    }
    fn parse(s: &str) -> Access {
        match s {
            "some" => Access::Some,
            "none" => Access::None,
            _ => Access::All,
        }
    }
}

/// Which of a tool's actions someone may use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Allowed {
    Everything,
    Only(Vec<String>),
    Nothing,
}

impl Allowed {
    pub fn allows(&self, action: &str) -> bool {
        match self {
            Allowed::Everything => true,
            Allowed::Only(list) => list.iter().any(|a| a == action),
            Allowed::Nothing => false,
        }
    }
}

/// What the owner set for this person, if anything.
async fn person_row(pool: &SqlitePool, tool_id: &str, account_id: &str) -> CoreResult<Option<(Access, Vec<String>)>> {
    let row: Option<(String, String)> = sqlx::query_as("SELECT access, actions FROM tool_people WHERE tool_id = ? AND account_id = ?")
        .bind(tool_id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(a, list)| (Access::parse(&a), serde_json::from_str(&list).unwrap_or_default())))
}

/// Which of this tool's actions someone may use: all of their own; of a shared one, what its
/// owner allowed them (by default all, restricted accounts only when the tool allows them).
pub async fn allowed(pool: &SqlitePool, t: &Tool, account_id: &str, restricted: bool) -> CoreResult<Allowed> {
    if t.account_id == account_id {
        return Ok(Allowed::Everything);
    }
    if !t.shared {
        return Ok(Allowed::Nothing);
    }
    Ok(match person_row(pool, &t.id, account_id).await? {
        Some((Access::All, _)) => Allowed::Everything,
        Some((Access::Some, list)) => Allowed::Only(list),
        Some((Access::None, _)) => Allowed::Nothing,
        None if restricted && !t.restricted_ok => Allowed::Nothing,
        None => Allowed::Everything,
    })
}

/// One person and what they may use of a shared tool (for its owner).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PersonAccess {
    pub account_id: String,
    pub name: String,
    /// owner, member or restricted.
    pub role: String,
    pub access: Access,
    /// For some: the actions allowed.
    pub actions: Vec<String>,
    /// The owner chose this (else it's the default).
    pub chosen: bool,
}

/// Everyone else in the household and what they may use of my tool.
pub async fn people(pool: &SqlitePool, owner_id: &str, tool_id: &str) -> CoreResult<Vec<PersonAccess>> {
    let t = get_own(pool, owner_id, tool_id).await?;
    let mut out = Vec::new();
    for a in crate::accounts::list(pool).await? {
        if a.id == owner_id || a.role == crate::accounts::Role::Service {
            continue;
        }
        let restricted = a.role == crate::accounts::Role::Restricted;
        let row = person_row(pool, &t.id, &a.id).await?;
        let (access, actions) = match (&row, allowed(pool, &t, &a.id, restricted).await?) {
            (Some((Access::Some, list)), _) => (Access::Some, list.clone()),
            (_, Allowed::Everything) => (Access::All, Vec::new()),
            (_, Allowed::Only(list)) => (Access::Some, list),
            (_, Allowed::Nothing) => (Access::None, Vec::new()),
        };
        out.push(PersonAccess { account_id: a.id, name: a.name, role: a.role.as_str().into(), access, actions, chosen: row.is_some() });
    }
    Ok(out)
}

/// Set what someone may use of my tool.
pub async fn set_person(pool: &SqlitePool, owner_id: &str, tool_id: &str, account_id: &str, access: Access, actions: &[String]) -> CoreResult<()> {
    let t = get_own(pool, owner_id, tool_id).await?;
    if account_id == owner_id {
        return Err(CoreError::Invalid("It's yours: you can always use all of it.".into()));
    }
    crate::accounts::get(pool, account_id).await?;
    let mut list: Vec<String> = actions.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    list.sort();
    list.dedup();
    if list.len() > 500 {
        return Err(CoreError::Invalid("That's too many actions.".into()));
    }
    sqlx::query(
        "INSERT INTO tool_people (tool_id, account_id, access, actions) VALUES (?, ?, ?, ?) \
         ON CONFLICT(tool_id, account_id) DO UPDATE SET access = excluded.access, actions = excluded.actions",
    )
    .bind(&t.id)
    .bind(account_id)
    .bind(access.as_str())
    .bind(serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()))
    .execute(pool)
    .await?;
    Ok(())
}

/// A tool as the apps see it: never its keys or sign-in, only their names and whether signed in.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolView {
    pub id: String,
    pub name: String,
    pub catalog_id: Option<String>,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub url: String,
    /// Names of its kept settings (keys, tokens); their values stay on the server.
    pub secret_names: Vec<String>,
    pub signed_in: bool,
    pub signed_in_as: Option<String>,
    pub enabled: bool,
    pub shared: bool,
    pub restricted_ok: bool,
    pub disabled_tools: Vec<String>,
    /// Actions that ask first, in every chat.
    pub ask_tools: Vec<String>,
    /// Yours (you can change or remove it), else someone's shared one.
    pub mine: bool,
    /// Its owner's name, for a shared one that isn't yours.
    pub owner_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::personas::{self, PersonaInput};

    async fn account(pool: &SqlitePool, name: &str, role: Role) -> String {
        accounts::create(pool, NewAccount { name: name.into(), role, secret: Some("4821".into()) })
            .await
            .unwrap()
            .id
    }

    fn http(name: &str) -> ToolFields {
        ToolFields { name: Some(name.into()), transport: Some("http".into()), url: Some("https://mcp.example.com/mcp".into()), ..Default::default() }
    }

    #[tokio::test]
    async fn tools_are_per_person_and_shared_ones_reach_everyone() {
        let pool = crate::db::open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan", Role::Owner).await;
        let sj = account(&pool, "SJ", Role::Member).await;
        let noah = account(&pool, "Noah", Role::Restricted).await;

        let google = new_tool(&dan, &ToolFields { name: Some("Google".into()), transport: Some("builtin".into()), command: Some("google".into()), ..Default::default() }).unwrap();
        insert(&pool, &google).await.unwrap();
        let mut ha = new_tool(&dan, &http("Home Assistant")).unwrap();
        ha.shared = true;
        insert(&pool, &ha).await.unwrap();
        assert!(new_tool(&dan, &ToolFields { name: Some("x".into()), transport: Some("builtin".into()), command: Some("nope".into()), ..Default::default() }).is_err());

        // SJ sees the shared one only; Noah (restricted) not until it's allowed.
        assert_eq!(usable(&pool, &sj, false).await.unwrap().len(), 1);
        assert!(usable(&pool, &noah, true).await.unwrap().is_empty());
        ha.restricted_ok = true;
        save(&pool, &ha).await.unwrap();
        assert_eq!(usable(&pool, &noah, true).await.unwrap().len(), 1);
        // Only its owner changes it.
        assert!(get_own(&pool, &sj, &ha.id).await.is_err());

        // Chats start from the persona's tools; a shared persona's personal tools aren't others'.
        let p = personas::create(&pool, &dan, PersonaInput { name: Some("Helper".into()), ..Default::default() }).await.unwrap();
        set_persona_tools(&pool, &dan, false, &p.id, &[google.id.clone(), ha.id.clone()]).await.unwrap();
        let chat = crate::chats::create(&pool, &dan, Some(&p.id), None).await.unwrap();
        assert_eq!(chat_tools(&pool, &dan, false, &chat.id).await.unwrap().len(), 2);
        assert_eq!(persona_tools(&pool, &sj, false, &p.id).await.unwrap(), vec![ha.id.clone()]);
        let only_ha = set_chat_tools(&pool, &dan, false, &chat.id, &[ha.id.clone()]).await.unwrap();
        assert_eq!(only_ha, vec![ha.id.clone()]);
        reset_chat_tools(&pool, &dan, &chat.id).await.unwrap();
        assert_eq!(chat_tools(&pool, &dan, false, &chat.id).await.unwrap().len(), 2);
        // SJ can't put Dan's personal tool on her chats.
        let sj_chat = crate::chats::create(&pool, &sj, None, None).await.unwrap();
        assert!(set_chat_tools(&pool, &sj, false, &sj_chat.id, &[google.id.clone()]).await.is_err());

        delete(&pool, &dan, &google.id).await.unwrap();
        assert_eq!(chat_tools(&pool, &dan, false, &chat.id).await.unwrap(), vec![ha.id.clone()]);
    }

    #[tokio::test]
    async fn an_owner_chooses_which_actions_each_person_may_use() {
        let pool = crate::db::open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan", Role::Owner).await;
        let sj = account(&pool, "SJ", Role::Member).await;
        let noah = account(&pool, "Noah", Role::Restricted).await;
        let mut ha = new_tool(&dan, &http("Home Assistant")).unwrap();
        ha.shared = true;
        insert(&pool, &ha).await.unwrap();

        // By default: SJ all of it, Noah none (restricted).
        assert_eq!(allowed(&pool, &ha, &dan, false).await.unwrap(), Allowed::Everything);
        assert_eq!(allowed(&pool, &ha, &sj, false).await.unwrap(), Allowed::Everything);
        assert_eq!(allowed(&pool, &ha, &noah, true).await.unwrap(), Allowed::Nothing);

        // SJ: lights and the AC, not locks.
        let some = vec!["HassTurnOn".to_string(), "HassTurnOff".to_string()];
        set_person(&pool, &dan, &ha.id, &sj, Access::Some, &some).await.unwrap();
        let sj_has = allowed(&pool, &ha, &sj, false).await.unwrap();
        assert!(sj_has.allows("HassTurnOn") && !sj_has.allows("HassUnlock"));
        assert_eq!(usable(&pool, &sj, false).await.unwrap().len(), 1);

        // Noah: given the weather only, though the tool isn't open to restricted accounts.
        set_person(&pool, &dan, &ha.id, &noah, Access::Some, &["GetLiveContext".to_string()]).await.unwrap();
        assert_eq!(usable(&pool, &noah, true).await.unwrap().len(), 1);
        assert!(allowed(&pool, &ha, &noah, true).await.unwrap().allows("GetLiveContext"));

        // Taken away: gone from SJ's tools and chats.
        set_person(&pool, &dan, &ha.id, &sj, Access::None, &[]).await.unwrap();
        assert!(usable(&pool, &sj, false).await.unwrap().is_empty());
        assert!(get_usable(&pool, &sj, false, &ha.id).await.is_err());

        // Only the owner sets it, and the list says who has what.
        assert!(set_person(&pool, &sj, &ha.id, &noah, Access::All, &[]).await.is_err());
        let list = people(&pool, &dan, &ha.id).await.unwrap();
        assert_eq!(list.len(), 2);
        let n = list.iter().find(|p| p.name == "Noah").unwrap();
        assert_eq!((n.access, n.actions.clone(), n.chosen), (Access::Some, vec!["GetLiveContext".to_string()], true));
    }
}
