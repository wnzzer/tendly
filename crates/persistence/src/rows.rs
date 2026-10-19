//! Turning rows into domain values, and the conversions both ways.

use super::*;

pub(crate) fn conversion_error(
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

pub(crate) fn validate_native_profile(profile: &EndpointProfile) -> Result<()> {
    tendly_domain::validate_environment(&profile.environment)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let Some(reference) = &profile.native_config else {
        return Ok(());
    };
    if profile.provider == ProviderKind::Terminal
        || profile.name.trim().is_empty()
        || reference.source_id.trim().is_empty()
        || reference.config_dir.trim().is_empty()
        || reference.config_env.as_deref() == Some("")
        || profile.endpoint_url.is_some()
        || profile.model.is_some()
        || profile.secret_ref.is_some()
        // A proxy is allowed: it is the route Tendly uses when it launches
        // the client, applied as launch environment. It never rewrites the
        // native configuration, unlike the settings rejected above.
        || !profile.model_aliases.is_empty()
        || !profile.models.is_empty()
        || profile.api.is_some()
        || profile.key_header.is_some()
        || profile.permission_mode != "native"
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}
pub(crate) fn parse_uuid(value: String) -> Result<Uuid> {
    Uuid::parse_str(&value).map_err(conversion_error)
}
pub(crate) fn parse_time(value: String) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|v| v.with_timezone(&Utc))
        .map_err(conversion_error)
}
pub(crate) fn parse_provider(value: String) -> Result<ProviderKind> {
    ProviderKind::parse(&value).ok_or(rusqlite::Error::InvalidQuery)
}
pub(crate) fn provider_name(p: &ProviderKind) -> &'static str {
    p.as_str()
}
pub(crate) fn status_name(s: &SessionStatus) -> &'static str {
    match s {
        SessionStatus::Starting => "starting",
        SessionStatus::Running => "running",
        SessionStatus::Waiting => "waiting",
        SessionStatus::Stopped => "stopped",
        SessionStatus::Failed => "failed",
    }
}
pub(crate) fn workspace_row(r: &rusqlite::Row<'_>) -> Result<Workspace> {
    Ok(Workspace {
        id: parse_uuid(r.get(0)?)?,
        name: r.get(1)?,
        root_path: r.get(2)?,
        created_at: parse_time(r.get(3)?)?,
    })
}
pub(crate) fn profile_row(r: &rusqlite::Row<'_>) -> Result<EndpointProfile> {
    let native_config = match (
        r.get::<_, Option<String>>(10)?,
        r.get::<_, Option<String>>(11)?,
        r.get::<_, Option<String>>(12)?,
    ) {
        (Some(source_id), Some(config_dir), config_env) => Some(NativeConfigReference {
            source_id,
            config_dir,
            config_env,
        }),
        (None, None, None) => None,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(EndpointProfile {
        environment: serde_json::from_str(&r.get::<_, String>(13)?).map_err(conversion_error)?,
        id: parse_uuid(r.get(0)?)?,
        name: r.get(1)?,
        provider: parse_provider(r.get(2)?)?,
        endpoint_url: r.get(3)?,
        model: r.get(4)?,
        permission_mode: r.get(5)?,
        secret_ref: r.get(6)?,
        created_at: parse_time(r.get(7)?)?,
        proxy_url: r.get(8)?,
        effort: r.get(14)?,
        model_aliases: serde_json::from_str(&r.get::<_, String>(9)?).map_err(conversion_error)?,
        models: serde_json::from_str(&r.get::<_, String>(15)?).map_err(conversion_error)?,
        api: r.get(16)?,
        key_header: r
            .get::<_, Option<String>>(17)?
            .map(|text| serde_json::from_str(&text).map_err(conversion_error))
            .transpose()?,
        native_config,
    })
}
pub(crate) fn session_row(r: &rusqlite::Row<'_>) -> Result<Session> {
    let status = match r.get::<_, String>(4)?.as_str() {
        "starting" => SessionStatus::Starting,
        "running" => SessionStatus::Running,
        "waiting" => SessionStatus::Waiting,
        "stopped" => SessionStatus::Stopped,
        "failed" => SessionStatus::Failed,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(Session {
        interaction_mode: match r.get::<_, String>(14)?.as_str() {
            "pty" => tendly_domain::InteractionMode::Pty,
            "structured" => tendly_domain::InteractionMode::Structured,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        configuration_revision: r.get(15)?,
        environment: serde_json::from_str(&r.get::<_, String>(13)?).map_err(conversion_error)?,
        native_source_id: r.get(11)?,
        native_config_dir: r.get(12)?,
        id: parse_uuid(r.get(0)?)?,
        workspace_id: parse_uuid(r.get(1)?)?,
        provider: parse_provider(r.get(2)?)?,
        title: r.get(3)?,
        status,
        created_at: parse_time(r.get(5)?)?,
        updated_at: parse_time(r.get(6)?)?,
        archived_at: r
            .get::<_, Option<String>>(16)?
            .map(parse_time)
            .transpose()?,
        ephemeral: r.get(17)?,
        resume_source_id: r
            .get::<_, Option<String>>(18)?
            .map(parse_uuid)
            .transpose()?,
        checkout_path: r.get(19)?,
        checkout_branch: r.get(20)?,
        resume_configuration_revision: r.get(21)?,
        preview: r
            .get::<_, Option<String>>(22)?
            .as_deref()
            .and_then(conversations::preview_line),
        endpoint_profile_id: r.get::<_, Option<String>>(7)?.map(parse_uuid).transpose()?,
        provider_session_id: r.get(8)?,
        error: r.get(9)?,
        endpoint_snapshot: r
            .get::<_, Option<String>>(10)?
            .map(|s| serde_json::from_str(&s).map_err(conversion_error))
            .transpose()?,
    })
}
