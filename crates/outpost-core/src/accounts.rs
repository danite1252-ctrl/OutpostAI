//! Accounts: one per person (or per service like Hermes).

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::secret::{hash_secret, validate_secret};
use crate::{now, CoreError, CoreResult};

/// What an account may do. Owners manage the Outpost; restricted accounts are for
/// kids or guests; service accounts are for apps like Hermes and only use API keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Member,
    Restricted,
    Service,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Member => "member",
            Role::Restricted => "restricted",
            Role::Service => "service",
        }
    }

    pub fn parse(s: &str) -> CoreResult<Role> {
        match s {
            "owner" => Ok(Role::Owner),
            "member" => Ok(Role::Member),
            "restricted" => Ok(Role::Restricted),
            "service" => Ok(Role::Service),
            other => Err(CoreError::Internal(format!("unknown role in database: {other}"))),
        }
    }
}

/// An account as the API shows it. Never includes the secret hash.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub role: Role,
    /// Hex colour for the avatar, e.g. `#FFB547`.
    pub avatar_color: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct AccountRow {
    pub id: String,
    pub name: String,
    pub role: String,
    pub avatar_color: String,
    pub secret_hash: Option<String>,
    pub failed_attempts: i64,
    pub locked_until: Option<i64>,
    pub created_at: i64,
}

impl TryFrom<AccountRow> for Account {
    type Error = CoreError;
    fn try_from(row: AccountRow) -> CoreResult<Account> {
        Ok(Account {
            id: row.id,
            name: row.name,
            role: Role::parse(&row.role)?,
            avatar_color: row.avatar_color,
            created_at: row.created_at,
        })
    }
}

/// Avatar colours handed out in turn, taken from the Ember palette.
const AVATAR_COLORS: [&str; 6] = ["#FFB547", "#9CB4FF", "#7BD88F", "#FF8A9A", "#C9B8FF", "#6FD3FF"];

pub struct NewAccount {
    pub name: String,
    pub role: Role,
    /// PIN or password. Required for every role except `service`, which must not have one.
    pub secret: Option<String>,
}

const SELECT_ROW: &str =
    "SELECT id, name, role, avatar_color, secret_hash, failed_attempts, locked_until, created_at FROM accounts";

pub async fn count(pool: &SqlitePool) -> CoreResult<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(pool)
        .await?)
}

pub async fn owner_exists(pool: &SqlitePool) -> CoreResult<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE role = 'owner'")
        .fetch_one(pool)
        .await?;
    Ok(n > 0)
}

pub async fn list(pool: &SqlitePool) -> CoreResult<Vec<Account>> {
    let rows: Vec<AccountRow> = sqlx::query_as(&format!("{SELECT_ROW} ORDER BY created_at, name"))
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(Account::try_from).collect()
}

pub async fn get(pool: &SqlitePool, id: &str) -> CoreResult<Account> {
    get_row(pool, id).await?.try_into()
}

