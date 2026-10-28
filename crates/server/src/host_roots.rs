//! Where on the host Tendly may browse and root a workspace.
//!
//! The environment (or `config.toml`) decides this at start. Folders added
//! from Settings widen it while the gateway runs and are kept in the state
//! directory, so nobody has to edit a variable and restart to add `E:\code`.
//!
//! Adding a folder hands this server more of the disk, so only someone at this
//! machine may do it: a browser on the loopback address, not one reaching it
//! over the network or the tailnet. Anyone else sees the folders, read-only.
//! The routes sit outside `/api/agent/`, so an agent can never add one itself.
use crate::{ApiError, AppState, Result, security, workspace_io};
use axum::{
    Json, Router,
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const FILE: &str = "folders.json";

#[derive(Clone)]
pub struct HostRoots {
    browse: Vec<PathBuf>,
    workspace: Vec<PathBuf>,
    added: Arc<Mutex<Vec<PathBuf>>>,
    /// Where added folders are kept; `None` keeps them in memory (tests).
    file: Option<PathBuf>,
}

impl HostRoots {
    /// The configured roots, and the folders added before, those that are
    /// still there. A folder on a drive that is gone stays in the file and
    /// comes back with the drive.
    pub fn load(state_dir: &Path, browse: Vec<PathBuf>, workspace: Vec<PathBuf>) -> Self {
        let file = state_dir.join(FILE);
        let added = fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<PathBuf>>(&text).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|path| dunce::canonicalize(path).ok().filter(|path| path.is_dir()))
            .collect();
        Self {
            browse,
            workspace,
            added: Arc::new(Mutex::new(added)),
            file: Some(file),
        }
    }
    #[cfg(test)]
    pub fn fixed(browse: Vec<PathBuf>, workspace: Vec<PathBuf>) -> Self {
        Self {
            browse,
            workspace,
            added: Arc::default(),
            file: None,
        }
    }
    /// Where the folder picker may look. Added folders come last, so the
    /// numbers the picker knows the configured ones by do not move.
    pub fn browse(&self) -> Vec<PathBuf> {
        with(&self.browse, &self.added())
    }
    /// Where a workspace, or a worktree for one, may be.
    pub fn workspace(&self) -> Vec<PathBuf> {
        with(&self.workspace, &self.added())
    }
    fn added(&self) -> Vec<PathBuf> {
        self.added.lock().expect("host roots").clone()
    }
    fn view(&self, editable: bool) -> RootsView {
        let text = |paths: &[PathBuf]| {
            paths
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect()
        };
        RootsView {
            configured: text(&with(&self.workspace, &self.browse)),
            added: text(&self.added()),
            editable,
        }
    }
    fn change(&self, edit: impl FnOnce(&mut Vec<PathBuf>)) -> Result<()> {
        let mut added = self.added.lock().expect("host roots");
        let mut next = added.clone();
        edit(&mut next);
        if let Some(file) = &self.file {
            // Every added folder that was ever kept, including any whose drive
            // is away right now, so removing one does not forget the others.
            let mut kept: Vec<PathBuf> = fs::read_to_string(file)
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default();
            kept.retain(|path| !added.contains(path) && !next.contains(path) && !path.exists());
            kept.extend(next.iter().cloned());
            write(file, &kept)?;
        }
        *added = next;
        Ok(())
    }
}

/// `first`, then whatever of `then` it does not already hold.
fn with(first: &[PathBuf], then: &[PathBuf]) -> Vec<PathBuf> {
    let mut all = first.to_vec();
    all.extend(then.iter().filter(|path| !first.contains(path)).cloned());
    all
}

