import { test } from "node:test";
import assert from "node:assert/strict";
import { branchesByWorkspace, bumped, gitFailureMatters } from "./workspace-git.ts";
import { ApiConnectionError, ApiError } from "./api.ts";

test("a reload counter moves on for one workspace, in a new record", () => {
  const counter = { a: 2 };
  assert.deepEqual(bumped(counter, "a"), { a: 3 });
  assert.deepEqual(bumped(counter, "b"), { a: 2, b: 1 });
  assert.deepEqual(counter, { a: 2 }, "the record passed in is left as it was");
});

test("each workspace on a branch is named with it, and a detached one is left out", () => {
  assert.deepEqual(branchesByWorkspace({ a: { branch: "main", files: [] }, b: { branch: null, files: [] } }), {
    a: "main",
  });
});

test("a failed status read is reported only when the host is unreachable or the sign-in lapsed", () => {
  assert.equal(gitFailureMatters(new ApiConnectionError("GET")), true);
  assert.equal(gitFailureMatters(new ApiError("signed out", 401)), true);
  assert.equal(gitFailureMatters(new ApiError("not a repository", 400)), false);
  assert.equal(gitFailureMatters(new Error("x")), false);
});
