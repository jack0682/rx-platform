//! Opt-in, monotonic namespace fences. No unseal or privileged write bypass.
use super::*;

const TABLE: (&str, &str) = (
    "sealed_prefixes",
    "CREATE TABLE sealed_prefixes (prefix TEXT PRIMARY KEY NOT NULL CHECK(length(prefix)>0)) STRICT",
);
const SEAL_GUARDS: &[(&str, &str)] = &[
    (
        "sealed_prefixes_no_update",
        "CREATE TRIGGER sealed_prefixes_no_update BEFORE UPDATE ON sealed_prefixes BEGIN SELECT RAISE(ABORT, 'namespace seals are permanent'); END",
    ),
    (
        "sealed_prefixes_no_delete",
        "CREATE TRIGGER sealed_prefixes_no_delete BEFORE DELETE ON sealed_prefixes BEGIN SELECT RAISE(ABORT, 'namespace seals are permanent'); END",
    ),
];
fn guard_sql(table: &str, operation: &str) -> (String, String) {
    let trigger = format!("{table}_sealed_{operation}");
    let condition = match operation {
        "insert" => "substr(NEW.key,1,length(prefix))=prefix",
        "delete" => "substr(OLD.key,1,length(prefix))=prefix",
        _ => "substr(NEW.key,1,length(prefix))=prefix OR substr(OLD.key,1,length(prefix))=prefix",
    };
    (
        trigger.clone(),
        format!(
            "CREATE TRIGGER {trigger} BEFORE {} ON {table} WHEN EXISTS (SELECT 1 FROM sealed_prefixes WHERE {condition}) BEGIN SELECT RAISE(ABORT, 'namespace permanently sealed'); END",
            operation.to_ascii_uppercase()
        ),
    )
}
fn schema() -> Vec<(String, String)> {
    let mut definitions = vec![(TABLE.0.into(), TABLE.1.into())];
    definitions.extend(
        SEAL_GUARDS
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string())),
    );
    for table in ["entities", "control_entities"] {
        for operation in ["insert", "update", "delete"] {
            definitions.push(guard_sql(table, operation));
        }
    }
    definitions
}
fn normalized(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn verify(connection: &Connection) -> Result<()> {
    for (name, expected) in schema() {
        let actual: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name=?1",
                [&name],
                |r| r.get(0),
            )
            .optional()
            .map_err(integrity)?;
        if actual.as_deref().map(normalized) != Some(normalized(&expected)) {
            return Err(integrity(format!(
                "namespace fence definition differs: {name}"
            )));
        }
    }
    let mut statement = connection
        .prepare("SELECT prefix FROM sealed_prefixes ORDER BY prefix")
        .map_err(integrity)?;
    let prefixes = statement
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(integrity)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(integrity)?;
    if prefixes.is_empty() {
        return Err(integrity("sealed store has no namespaces"));
    }
    for prefix in prefixes {
        Name::new(&prefix).map_err(integrity)?;
        prefix_range(&prefix)?;
    }
    Ok(())
}

impl SqliteRepository {
    /// Commit the caller's freeze marker and permanent prefix fences atomically.
    /// Only this store is promoted to schema 7. Older schema-6 binaries refuse it.
    /// The callback runs once, before any newly requested fences become active;
    /// existing fences remain effective. Errors/panics roll everything back.
    /// This is a local storage boundary, not remote attestation or process ownership.
    pub fn seal_prefixes<T>(
        &mut self,
        prefixes: &[Name],
        operation: impl FnOnce(&mut dyn rx_ports::Transaction) -> Result<T>,
    ) -> Result<T> {
        if prefixes.is_empty() || prefixes.len() > 16 {
            return Err(StoreError::Invalid("seal requires 1..16 prefixes".into()));
        }
        let mut unique = std::collections::BTreeSet::new();
        for prefix in prefixes {
            prefix_range(prefix.as_str())?;
            if !unique.insert(prefix) {
                return Err(StoreError::Invalid("duplicate sealed prefix".into()));
            }
        }
        let creator = self.ownership.as_ref().expect("open repository").creator();
        let scope = TransactionScope {
            transaction: Some(
                self.connection_mut()?
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(unavailable)?,
            ),
            creator,
        };
        let transaction = scope.transaction.as_ref().expect("live transaction");
        let outcome = operation(&mut SqliteTransaction {
            transaction,
            creator,
        });
        ownership::check_creator(creator)?;
        let value = outcome?;
        let version: i64 = transaction
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(unavailable)?;
        if version == 6 {
            for (_, sql) in schema() {
                transaction.execute_batch(&sql).map_err(unavailable)?;
            }
        } else if version != 7 {
            return Err(integrity("unsupported namespace fence source schema"));
        }
        for prefix in prefixes {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO sealed_prefixes(prefix) VALUES(?1)",
                    [prefix.as_str()],
                )
                .map_err(unavailable)?;
        }
        transaction
            .pragma_update(None, "user_version", 7)
            .map_err(unavailable)?;
        verify(transaction)?;
        scope.commit()?;
        Ok(value)
    }

    /// Read only; does not establish that an exported file came from this store.
    pub fn sealed_prefixes(&self) -> Result<Vec<Name>> {
        let connection = self.connection()?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(unavailable)?;
        if version == 6 {
            return Ok(vec![]);
        }
        if version != 7 {
            return Err(integrity("unsupported seal schema"));
        }
        verify(connection)?;
        let mut statement = connection
            .prepare("SELECT prefix FROM sealed_prefixes ORDER BY prefix")
            .map_err(unavailable)?;
        statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(unavailable)?
            .map(|v| Name::new(v.map_err(unavailable)?).map_err(integrity))
            .collect()
    }
}