fn write(file: &Path, folders: &[PathBuf]) -> Result<()> {
    let text = serde_json::to_string_pretty(folders).map_err(ApiError::internal)?;
    let temporary = file.with_extension(format!("json.{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut out = fs::File::create(&temporary)?;
        out.write_all(text.as_bytes())?;
        out.sync_all()?;
        fs::rename(&temporary, file)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(ApiError::internal)
}

#[derive(Serialize)]
struct RootsView {
    /// Set at start by the environment or `config.toml`.
    configured: Vec<String>,
    /// Added from Settings.
    added: Vec<String>,
    /// Whether this caller may add or remove folders.
    editable: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderInput {
    path: String,
}

/// A browser on this machine, not one reaching it over the network: the
/// tailnet forwards from loopback too, and says so.
pub(crate) fn from_this_machine(request: &Request) -> bool {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|peer| peer.0.ip().is_loopback())
        && request.extensions().get::<security::ViaTailnet>().is_none()
}

/// The folder `path` names, if it may become one.
fn folder(path: &str) -> Result<PathBuf> {
    let requested = Path::new(path.trim());
    if !requested.is_absolute() {
        return Err(ApiError::bad("Use an absolute host directory path"));
    }
    let folder = dunce::canonicalize(requested)
        .ok()
        .filter(|folder| folder.is_dir())
        .ok_or_else(|| ApiError::bad("Host directory does not exist"))?;
    // A drive is a folder people keep code in on Windows; `/` is everything.
    if folder.parent().is_none() && !cfg!(windows) {
        return Err(ApiError::bad("The root of the file system cannot be added"));
    }
    if workspace_io::is_protected_path(&folder) {
        return Err(ApiError::bad(
            "That folder holds credentials or configuration and cannot be added",
        ));
    }
    Ok(folder)
}

async fn list(State(state): State<AppState>, request: Request) -> Json<RootsView> {
    Json(state.roots.view(from_this_machine(&request)))
}

fn local_only(request: &Request) -> Result<()> {
    if from_this_machine(request) {
        Ok(())
    } else {
        Err(ApiError {
            status: StatusCode::FORBIDDEN,
            message: "Folders can only be added or removed on the machine Tendly runs on".into(),
        })
    }
}

async fn add(State(state): State<AppState>, request: Request) -> Result<Json<RootsView>> {
    local_only(&request)?;
    let input: FolderInput = body(request).await?;
    let folder = folder(&input.path)?;
    state.roots.change(|added| {
        if !added.contains(&folder) {
            added.push(folder);
        }
    })?;
    Ok(Json(state.roots.view(true)))
}

async fn remove(State(state): State<AppState>, request: Request) -> Result<Json<RootsView>> {
    local_only(&request)?;
    let input: FolderInput = body(request).await?;
    let folder = PathBuf::from(input.path);
    state
        .roots
        .change(|added| added.retain(|path| *path != folder))?;
    Ok(Json(state.roots.view(true)))
}

async fn body<T: serde::de::DeserializeOwned>(request: Request) -> Result<T> {
    let bytes = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .map_err(|_| ApiError::bad("Request body is too large"))?;
    serde_json::from_slice(&bytes).map_err(|error| ApiError::bad(error.to_string()))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/host/roots", get(list).post(add).delete(remove))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directories;

    fn temp() -> PathBuf {
        let path = std::env::temp_dir().join(format!("tendly-roots-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        dunce::canonicalize(path).unwrap()
    }

    #[test]
    fn an_added_folder_widens_both_boundaries_and_survives_a_restart() {
        let state = temp();
        let home = temp();
        let code = temp();
        let roots = HostRoots::load(&state, vec![home.clone()], vec![home.clone()]);
        assert!(!directories::within_roots(&roots.workspace(), &code));

        roots.change(|added| added.push(code.clone())).unwrap();
        assert!(directories::within_roots(&roots.workspace(), &code));
        // After the configured ones, so the picker's numbering holds.
        assert_eq!(roots.browse(), [home.clone(), code.clone()]);

        let again = HostRoots::load(&state, vec![home.clone()], vec![home.clone()]);
        assert_eq!(again.workspace(), [home.clone(), code.clone()]);
        again
            .change(|added| added.retain(|path| *path != code))
            .unwrap();
        assert_eq!(
            HostRoots::load(&state, vec![], vec![home.clone()]).workspace(),
            std::slice::from_ref(&home)
        );
        for path in [state, home, code] {
            fs::remove_dir_all(path).ok();
        }
    }

    #[test]
    fn a_folder_that_is_away_is_kept_for_when_it_returns() {
        let state = temp();
        let away = temp();
        let here = temp();
        let roots = HostRoots::load(&state, vec![], vec![]);
        roots
            .change(|added| added.extend([away.clone(), here.clone()]))
            .unwrap();
        fs::remove_dir_all(&away).unwrap();

        let later = HostRoots::load(&state, vec![], vec![]);
        assert_eq!(later.workspace(), std::slice::from_ref(&here));
        later
            .change(|added| added.retain(|path| *path != here))
            .unwrap();
        let kept: Vec<PathBuf> =
            serde_json::from_str(&fs::read_to_string(state.join(FILE)).unwrap()).unwrap();
        assert_eq!(kept, [away]);
        for path in [state, here] {
            fs::remove_dir_all(path).ok();
        }
    }

    #[test]
    fn a_folder_must_exist_and_not_be_everything_or_credentials() {
        let base = temp();
        fs::create_dir_all(base.join(".ssh")).unwrap();
        fs::write(base.join("file"), "x").unwrap();
        assert_eq!(folder(base.to_str().unwrap()).unwrap(), base);
        for refused in ["relative/path", "/no/such/folder/here"] {
            assert!(folder(refused).is_err(), "{refused}");
        }
        assert!(folder(base.join("file").to_str().unwrap()).is_err());
        assert!(folder(base.join(".ssh").to_str().unwrap()).is_err());
        #[cfg(unix)]
        assert!(folder("/").is_err());
        fs::remove_dir_all(base).ok();
    }
}
