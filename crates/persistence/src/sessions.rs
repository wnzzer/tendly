//! Sessions: creating them with their configuration snapshot, and the
//! lifecycle, title, archive, environment and agent-tree changes after.

use super::*;

impl Store {
    pub fn create_session(
        &self,
        workspace_id: WorkspaceId,
        provider: ProviderKind,
        title: &str,
    ) -> Result<Session> {
        self.create_session_with_profile(workspace_id, provider, title, None)
    }

    pub fn create_session_with_profile(
        &self,
        workspace_id: WorkspaceId,
        provider: ProviderKind,
        title: &str,
        endpoint_profile_id: Option<Uuid>,
    ) -> Result<Session> {
        self.create_session_with_model(workspace_id, provider, title, endpoint_profile_id, None)
    }

    pub fn create_session_with_model(
        &self,
        workspace_id: WorkspaceId,
        provider: ProviderKind,
        title: &str,
        endpoint_profile_id: Option<Uuid>,
        model: Option<String>,
    ) -> Result<Session> {
        self.create_session_with_environment(
            workspace_id,
            provider,
            title,
            endpoint_profile_id,
            model,
            EnvironmentOverrides::new(),
        )
    }
    /// Discard a temporary session outright. Refusing anything else keeps the
    /// destructive path reachable only for sessions created as throwaway.
    /// Remove a session record. Only a temporary session, or a permanent one
    /// that has already been archived, can go: deleting is never one step away
    /// from a session still listed in its workspace.
    pub fn delete_session(&self, id: SessionId) -> Result<bool> {
        Ok(self.connection.lock().expect("sqlite lock").execute(
            "DELETE FROM sessions WHERE id=?1 AND (ephemeral=1 OR archived_at IS NOT NULL)",
            [id.to_string()],
        )? > 0)
    }
    /// Update only the branch label of a session's checkout; the path it is
    /// bound to, and its native context, are unchanged.
    pub fn set_session_checkout_branch(&self, id: SessionId, branch: Option<&str>) -> Result<()> {
        self.connection.lock().expect("sqlite lock").execute(
            "UPDATE sessions SET checkout_branch=?1 WHERE id=?2 AND checkout_path IS NOT NULL",
            params![branch, id.to_string()],
        )?;
        Ok(())
    }
    /// Promote a temporary session to a permanent one. One-way on purpose.
    pub fn keep_session(&self, id: SessionId) -> Result<Option<Session>> {
        let mut conn = self.connection.lock().expect("sqlite lock");
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE sessions SET ephemeral=0,updated_at=?1 WHERE id=?2 AND ephemeral=1",
            params![Utc::now().to_rfc3339(), id.to_string()],
        )?;
        let session = tx
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id=?1"),
                [id.to_string()],
                session_row,
            )
            .optional()?;
        tx.commit()?;
        Ok(session)
    }

    pub fn create_session_with_environment(
        &self,
        workspace_id: WorkspaceId,
        provider: ProviderKind,
        title: &str,
        endpoint_profile_id: Option<Uuid>,
        model: Option<String>,
        overrides: EnvironmentOverrides,
    ) -> Result<Session> {
        self.create_session_with_configuration(
            workspace_id,
            provider,
            title,
            endpoint_profile_id,
            model,
            None,
            overrides,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_session_with_configuration(
        &self,
        workspace_id: WorkspaceId,
        provider: ProviderKind,
        title: &str,
        endpoint_profile_id: Option<Uuid>,
        model: Option<String>,
        effort: Option<String>,
        overrides: EnvironmentOverrides,
        ephemeral: bool,
    ) -> Result<Session> {
        self.create_session_with_options(
            workspace_id,
            provider,
            title,
            endpoint_profile_id,
            model,
            effort,
            overrides,
            ephemeral,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_session_with_options(
        &self,
        workspace_id: WorkspaceId,
        provider: ProviderKind,
        title: &str,
        endpoint_profile_id: Option<Uuid>,
        model: Option<String>,
        effort: Option<String>,
        overrides: EnvironmentOverrides,
        ephemeral: bool,
        resume_source_id: Option<SessionId>,
        provider_session_id: Option<String>,
    ) -> Result<Session> {
        tendly_domain::validate_environment(&overrides)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let mut conn = self.connection.lock().expect("sqlite lock");
        let tx = conn.transaction()?;
        let mut snapshot = if let Some(id) = endpoint_profile_id {
            let profile = tx.query_row(
                &format!("SELECT {PROFILE_COLUMNS} FROM endpoint_profiles WHERE id=?1"),
                [id.to_string()],
                profile_row,
            )?;
            // A session-level model/effort override is a launch flag, so it is
            // allowed even for a shared native configuration: it selects what
            // this session runs and never rewrites the client's own settings.
            if profile.provider != provider {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Some(profile)
        } else {
            None
        };
        let now = Utc::now();
        if model.is_some() || effort.is_some() {
            let profile = snapshot.get_or_insert_with(|| EndpointProfile {
                id: Uuid::new_v4(),
                name: "Native session".into(),
                provider,
                endpoint_url: None,
                model: None,
                permission_mode: "native".into(),
                secret_ref: None,
                proxy_url: None,
                effort: None,
                model_aliases: Default::default(),
                models: Vec::new(),
                api: None,
                key_header: None,
                native_config: None,
                environment: Default::default(),
                created_at: now,
            });
            if let Some(model) = model {
                profile.model = Some(model);
            }
            if let Some(effort) = effort {
                profile.effort = Some(effort);
            }
        }
        let mut environment = snapshot
            .as_ref()
            .map(|p| p.environment.clone())
            .unwrap_or_default();
        environment.extend(overrides);
        tendly_domain::validate_environment(&environment)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let environment_json = serde_json::to_string(&environment).map_err(conversion_error)?;
        let session = Session {
            interaction_mode: Default::default(),
            configuration_revision: 0,
            environment,
            id: Uuid::new_v4(),
            workspace_id,
            provider,
            title: title.into(),
            status: SessionStatus::Stopped,
            created_at: now,
            updated_at: now,
            archived_at: None,
            endpoint_profile_id,
            provider_session_id: provider_session_id.clone(),
            error: None,
            endpoint_snapshot: snapshot,
            native_source_id: None,
            native_config_dir: None,
            resume_source_id,
            resume_configuration_revision: None,
            checkout_path: None,
            checkout_branch: None,
            checkout_base: None,
            preview: None,
            ephemeral,
        };
        let snapshot_json = session
            .endpoint_snapshot
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(conversion_error)?;
        // A created session starts with a placeholder name, so the first message
        // may replace it. An imported native session keeps the default 'manual':
        // its name came from the client's own history, not from this column.
        tx.execute("INSERT INTO sessions (id,workspace_id,provider,title,status,created_at,updated_at,endpoint_profile_id,endpoint_snapshot,environment,ephemeral,title_source,resume_source_id,provider_session_id) VALUES (?1,?2,?3,?4,?5,?6,?6,?7,?8,?9,?10,'auto',?11,?12)",
            params![session.id.to_string(),workspace_id.to_string(),provider_name(&session.provider),session.title,"stopped",now.to_rfc3339(),endpoint_profile_id.map(|v|v.to_string()),snapshot_json,environment_json,ephemeral,resume_source_id.map(|v|v.to_string()),provider_session_id])?;
        tx.commit()?;
        Ok(session)
    }

    /// Persist a validated terminal continuation without re-reading a mutable
    /// endpoint template. All launch settings are copied from the source session.
    pub fn insert_terminal_reopen(&self, session: &Session) -> Result<()> {
        if session.resume_source_id.is_none()
            || session.resume_configuration_revision.is_none()
            || session.provider == ProviderKind::Terminal
            || !session.ephemeral
            || session.provider_session_id.is_none()
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tendly_domain::validate_environment(&session.environment)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let snapshot = session
            .endpoint_snapshot
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(conversion_error)?;
        let environment = serde_json::to_string(&session.environment).map_err(conversion_error)?;
        self.connection.lock().expect("sqlite lock").execute(
            "INSERT INTO sessions (id,workspace_id,provider,title,status,created_at,updated_at,endpoint_profile_id,endpoint_snapshot,environment,ephemeral,title_source,resume_source_id,provider_session_id,resume_configuration_revision,native_source_id,native_config_dir,checkout_path,checkout_branch) VALUES (?1,?2,?3,?4,'stopped',?5,?5,?6,?7,?8,1,'manual',?9,?10,?11,?12,?13,?14,?15)",
            params![session.id.to_string(), session.workspace_id.to_string(), provider_name(&session.provider), session.title,
                session.created_at.to_rfc3339(), session.endpoint_profile_id.map(|id| id.to_string()), snapshot, environment,
                session.resume_source_id.map(|id| id.to_string()), session.provider_session_id, session.resume_configuration_revision,
                session.native_source_id, session.native_config_dir, session.checkout_path, session.checkout_branch],
        )?;
        Ok(())
    }

    pub fn list_sessions(&self, workspace_id: Option<WorkspaceId>) -> Result<Vec<Session>> {
        let conn = self.connection.lock().expect("sqlite lock");
        let mut stmt = conn.prepare(&format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE (?1 IS NULL OR workspace_id=?1) ORDER BY created_at DESC,id"))?;
        stmt.query_map([workspace_id.map(|v| v.to_string())], session_row)?
            .collect()
    }

    pub fn get_session(&self, id: SessionId) -> Result<Option<Session>> {
        self.connection
            .lock()
            .expect("sqlite lock")
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id=?1"),
                [id.to_string()],
                session_row,
            )
            .optional()
    }

    /// The stored preferences document, or `null` when none was ever saved.
    /// Record that `parent` started `child` through the agent tools.
    pub fn set_agent_parent(&self, child: SessionId, parent: SessionId) -> Result<()> {
        self.connection.lock().expect("sqlite lock").execute(
            "INSERT OR REPLACE INTO agent_children (session_id,parent_session_id,created_at) VALUES (?1,?2,?3)",
            params![child.to_string(), parent.to_string(), Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }
    /// The session that started `child`, if an agent did.
    pub fn agent_parent(&self, child: SessionId) -> Result<Option<SessionId>> {
        let connection = self.connection.lock().expect("sqlite lock");
        let parent: Option<String> = connection
            .query_row(
                "SELECT parent_session_id FROM agent_children WHERE session_id=?1",
                [child.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(parent.and_then(|value| value.parse().ok()))
    }
    /// The sessions `parent` started, oldest first.
    pub fn agent_children(&self, parent: SessionId) -> Result<Vec<SessionId>> {
        let connection = self.connection.lock().expect("sqlite lock");
        let mut statement = connection.prepare(
            "SELECT session_id FROM agent_children WHERE parent_session_id=?1 ORDER BY created_at",
        )?;
        let ids = statement
            .query_map([parent.to_string()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>>>()?;
        Ok(ids.into_iter().filter_map(|id| id.parse().ok()).collect())
    }

    /// A session's own choice about Tendly's agent tools; `None` follows
    /// the preference.
    pub fn session_agent_tools(&self, id: SessionId) -> Result<Option<bool>> {
        self.connection
            .lock()
            .expect("sqlite lock")
            .query_row(
                "SELECT enabled FROM session_agent_tools WHERE session_id=?1",
                [id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map(|value| value.map(|enabled| enabled != 0))
    }
    /// Set a session's own choice, or `None` to follow the preference again.
    /// False when there is no such session.
    pub fn set_session_agent_tools(&self, id: SessionId, enabled: Option<bool>) -> Result<bool> {
        let connection = self.connection.lock().expect("sqlite lock");
        let exists = connection
            .query_row(
                "SELECT 1 FROM sessions WHERE id=?1",
                [id.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Ok(false);
        }
        match enabled {
            None => connection.execute("DELETE FROM session_agent_tools WHERE session_id=?1", [id.to_string()])?,
            Some(on) => connection.execute(
                "INSERT INTO session_agent_tools (session_id,enabled) VALUES (?1,?2) ON CONFLICT(session_id) DO UPDATE SET enabled=excluded.enabled",
                params![id.to_string(), on as i64],
            )?,
        };
        Ok(true)
    }

    pub fn update_session_title(&self, id: SessionId, title: &str) -> Result<Option<Session>> {
        let mut connection = self.connection.lock().expect("sqlite lock");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            // A name typed by hand is never replaced by a derived one afterwards.
            "UPDATE sessions SET title=?1,title_source='manual',updated_at=?2 WHERE id=?3",
            params![title, Utc::now().to_rfc3339(), id.to_string()],
        )?;
        let session = tx
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id=?1"),
                [id.to_string()],
                session_row,
            )
            .optional()?;
        tx.commit()?;
        Ok(session)
    }

    /// Archive is independent of process status, native resume identity and
    /// conversation history. Repeating the same request preserves timestamps.
    pub fn set_session_archived(&self, id: SessionId, archived: bool) -> Result<Option<Session>> {
        let mut connection = self.connection.lock().expect("sqlite lock");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE sessions SET archived_at=CASE WHEN ?1 THEN ?2 ELSE NULL END,updated_at=?2 WHERE id=?3 AND ((?1 AND archived_at IS NULL) OR (NOT ?1 AND archived_at IS NOT NULL))",
            params![archived, Utc::now().to_rfc3339(), id.to_string()],
        )?;
        let session = tx
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id=?1"),
                [id.to_string()],
                session_row,
            )
            .optional()?;
        tx.commit()?;
        Ok(session)
    }

    pub fn update_session_environment(
        &self,
        id: SessionId,
        environment: &EnvironmentOverrides,
    ) -> Result<bool> {
        tendly_domain::validate_environment(environment)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let encoded = serde_json::to_string(environment).map_err(conversion_error)?;
        Ok(self.connection.lock().expect("sqlite lock").execute(
            "UPDATE sessions SET environment=?1,updated_at=?2 WHERE id=?3 AND status IN ('stopped','failed')",
            params![encoded,Utc::now().to_rfc3339(),id.to_string()],
        )? == 1)
    }

    pub fn set_session_status(&self, id: SessionId, status: SessionStatus) -> Result<bool> {
        self.set_session_result(id, status, None)
    }

    pub fn set_session_result(
        &self,
        id: SessionId,
        status: SessionStatus,
        error: Option<&str>,
    ) -> Result<bool> {
        Ok(self.connection.lock().expect("sqlite lock").execute(
            "UPDATE sessions SET status=?1,error=?2,updated_at=?3 WHERE id=?4",
            params![
                status_name(&status),
                error,
                Utc::now().to_rfc3339(),
                id.to_string()
            ],
        )? == 1)
    }
}
