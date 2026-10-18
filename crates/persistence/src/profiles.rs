//! Endpoint profiles, including those imported from a client's own configuration.

use super::*;

impl Store {
    pub fn list_endpoint_profiles(&self) -> Result<Vec<EndpointProfile>> {
        let conn = self.connection.lock().expect("sqlite lock");
        let mut stmt = conn.prepare(&format!(
            "SELECT {PROFILE_COLUMNS} FROM endpoint_profiles ORDER BY created_at DESC"
        ))?;
        stmt.query_map([], profile_row)?.collect()
    }

    pub fn get_endpoint_profile(&self, id: Uuid) -> Result<Option<EndpointProfile>> {
        self.connection
            .lock()
            .expect("sqlite lock")
            .query_row(
                &format!("SELECT {PROFILE_COLUMNS} FROM endpoint_profiles WHERE id=?1"),
                [id.to_string()],
                profile_row,
            )
            .optional()
    }

    pub fn create_endpoint_profile(&self, p: &EndpointProfile) -> Result<()> {
        validate_native_profile(p)?;
        let aliases = serde_json::to_string(&p.model_aliases).map_err(conversion_error)?;
        let mut conn = self.connection.lock().expect("sqlite lock");
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let first_custom = p.native_config.is_none() && !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM endpoint_profiles WHERE provider=?1 AND native_source_id IS NULL)",
            [provider_name(&p.provider)], |row| row.get::<_, bool>(0),
        )?;
        tx.execute("INSERT INTO endpoint_profiles (id,name,provider,endpoint_url,model,permission_mode,secret_ref,created_at,proxy_url,model_aliases,native_source_id,native_config_dir,native_config_env,environment,effort,models,api) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",params![p.id.to_string(),p.name,provider_name(&p.provider),p.endpoint_url,p.model,p.permission_mode,p.secret_ref,p.created_at.to_rfc3339(),p.proxy_url,aliases,p.native_config.as_ref().map(|native| &native.source_id),p.native_config.as_ref().map(|native| &native.config_dir),p.native_config.as_ref().and_then(|native| native.config_env.as_deref()),serde_json::to_string(&p.environment).map_err(conversion_error)?,p.effort,serde_json::to_string(&p.models).map_err(conversion_error)?,p.api])?;
        if first_custom {
            let stored: Option<String> = tx
                .query_row("SELECT value FROM preferences WHERE id=1", [], |row| {
                    row.get(0)
                })
                .optional()?;
            let mut preferences: serde_json::Value = stored
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(conversion_error)?
                .unwrap_or_else(|| serde_json::json!({}));
            let provider = provider_name(&p.provider);
            if preferences
                .get(provider)
                .and_then(|entry| entry.get("endpoint_profile_id"))
                .is_none_or(serde_json::Value::is_null)
            {
                if preferences
                    .get(provider)
                    .is_none_or(serde_json::Value::is_null)
                {
                    preferences[provider] = serde_json::json!({});
                }
                preferences[provider]["endpoint_profile_id"] = serde_json::json!(p.id);
                tx.execute(
                    "INSERT INTO preferences(id,value,updated_at) VALUES (1,?1,?2) ON CONFLICT(id) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
                    params![preferences.to_string(), chrono::Utc::now().to_rfc3339()],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn update_endpoint_profile(&self, p: &EndpointProfile) -> Result<bool> {
        validate_native_profile(p)?;
        let mut conn = self.connection.lock().expect("sqlite lock");
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(existing) = tx
            .query_row(
                &format!("SELECT {PROFILE_COLUMNS} FROM endpoint_profiles WHERE id=?1"),
                [p.id.to_string()],
                profile_row,
            )
            .optional()?
        else {
            return Ok(false);
        };
        // Native configuration belongs to its source client. A profile rename
        // is allowed; neither retargeting nor injecting overrides is allowed.
        // Ordinary profiles must use the import API to create a native reference.
        if existing.native_config != p.native_config
            || (existing.native_config.is_some() && existing.provider != p.provider)
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let aliases = serde_json::to_string(&p.model_aliases).map_err(conversion_error)?;
        let updated = tx.execute("UPDATE endpoint_profiles SET name=?1,endpoint_url=?2,model=?3,permission_mode=?4,secret_ref=?5,proxy_url=?6,model_aliases=?7,environment=?9,effort=?10,models=?11,api=?12 WHERE id=?8",params![p.name,p.endpoint_url,p.model,p.permission_mode,p.secret_ref,p.proxy_url,aliases,p.id.to_string(),serde_json::to_string(&p.environment).map_err(conversion_error)?,p.effort,serde_json::to_string(&p.models).map_err(conversion_error)?,p.api])? == 1;
        tx.commit()?;
        Ok(updated)
    }

    /// Register a reference only: no source file is opened, copied or modified.
    /// IMMEDIATE serializes the lookup/insert across SQLite connections as well
    /// as this Store's mutex; the unique index is the final identity constraint.
    pub fn import_native_profile(
        &self,
        name: &str,
        provider: ProviderKind,
        reference: NativeConfigReference,
    ) -> Result<EndpointProfile> {
        let profile = EndpointProfile {
            id: Uuid::new_v4(),
            name: name.into(),
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
            native_config: Some(reference),
            environment: Default::default(),
            created_at: Utc::now(),
        };
        validate_native_profile(&profile)?;
        let reference = profile
            .native_config
            .as_ref()
            .expect("native import reference");
        let mut conn = self.connection.lock().expect("sqlite lock");
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = tx.query_row(
            &format!("SELECT {PROFILE_COLUMNS} FROM endpoint_profiles WHERE provider=?1 AND native_source_id=?2 AND native_config_dir=?3 AND native_config_env IS ?4"),
            params![provider_name(&profile.provider),reference.source_id,reference.config_dir,reference.config_env],
            profile_row,
        ).optional()? {
            return Ok(existing);
        }
        tx.execute(
            "INSERT INTO endpoint_profiles (id,name,provider,permission_mode,created_at,model_aliases,native_source_id,native_config_dir,native_config_env) VALUES (?1,?2,?3,'native',?4,'{}',?5,?6,?7)",
            params![profile.id.to_string(),profile.name,provider_name(&profile.provider),profile.created_at.to_rfc3339(),reference.source_id,reference.config_dir,reference.config_env],
        )?;
        tx.commit()?;
        Ok(profile)
    }

    pub fn delete_endpoint_profile(&self, id: Uuid) -> Result<bool> {
        // Sessions retain their immutable profile snapshot even when a template is deleted.
        Ok(self.connection.lock().expect("sqlite lock").execute(
            "DELETE FROM endpoint_profiles WHERE id=?1",
            [id.to_string()],
        )? == 1)
    }
}
