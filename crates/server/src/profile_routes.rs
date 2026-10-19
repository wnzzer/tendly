//! Endpoint profiles, importing one from a client's own configuration, and
//! discovering the models an endpoint offers.

use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateProfile {
    #[serde(default)]
    environment: tendly_domain::EnvironmentOverrides,
    name: String,
    provider: ProviderKind,
    endpoint_url: Option<String>,
    model: Option<String>,
    permission_mode: Option<String>,
    secret_ref: Option<String>,
    proxy_url: Option<String>,
    effort: Option<String>,
    #[serde(default)]
    model_aliases: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    api: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportNativeProfile {
    source_id: String,
    name: Option<String>,
    #[serde(default)]
    confirmed_shared_config: bool,
}

/// A draft profile plus the models to check against it. The profile is not
/// saved: a check is a question about the form's current settings, so it must
/// see exactly what the form shows rather than what was last stored.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProbeModels {
    profile: CreateProfile,
    models: Vec<String>,
}

pub(super) async fn probe_models(
    State(state): State<AppState>,
    Json(input): Json<ProbeModels>,
) -> Result<Json<Vec<model_probe::ProbeResult>>> {
    let p = draft_profile(input.profile, "Model check");
    model_probe::probe(&state.state_dir, &p, &input.models)
        .await
        .map(Json)
}

/// Which protocol a check would speak, without sending one. The form asks this
/// as its fields change, so the API and path are on screen before anything is
/// billed.
pub(super) async fn probe_plan(
    Json(input): Json<CreateProfile>,
) -> Result<Json<model_probe::ProbePlan>> {
    model_probe::plan(&draft_profile(input, "Model check")).map(Json)
}

/// The profile a form's current settings describe, before it is saved. The
/// name only fills a blank one; a draft is never stored under it.
fn draft_profile(input: CreateProfile, fallback_name: &str) -> EndpointProfile {
    EndpointProfile {
        id: Uuid::new_v4(),
        name: if input.name.trim().is_empty() {
            fallback_name.into()
        } else {
            input.name
        },
        provider: input.provider,
        endpoint_url: optional(input.endpoint_url),
        model: optional(input.model),
        permission_mode: input.permission_mode.unwrap_or_else(|| "native".into()),
        secret_ref: optional(input.secret_ref),
        proxy_url: optional(input.proxy_url),
        effort: optional(input.effort),
        model_aliases: input.model_aliases,
        models: input.models,
        api: input.api,
        native_config: None,
        environment: input.environment,
        created_at: chrono::Utc::now(),
    }
}

pub(super) async fn discover_models(
    State(state): State<AppState>,
    Json(input): Json<CreateProfile>,
) -> Result<Json<model_catalog::ModelCatalog>> {
    let p = draft_profile(input, "Model discovery");
    model_catalog::discover(&state, &p).await.map(Json)
}

