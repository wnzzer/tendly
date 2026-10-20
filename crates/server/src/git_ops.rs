//! The rest of Git, beyond the working tree: history, sync with remotes,
//! stashes, tags, merging and the operations that can stop part-way.
//!
//! Every command runs through `workspace_io::run_git` -- no shell, no pager,
//! no editor, bounded output -- and every name or revision a request carries
//! is checked by Git before it is used, never trusted to look harmless.
//! Operations that rewrite files hold the workspace lock like the others;
//! fetch and push only move refs, so they leave it free while the network is
//! slow.
use crate::{
    ApiError, AppState, CheckoutParam, Result, checkout_root,
    workspace_io::{
        self, COMMIT_TIMEOUT, GIT_TIMEOUT, GitDiff, IoError, git_error, git_operation, git_text,
        is_protected_path, run_git, safe_relative, valid_branch, valid_rev,
    },
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use tendly_domain::WorkspaceId;

mod history;
mod merging;
mod remotes;
mod run;
mod stashes;
mod sync;
mod tags;
#[cfg(test)]
mod tests;
mod working_tree;

use history::*;
use merging::*;
use remotes::*;
use run::*;
use stashes::*;
use sync::*;
use tags::*;
use working_tree::*;

pub fn routes() -> Router<AppState> {
    let base = "/api/workspaces/{id}/git";
    let at = |path: &str| format!("{base}/{path}");
    Router::new()
        .route(&at("log"), get(log))
        .route(&at("commit/detail"), get(commit_detail))
        .route(&at("commit/diff"), get(commit_diff))
        .route(&at("remotes"), get(remotes).post(add_remote))
        .route(&at("remotes/edit"), post(edit_remote))
        .route(&at("remotes/remove"), post(remove_remote))
        .route(&at("fetch"), post(fetch))
        .route(&at("pull"), post(pull))
        .route(&at("push"), post(push))
        .route(&at("merge"), post(merge))
        .route(&at("rebase"), post(rebase))
        .route(&at("cherry-pick"), post(cherry_pick))
        .route(&at("revert"), post(revert))
        .route(&at("reset"), post(reset))
        .route(&at("operation"), post(operation))
        .route(&at("resolve"), post(resolve))
        .route(&at("stashes"), get(stashes).post(stash_push))
        .route(&at("stashes/apply"), post(stash_apply))
        .route(&at("stashes/drop"), post(stash_drop))
        .route(&at("tags"), get(tags).post(create_tag))
        .route(&at("tags/delete"), post(delete_tag))
        .route(&at("apply"), post(apply))
        .route(&at("init"), post(init))
        .route(&at("ignore"), post(ignore))
}
