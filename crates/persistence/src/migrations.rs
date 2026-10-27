//! Opening the database: bringing any older schema up to the current
//! version in one transaction, and settling what a restart left half-done.

use super::*;

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 23 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let tx = connection.transaction()?;
        tx.execute_batch(include_str!("../../../migrations/0001_metadata.sql"))?;
        if version < 2 {
            tx.execute_batch(include_str!("../../../migrations/0002_endpoint_models.sql"))?;
        }
        // Upgrade the unversioned development schema without dropping any data.
        let names = {
            let mut stmt = tx.prepare("PRAGMA table_info(sessions)")?;
            stmt.query_map([], |r| r.get::<_, String>(1))?
                .collect::<Result<Vec<_>>>()?
        };
        for (name, definition) in [
            ("updated_at", "TEXT NOT NULL DEFAULT ''"),
            ("endpoint_profile_id", "TEXT"),
            ("provider_session_id", "TEXT"),
            ("error", "TEXT"),
            ("endpoint_snapshot", "TEXT"),
        ] {
            if !names.iter().any(|n| n == name) {
                tx.execute(
                    &format!("ALTER TABLE sessions ADD COLUMN {name} {definition}"),
                    [],
                )?;
            }
        }
        tx.execute(
            "UPDATE sessions SET updated_at = created_at WHERE updated_at = ''",
            [],
        )?;
        if version < 3 {
            tx.execute_batch(include_str!("../../../migrations/0003_native_history.sql"))?;
        }
        if version < 4 {
            tx.execute_batch(include_str!("../../../migrations/0004_native_profiles.sql"))?;
        }
        if version < 5 {
            tx.execute_batch(include_str!("../../../migrations/0005_shared_canvas.sql"))?;
        }
        if version < 6 {
            tx.execute_batch(include_str!(
                "../../../migrations/0006_session_environment.sql"
            ))?;
        }
        if version < 7 {
            tx.execute_batch(include_str!("../../../migrations/0007_conversations.sql"))?;
        }
        if version < 8 {
            tx.execute_batch(include_str!("../../../migrations/0008_session_archive.sql"))?;
        }
        if version < 9 {
            tx.execute_batch(include_str!(
                "../../../migrations/0009_reasoning_effort.sql"
            ))?;
        }
        if version < 10 {
            tx.execute_batch(include_str!(
                "../../../migrations/0010_ephemeral_sessions.sql"
            ))?;
        }
        if version < 11 {
            tx.execute_batch(include_str!(
                "../../../migrations/0011_session_title_source.sql"
            ))?;
        }
        if version < 12 {
            tx.execute_batch(include_str!(
                "../../../migrations/0012_session_resume_source.sql"
            ))?;
        }
        if version < 13 {
            tx.execute_batch(include_str!(
                "../../../migrations/0013_session_checkout.sql"
            ))?;
        }
        if version < 14 {
            tx.execute_batch(include_str!("../../../migrations/0014_preferences.sql"))?;
        }
        if version < 15 {
            tx.execute_batch(include_str!("../../../migrations/0015_agent_children.sql"))?;
        }
        if version < 16 {
            tx.execute_batch(include_str!(
                "../../../migrations/0016_session_agent_tools.sql"
            ))?;
        }
        if version < 17 {
            tx.execute_batch(include_str!("../../../migrations/0017_profile_models.sql"))?;
        }
        if version < 18 {
            tx.execute_batch(include_str!("../../../migrations/0018_profile_api.sql"))?;
        }
        if version < 19 {
            tx.execute_batch(include_str!(
                "../../../migrations/0019_resume_configuration.sql"
            ))?;
        }
        if version < 20 {
            tx.execute_batch(include_str!("../../../migrations/0020_session_preview.sql"))?;
        }
        if version < 21 {
            tx.execute_batch(include_str!(
                "../../../migrations/0021_endpoint_balances.sql"
            ))?;
        }
        if version < 22 {
            tx.execute_batch(include_str!(
                "../../../migrations/0022_profile_key_header.sql"
            ))?;
        }
        if version < 23 {
            tx.execute_batch(include_str!("../../../migrations/0023_isolated_tasks.sql"))?;
        }
        // Capture the endpoint settings for legacy M0 sessions once, before templates change.
        let legacy = {
            let mut stmt = tx.prepare("SELECT s.id,p.id FROM sessions s JOIN endpoint_profiles p ON p.id=s.endpoint_profile_id WHERE s.endpoint_snapshot IS NULL")?;
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>>>()?
        };
        for (id, profile_id) in legacy {
            let profile = tx.query_row(
                &format!("SELECT {PROFILE_COLUMNS} FROM endpoint_profiles WHERE id=?1"),
                [profile_id],
                profile_row,
            )?;
            let snapshot = serde_json::to_string(&profile).map_err(conversion_error)?;
            tx.execute(
                "UPDATE sessions SET endpoint_snapshot=?1 WHERE id=?2",
                params![snapshot, id],
            )?;
        }
        tx.pragma_update(None, "user_version", 23)?;
        tx.commit()?;
        Ok(Self {
            connection: Mutex::new(connection),
            streaming: Mutex::new(std::collections::HashMap::new()),
        })
    }

    pub fn reconcile_after_restart(&self) -> Result<()> {
        self.reconcile_after_restart_keeping(&[])
    }

    /// As `reconcile_after_restart`, leaving `kept` running: terminals whose
    /// hosts outlived the server and have been attached to again.
    pub fn reconcile_after_restart_keeping(&self, kept: &[SessionId]) -> Result<()> {
        let mut connection = self.connection.lock().expect("sqlite lock");
        let tx = connection.transaction()?;
        let ids = {
            let mut statement=tx.prepare("SELECT id FROM sessions WHERE interaction_mode='structured' AND status IN ('running','starting','waiting')")?;
            statement
                .query_map([], |r| parse_uuid(r.get(0)?))?
                .collect::<Result<Vec<_>>>()?
        };
        for id in ids {
            conversations::append(&tx, id, serde_json::json!({"type":"exit"}))?;
        }
        let stopped = {
            let mut statement = tx.prepare(
                "SELECT id FROM sessions WHERE status IN ('running','starting','waiting')",
            )?;
            statement
                .query_map([], |r| parse_uuid(r.get(0)?))?
                .collect::<Result<Vec<_>>>()?
        };
        for id in stopped.into_iter().filter(|id| !kept.contains(id)) {
            tx.execute(
                "UPDATE sessions SET status='stopped', error='Server restarted; reconnect history is unavailable. Restart the CLI and use its native resume command.', updated_at=?1 WHERE id=?2",
                params![Utc::now().to_rfc3339(), id.to_string()],
            )?;
        }
        // A temporary session cannot outlive the process it was opened for, so
        // a restart clears it instead of leaving a stopped record behind. That
        // is the clutter the flag exists to avoid.
        tx.execute("DELETE FROM sessions WHERE ephemeral=1", [])?;
        tx.commit()?;
        Ok(())
    }
}