/// Models a stored profile can run.
///
/// Official accounts are asked through the profile Tendly already holds,
/// never through a native configuration supplied by the caller: the directory a
/// client is pointed at decides which credentials it reads.
pub(super) async fn discover_profile_models(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<model_catalog::ModelCatalog>> {
    let profile = db(&state, move |s| s.get_endpoint_profile(id))
        .await?
        .ok_or_else(|| ApiError::missing("Profile"))?;
    model_catalog::discover_cached(&state, &profile)
        .await
        .map(Json)
}

pub(super) async fn list_profiles(
    State(state): State<AppState>,
) -> Result<Json<Vec<EndpointProfile>>> {
    Ok(Json(db(&state, |s| s.list_endpoint_profiles()).await?))
}
pub(super) async fn native_configuration_sources(
    State(state): State<AppState>,
) -> Result<Json<Vec<native_history::SourceView>>> {
    Ok(Json(
        tokio::task::spawn_blocking(move || native_config::source_views(&state))
            .await
            .map_err(ApiError::internal)?,
    ))
}
pub(super) async fn import_native_profile(
    State(state): State<AppState>,
    Json(input): Json<ImportNativeProfile>,
) -> Result<Json<EndpointProfile>> {
    if !input.confirmed_shared_config {
        return Err(ApiError::bad(
            "Confirm sharing the original client configuration before importing it",
        ));
    }
    let config_state = state.clone();
    let (provider, reference) =
        tokio::task::spawn_blocking(move || native_config::pin(&config_state, &input.source_id))
            .await
            .map_err(ApiError::internal)??;
    if !provider.is_agent() {
        return Err(ApiError::bad("Terminal has no native configuration"));
    }
    let host = format!("Host {} configuration", provider.label());
    let name = bounded_name(input.name.as_deref().unwrap_or(&host))?;
    Ok(Json(
        db(&state, move |s| {
            s.import_native_profile(&name, provider, reference)
        })
        .await?,
    ))
}
pub(super) async fn get_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EndpointProfile>> {
    Ok(Json(
        db(&state, move |s| s.get_endpoint_profile(id))
            .await?
            .ok_or_else(|| ApiError::missing("Profile"))?,
    ))
}
pub(super) async fn create_profile(
    State(state): State<AppState>,
    Json(input): Json<CreateProfile>,
) -> Result<(StatusCode, Json<EndpointProfile>)> {
    let p = EndpointProfile {
        id: Uuid::new_v4(),
        name: bounded_name(&input.name)?,
        provider: input.provider,
        endpoint_url: optional(input.endpoint_url),
        model: optional(input.model),
        permission_mode: input.permission_mode.unwrap_or_else(|| "native".into()),
        secret_ref: optional(input.secret_ref),
        proxy_url: optional(input.proxy_url),
        effort: optional(input.effort),
        model_aliases: input.model_aliases,
        models: input.models,
        api: input.api,
        native_config: None,
        environment: input.environment,
        created_at: chrono::Utc::now(),
    };
    providers::validate_profile(&p)?;
    let save = p.clone();
    db(&state, move |s| s.create_endpoint_profile(&save)).await?;
    Ok((StatusCode::CREATED, Json(p)))
}
pub(super) async fn update_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<Value>,
) -> Result<Json<EndpointProfile>> {
    let mut p = db(&state, move |s| s.get_endpoint_profile(id))
        .await?
        .ok_or_else(|| ApiError::missing("Profile"))?;
    let map = input
        .as_object()
        .ok_or_else(|| ApiError::bad("Expected profile object"))?;
    if p.native_config.is_some() && map.keys().any(|key| key != "name" && key != "environment") {
        return Err(ApiError::bad(
            "An existing native configuration only allows name and session environment edits; original settings stay managed by the native client",
        ));
    }
    for (key, value) in map {
        if key == "environment" {
            p.environment = serde_json::from_value(value.clone())
                .map_err(|_| ApiError::bad("Invalid environment overrides"))?;
            continue;
        }
        if key == "models" {
            p.models = serde_json::from_value(value.clone())
                .map_err(|_| ApiError::bad("Models must be a list of model IDs"))?;
            continue;
        }
        if key == "model_aliases" {
            p.model_aliases = serde_json::from_value(value.clone())
                .map_err(|_| ApiError::bad("Model aliases must be a name-to-ID object"))?;
            continue;
        }
        let text = if value.is_null() {
            None
        } else {
            Some(
                value
                    .as_str()
                    .ok_or_else(|| ApiError::bad("Profile fields must be text or null"))?
                    .to_owned(),
            )
        };
        match key.as_str() {
            "name" => p.name = bounded_name(text.as_deref().unwrap_or(""))?,
            "endpoint_url" => p.endpoint_url = optional(text),
            "model" => p.model = optional(text),
            "secret_ref" => p.secret_ref = optional(text),
            "proxy_url" => p.proxy_url = optional(text),
            "effort" => p.effort = optional(text),
            "api" => p.api = optional(text),
            "permission_mode" => {
                p.permission_mode = text.ok_or_else(|| ApiError::bad("Permission mode required"))?
            }
            "provider" => {
                if text.as_deref() != Some(p.provider.as_str()) {
                    return Err(ApiError::bad("Create a new profile to change provider"));
                }
            }
            _ => return Err(ApiError::bad("Unknown profile field")),
        }
    }
    providers::validate_profile(&p)?;
    let save = p.clone();
    db(&state, move |s| s.update_endpoint_profile(&save)).await?;
    Ok(Json(p))
}
pub(super) async fn delete_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    let profile = db(&state, move |s| s.get_endpoint_profile(id))
        .await?
        .ok_or_else(|| ApiError::missing("Profile"))?;
    if profile
        .native_config
        .as_ref()
        .is_some_and(|reference| reference.source_id.starts_with("account:"))
        || accounts::owns_profile(&state, id)
    {
        return Err(ApiError::conflict(
            "Managed account profiles cannot be unlinked here. Maintain their sign-in from Official accounts; credential files are retained.",
        ));
    }
    if !db(&state, move |s| s.delete_endpoint_profile(id)).await? {
        return Err(ApiError::missing("Profile"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The routes this module serves.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/host/native-configurations",
            get(native_configuration_sources),
        )
        .route(
            "/api/endpoint-profiles/discover-models",
            post(discover_models),
        )
        .route("/api/endpoint-profiles/probe-models", post(probe_models))
        .route("/api/endpoint-profiles/probe-plan", post(probe_plan))
        .route(
            "/api/endpoint-profiles/{id}/models",
            get(discover_profile_models),
        )
        .route(
            "/api/endpoint-profiles/import-native",
            post(import_native_profile),
        )
        .route(
            "/api/endpoint-profiles",
            get(list_profiles).post(create_profile),
        )
        .route(
            "/api/endpoint-profiles/{id}",
            get(get_profile)
                .patch(update_profile)
                .delete(delete_profile),
        )
}
