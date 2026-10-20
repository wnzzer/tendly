//! Tags: listing, creating and deleting them, and the names Git allows.

use super::*;

#[derive(Serialize)]
pub(super) struct Tag {
    name: String,
    /// The commit it marks.
    hash: String,
    annotated: bool,
    subject: String,
    date: i64,
}

pub(super) async fn valid_tag(root: &std::path::Path, name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 200 || name.starts_with('-') {
        return Err(ApiError::bad("Invalid tag name"));
    }
    git_text(root, &["check-ref-format", &format!("refs/tags/{name}")])
        .await?
        .map(|_| ())
        .ok_or_else(|| ApiError::bad("Invalid tag name"))
}

async fn list_tags(root: &std::path::Path) -> Result<Vec<Tag>> {
    let listing = git(
        root,
        args(&[
            "for-each-ref",
            "--sort=-creatordate",
            "--format=%(refname:short)%1f%(objecttype)%1f%(objectname)%1f%(*objectname)%1f%(creatordate:unix)%1f%(contents:subject)",
            "refs/tags",
        ]),
        GIT_TIMEOUT,
    )
    .await?;
    Ok(listing
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.splitn(6, '\x1f').collect();
            let [name, kind, object, peeled, date, subject] = fields[..] else {
                return None;
            };
            let annotated = kind == "tag";
            Some(Tag {
                name: name.to_owned(),
                hash: if annotated { peeled } else { object }.to_owned(),
                annotated,
                subject: if annotated { subject } else { "" }.to_owned(),
                date: date.parse().unwrap_or_default(),
            })
        })
        .collect())
}

pub(super) async fn tags(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
) -> Result<Json<Vec<Tag>>> {
    Ok(Json(list_tags(&repo(&state, id, &checkout).await?).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TagInput {
    name: String,
    /// The commit to tag; HEAD when absent.
    #[serde(default)]
    rev: Option<String>,
    /// Makes an annotated tag.
    #[serde(default)]
    message: Option<String>,
}

pub(super) async fn create_tag(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<TagInput>,
) -> Result<Json<Vec<Tag>>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let name = input.name.trim();
    valid_tag(&root, name).await?;
    let hash = valid_rev(&root, input.rev.as_deref().map(str::trim).unwrap_or("HEAD")).await?;
    let mut list = args(&["tag"]);
    if let Some(message) = input
        .message
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        list.extend(["-a".into(), "-m".into(), message.to_owned()]);
    }
    list.extend([name.to_owned(), hash]);
    git(&root, list, COMMIT_TIMEOUT).await?;
    Ok(Json(list_tags(&root).await?))
}

pub(super) async fn delete_tag(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<TagInput>,
) -> Result<Json<Vec<Tag>>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let name = input.name.trim();
    valid_tag(&root, name).await?;
    git(&root, args(&["tag", "-d", name]), GIT_TIMEOUT).await?;
    Ok(Json(list_tags(&root).await?))
}
