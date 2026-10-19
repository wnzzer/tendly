//! Tendly's SQLite store.
//!
//! One `Store` holds the connection; each group of tables has its own module
//! adding an `impl Store` block: workspaces, sessions, endpoint profiles,
//! layouts and the shared canvas, preferences, imported history and
//! conversation events. `migrations` brings an older database up to the
//! current schema when the store opens, and `rows` turns rows into domain
//! values. Metrics live in a separate database (`metrics`): they are written
//! every minute and are disposable.

mod conversations;
pub use conversations::{ConversationPage, MessageSubmission, SearchHit, StoredConversation};
mod balances;
mod canvas;
pub mod metrics;
mod migrations;
mod native_import;
mod preferences;
mod profiles;
mod rows;
mod sessions;
#[cfg(test)]
mod tests;
mod workspaces;

use rows::*;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Result, TransactionBehavior, params};
use std::{path::Path, sync::Mutex};
use tendly_domain::{
    EndpointProfile, EnvironmentOverrides, NativeConfigReference, ProviderKind, Session, SessionId,
    SessionStatus, Workspace, WorkspaceId,
};
use uuid::Uuid;
pub type DbError = rusqlite::Error;

const SESSION_COLUMNS: &str = "id, workspace_id, provider, title, status, created_at, updated_at, endpoint_profile_id, provider_session_id, error, endpoint_snapshot, native_source_id, native_config_dir, environment, interaction_mode, configuration_revision, archived_at, ephemeral, resume_source_id, checkout_path, checkout_branch, resume_configuration_revision, preview";
const PROFILE_COLUMNS: &str = "id, name, provider, endpoint_url, model, permission_mode, secret_ref, created_at, proxy_url, model_aliases, native_source_id, native_config_dir, native_config_env, environment, effort, models, api, key_header";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedCanvasLayout {
    pub layout: Option<String>,
    pub revision: u64,
}

pub struct Store {
    connection: Mutex<Connection>,
    /// Where each message still streaming wrote its first delta, by session
    /// and message id, so finishing it compacts exactly its own range.
    streaming: Mutex<std::collections::HashMap<(String, String), u64>>,
}
