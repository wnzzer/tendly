import { LIMIT, normalizeSessions } from "./history-common.mjs";

export const claudeSession=row=>({id:row.sessionId,cwd:row.cwd,time:row.lastModified});
/** @param {string} cwd @param {string} configDir @param {string} configEnv */
export async function claudeHistory(cwd,configDir,configEnv,nativeId) {
  process.env[configEnv]=configDir;
  // listSessions is a metadata-only SDK helper; no query() or tool loop is started.
  const { listSessions, getSessionMessages }=await import("@anthropic-ai/claude-agent-sdk");
  if(nativeId) return {items:[],truncated:false,messages:(await getSessionMessages(nativeId,{dir:cwd})).filter(row=>!row.parent_tool_use_id).map(row=>({.../** @type {any} */(row.message), id:row.uuid, role:row.type}))};
  const rows=await listSessions({dir:cwd,includeWorktrees:false,limit:LIMIT+1});
  return {items:await normalizeSessions(rows.slice(0,LIMIT),"claude_code",cwd,claudeSession),truncated:rows.length>LIMIT};
}
