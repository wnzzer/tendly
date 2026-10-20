//! Remotes: listing, adding, renaming and removing them, and the checks a
//! remote name or URL must pass.

use super::*;

#[derive(Serialize)]
pub(super) struct Remote {
    pub(super) name: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    push_url: Option<String>,
}

pub(super) async fn list_remotes(root: &std::path::Path) -> Result<Vec<Remote>> {
    let mut remotes: Vec<Remote> = Vec::new();
    for line in git(root, args(&["remote", "-v"]), GIT_TIMEOUT)
        .await?
        .lines()
    {
        let mut parts = line.split_whitespace();
        let (Some(name), Some(url), Some(kind)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        match remotes.iter_mut().find(|remote| remote.name == name) {
            Some(remote) if kind == "(push)" && remote.url != url => {
                remote.push_url = Some(url.to_owned())
            }
            Some(_) => {}
            None => remotes.push(Remote {
                name: name.to_owned(),
                url: url.to_owned(),
                push_url: None,
            }),
        }
    }
    Ok(remotes)
}

pub(super) async fn remotes(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
) -> Result<Json<Vec<Remote>>> {
    Ok(Json(
        list_remotes(&repo(&state, id, &checkout).await?).await?,
    ))
}

pub(super) fn valid_remote_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 100
        && !name.starts_with(['-', '.'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(ApiError::bad("Invalid remote name"))
    }
}

/// Transports that run a command of the URL's choosing are never added.
pub(super) fn valid_url(url: &str) -> Result<()> {
    let lower = url.to_ascii_lowercase();
    if url.is_empty()
        || url.len() > 2048
        || url.starts_with('-')
        || url.chars().any(char::is_control)
        || lower.starts_with("ext::")
        || lower.starts_with("fd::")
    {
        return Err(ApiError::bad("Invalid remote URL"));
    }
    Ok(())
}

/// A remote this repository already has, by name.
pub(super) async fn known_remote(root: &std::path::Path, name: &str) -> Result<String> {
    valid_remote_name(name)?;
    if list_remotes(root)
        .await?
        .iter()
        .any(|remote| remote.name == name)
    {
        Ok(name.to_owned())
    } else {
        Err(ApiError::bad(format!("No remote named {name}")))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RemoteInput {
    name: String,
    #[serde(default)]
    url: Option<String>,
    /// Renaming: the new name.
    #[serde(default)]
    to: Option<String>,
}

pub(super) async fn add_remote(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<RemoteInput>,
) -> Result<Json<Vec<Remote>>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let (name, url) = (
        input.name.trim(),
        input.url.as_deref().unwrap_or_default().trim(),
    );
    valid_remote_name(name)?;
    valid_url(url)?;
    git(&root, args(&["remote", "add", name, url]), GIT_TIMEOUT).await?;
    Ok(Json(list_remotes(&root).await?))
}

pub(super) async fn edit_remote(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<RemoteInput>,
) -> Result<Json<Vec<Remote>>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let mut name = known_remote(&root, input.name.trim()).await?;
    if let Some(to) = input.to.as_deref().map(str::trim).filter(|to| *to != name) {
        valid_remote_name(to)?;
        git(&root, args(&["remote", "rename", &name, to]), GIT_TIMEOUT).await?;
        name = to.to_owned();
    }
    if let Some(url) = input.url.as_deref().map(str::trim) {
        valid_url(url)?;
        git(&root, args(&["remote", "set-url", &name, url]), GIT_TIMEOUT).await?;
    }
    Ok(Json(list_remotes(&root).await?))
}

pub(super) async fn remove_remote(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<RemoteInput>,
) -> Result<Json<Vec<Remote>>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let name = known_remote(&root, input.name.trim()).await?;
    git(&root, args(&["remote", "remove", &name]), GIT_TIMEOUT).await?;
    Ok(Json(list_remotes(&root).await?))
}
