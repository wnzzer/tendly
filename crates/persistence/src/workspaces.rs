//! Registered workspaces.

use super::*;

impl Store {
    pub fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        let conn = self.connection.lock().expect("sqlite lock");
        let mut stmt = conn.prepare(
            "SELECT id,name,root_path,created_at,worktree_setup FROM workspaces ORDER BY created_at DESC,id",
        )?;
        stmt.query_map([], workspace_row)?.collect()
    }

    pub fn get_workspace(&self, id: WorkspaceId) -> Result<Option<Workspace>> {
        self.connection
            .lock()
            .expect("sqlite lock")
            .query_row(
                "SELECT id,name,root_path,created_at,worktree_setup FROM workspaces WHERE id=?1",
                [id.to_string()],
                workspace_row,
            )
            .optional()
    }

    pub fn create_workspace(&self, name: &str, root_path: &str) -> Result<Workspace> {
        let conn = self.connection.lock().expect("sqlite lock");
        // Registering the same directory twice reopens its existing workspace.
        if let Some(existing) = conn
            .query_row(
                "SELECT id,name,root_path,created_at,worktree_setup FROM workspaces WHERE root_path=?1 LIMIT 1",
                [root_path],
                workspace_row,
            )
            .optional()?
        {
            return Ok(existing);
        }
        let workspace = Workspace {
            id: Uuid::new_v4(),
            name: name.into(),
            root_path: root_path.into(),
            created_at: Utc::now(),
            worktree_setup: None,
        };
        conn.execute(
            "INSERT INTO workspaces (id,name,root_path,created_at) VALUES (?1,?2,?3,?4)",
            params![
                workspace.id.to_string(),
                workspace.name,
                workspace.root_path,
                workspace.created_at.to_rfc3339()
            ],
        )?;
        Ok(workspace)
    }

    /// How new worktrees of a workspace are prepared; `None` prepares nothing.
    /// Returns false when there is no such workspace.
    pub fn set_worktree_setup(
        &self,
        id: WorkspaceId,
        setup: Option<&tendly_domain::WorktreeSetup>,
    ) -> Result<bool> {
        let encoded = setup
            .map(serde_json::to_string)
            .transpose()
            .map_err(conversion_error)?;
        Ok(self.connection.lock().expect("sqlite lock").execute(
            "UPDATE workspaces SET worktree_setup=?1 WHERE id=?2",
            params![encoded, id.to_string()],
        )? > 0)
    }
}
