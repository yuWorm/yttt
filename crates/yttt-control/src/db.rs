//! The control plane's database: PostgreSQL or SQLite, chosen by
//! `database_url`.
//!
//! Queries are written once in SQL both accept: `$N` placeholders, `CAST`
//! rather than `::`, `RETURNING` and `ON CONFLICT`. What differs is spelled
//! through [`Dialect`]: row locks (SQLite takes the whole database for a
//! write transaction instead) and `GREATEST`. Booleans are read through
//! [`Flag`], since SQLite stores them as integers.

use std::time::Duration;

use sqlx::{
    Any, AnyPool, Decode, Transaction, Type,
    any::{AnyPoolOptions, AnyTypeInfo, AnyTypeInfoKind},
    error::BoxDynError,
};

pub(crate) type Pool = AnyPool;
pub(crate) type Tx<'a> = Transaction<'a, Any>;
pub(crate) type Row = sqlx::any::AnyRow;

static POSTGRES: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
static SQLITE: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dialect {
    Postgres,
    Sqlite,
}

impl Dialect {
    fn of_url(url: &str) -> Option<Self> {
        if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            Some(Self::Postgres)
        } else if url.starts_with("sqlite:") {
            Some(Self::Sqlite)
        } else {
            None
        }
    }

    /// The dialect of the connection a transaction runs on.
    pub(crate) fn of(tx: &Tx<'_>) -> Self {
        if (**tx).backend_name() == "SQLite" {
            Self::Sqlite
        } else {
            Self::Postgres
        }
    }

    /// Locks the rows a `SELECT` read for the rest of the transaction; a
    /// SQLite write transaction already holds the whole database.
    pub(crate) fn for_update(self) -> &'static str {
        match self {
            Self::Postgres => " FOR UPDATE",
            Self::Sqlite => "",
        }
    }

    /// The larger of two values.
    pub(crate) fn greatest(self) -> &'static str {
        match self {
            Self::Postgres => "GREATEST",
            Self::Sqlite => "MAX",
        }
    }
}

/// Opens the database `url` names and brings its schema up to date.
pub(crate) async fn open(url: &str, max_connections: u32) -> Result<(Pool, Dialect), sqlx::Error> {
    sqlx::any::install_default_drivers();
    let dialect = Dialect::of_url(url).ok_or_else(|| {
        sqlx::Error::Configuration("database_url must be postgres:// or sqlite://".into())
    })?;
    let options = AnyPoolOptions::new();
    let pool = match dialect {
        Dialect::Postgres => {
            options
                .max_connections(max_connections)
                .connect(url)
                .await?
        }
        Dialect::Sqlite => {
            // A missing file is created; writers queue rather than fail.
            let url = if url.contains('?') {
                url.to_owned()
            } else {
                format!("{url}?mode=rwc")
            };
            // SQLite connections enforce foreign keys and wait out a busy
            // database by default.
            let pool = options
                .max_connections(max_connections.min(8))
                .acquire_timeout(Duration::from_secs(30))
                .connect(&url)
                .await?;
            // Readers do not block the writer; kept in the file once set.
            sqlx::raw_sql("PRAGMA journal_mode = WAL")
                .execute(&pool)
                .await?;
            pool
        }
    };
    match dialect {
        Dialect::Postgres => POSTGRES.run(&pool).await?,
        Dialect::Sqlite => SQLITE.run(&pool).await?,
    }
    Ok((pool, dialect))
}

/// Starts a transaction that may write. On SQLite it takes the write lock at
/// once, so reading before writing cannot fail halfway with a busy database.
pub(crate) async fn begin(pool: &Pool, dialect: Dialect) -> Result<Tx<'static>, sqlx::Error> {
    match dialect {
        Dialect::Postgres => pool.begin().await,
        Dialect::Sqlite => pool.begin_with("BEGIN IMMEDIATE").await,
    }
}

/// `$start, $start+1, …` for `count` values, for an `IN (…)` list.
pub(crate) fn placeholders(start: usize, count: usize) -> String {
    (start..start + count)
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Values bound per `IN (…)` list, well under either database's limit.
pub(crate) const BATCH: usize = 500;

/// A boolean as either database returns it: PostgreSQL's `BOOLEAN`, or a
/// SQLite integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Flag(pub bool);

impl Type<Any> for Flag {
    fn type_info() -> AnyTypeInfo {
        <bool as Type<Any>>::type_info()
    }

    fn compatible(ty: &AnyTypeInfo) -> bool {
        ty.kind() == AnyTypeInfoKind::Bool || ty.kind().is_integer()
    }
}

impl<'r> Decode<'r, Any> for Flag {
    fn decode(value: sqlx::any::AnyValueRef<'r>) -> Result<Self, BoxDynError> {
        match <i64 as Decode<Any>>::decode(value.clone()) {
            Ok(number) => Ok(Self(number != 0)),
            Err(_) => <bool as Decode<Any>>::decode(value).map(Self),
        }
    }
}

impl From<Flag> for bool {
    fn from(flag: Flag) -> Self {
        flag.0
    }
}

/// Reads a boolean column from either database.
pub(crate) fn flag(row: &Row, column: &str) -> bool {
    use sqlx::Row as _;
    row.get::<Flag, _>(column).0
}

/// Reads a nullable boolean column from either database.
pub(crate) fn maybe_flag(row: &Row, column: &str) -> Option<bool> {
    use sqlx::Row as _;
    row.get::<Option<Flag>, _>(column).map(bool::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_opens_with_its_schema_and_reads_booleans() {
        let dir = std::env::temp_dir().join(format!("yttt-db-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let url = format!("sqlite://{}", dir.join("control.db").display());
        let (pool, dialect) = open(&url, 2).await.unwrap();
        assert_eq!(dialect, Dialect::Sqlite);
        let mut tx = begin(&pool, dialect).await.unwrap();
        assert_eq!(Dialect::of(&tx), Dialect::Sqlite);
        let yes: Flag = sqlx::query_scalar("SELECT EXISTS (SELECT 1)")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert!(yes.0);
        tx.rollback().await.unwrap();
        assert_eq!(placeholders(3, 3), "$3, $4, $5");
        pool.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }
}