pub(crate) async fn get_row(pool: &SqlitePool, id: &str) -> CoreResult<AccountRow> {
    sqlx::query_as(&format!("{SELECT_ROW} WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

fn clean_name(name: &str) -> CoreResult<String> {
    let name = name.trim();
    let len = name.chars().count();
    if len == 0 || len > 40 {
        return Err(CoreError::Invalid("A name must be 1 to 40 characters.".into()));
    }
    if name.chars().any(char::is_control) {
        return Err(CoreError::Invalid("A name cannot contain control characters.".into()));
    }
    Ok(name.to_string())
}

pub async fn create(pool: &SqlitePool, new: NewAccount) -> CoreResult<Account> {
    let name = clean_name(&new.name)?;
    let secret_hash = match (new.role, new.secret) {
        (Role::Service, None) => None,
        (Role::Service, Some(_)) => {
            return Err(CoreError::Invalid(
                "Service accounts sign in with API keys, not a PIN.".into(),
            ))
        }
        (_, None) => return Err(CoreError::Invalid("Every account needs a PIN or password.".into())),
        (_, Some(secret)) => {
            validate_secret(&secret)?;
            Some(hash_blocking(secret).await?)
        }
    };

    let n = count(pool).await?;
    let color = AVATAR_COLORS[(n as usize) % AVATAR_COLORS.len()];
    let account = Account {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        role: new.role,
        avatar_color: color.to_string(),
        created_at: now(),
    };

    let result = sqlx::query(
        "INSERT INTO accounts (id, name, role, avatar_color, secret_hash, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&account.id)
    .bind(&account.name)
    .bind(account.role.as_str())
    .bind(&account.avatar_color)
    .bind(secret_hash)
    .bind(account.created_at)
    .execute(pool)
    .await;

    match result {
        Ok(_) => Ok(account),
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => Err(CoreError::Conflict(format!(
            "An account called \"{}\" already exists.",
            account.name
        ))),
        Err(e) => Err(e.into()),
    }
}

/// Delete an account and its sessions. The last owner cannot be deleted.
pub async fn delete(pool: &SqlitePool, id: &str) -> CoreResult<()> {
    let account = get(pool, id).await?;
    if account.role == Role::Owner {
        let owners: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE role = 'owner'")
            .fetch_one(pool)
            .await?;
        if owners <= 1 {
            return Err(CoreError::Forbidden("Outpost needs at least one owner.".into()));
        }
    }
    sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Replace an account's PIN or password and sign out all of its sessions.
pub async fn set_secret(pool: &SqlitePool, id: &str, secret: String) -> CoreResult<()> {
    let account = get(pool, id).await?;
    if account.role == Role::Service {
        return Err(CoreError::Invalid(
            "Service accounts sign in with API keys, not a PIN.".into(),
        ));
    }
    validate_secret(&secret)?;
    let hash = hash_blocking(secret).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE accounts SET secret_hash = ?, failed_attempts = 0, locked_until = NULL WHERE id = ?")
        .bind(hash)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE sessions SET revoked_at = ? WHERE account_id = ? AND revoked_at IS NULL")
        .bind(now())
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn hash_blocking(secret: String) -> CoreResult<String> {
    tokio::task::spawn_blocking(move || hash_secret(&secret))
        .await
        .map_err(|e| CoreError::Internal(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn owner(name: &str) -> NewAccount {
        NewAccount {
            name: name.into(),
            role: Role::Owner,
            secret: Some("4821".into()),
        }
    }

    #[tokio::test]
    async fn create_list_and_unique_names() {
        let pool = open_in_memory().await.unwrap();
        assert!(!owner_exists(&pool).await.unwrap());
        let dan = create(&pool, owner("Dan")).await.unwrap();
        assert_eq!(dan.role, Role::Owner);
        assert!(owner_exists(&pool).await.unwrap());

        let dup = create(&pool, owner("dan")).await;
        assert!(matches!(dup, Err(CoreError::Conflict(_))));

        let sarah = create(
            &pool,
            NewAccount {
                name: "Sarah".into(),
                role: Role::Member,
                secret: Some("hunter22".into()),
            },
        )
        .await
        .unwrap();
        assert_ne!(dan.avatar_color, sarah.avatar_color);
        assert_eq!(list(&pool).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn secret_rules_per_role() {
        let pool = open_in_memory().await.unwrap();
        let no_pin = create(
            &pool,
            NewAccount {
                name: "Kids".into(),
                role: Role::Restricted,
                secret: None,
            },
        )
        .await;
        assert!(matches!(no_pin, Err(CoreError::Invalid(_))));
        let svc = create(
            &pool,
            NewAccount {
                name: "Hermes".into(),
                role: Role::Service,
                secret: None,
            },
        )
        .await;
        assert!(svc.is_ok());
        let svc_pin = create(
            &pool,
            NewAccount {
                name: "Bot".into(),
                role: Role::Service,
                secret: Some("1234".into()),
            },
        )
        .await;
        assert!(matches!(svc_pin, Err(CoreError::Invalid(_))));
    }

    #[tokio::test]
    async fn last_owner_cannot_be_deleted() {
        let pool = open_in_memory().await.unwrap();
        let dan = create(&pool, owner("Dan")).await.unwrap();
        assert!(matches!(delete(&pool, &dan.id).await, Err(CoreError::Forbidden(_))));
        let second = create(&pool, owner("Sam")).await.unwrap();
        delete(&pool, &dan.id).await.unwrap();
        assert!(matches!(get(&pool, &dan.id).await, Err(CoreError::NotFound)));
        assert!(get(&pool, &second.id).await.is_ok());
    }
}
