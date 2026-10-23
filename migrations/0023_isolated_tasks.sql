-- A session Tendly gave a worktree of its own -- an isolated task -- records
-- the branch that work started from: what its changes are measured against
-- and what it is merged back into. Absent for every other session, including
-- one moved by hand onto a worktree that may be shared.
ALTER TABLE sessions ADD COLUMN checkout_base TEXT;
-- How a new worktree of this workspace is prepared, as JSON
-- {"copy":[relative paths],"command":"..."}: files and directories Git does
-- not carry (dependencies, .env) copied from the workspace directory, then a
-- command run in the worktree. Set by the person; absent prepares nothing.
ALTER TABLE workspaces ADD COLUMN worktree_setup TEXT;
