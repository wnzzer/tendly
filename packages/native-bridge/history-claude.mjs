import { open, readdir } from "node:fs/promises";
import { join } from "node:path";
import { LIMIT, normalizeSessions } from "./history-common.mjs";

export const claudeSession=row=>({id:row.sessionId,cwd:row.cwd,time:row.lastModified});
/** How much of a session's end is read for its model: the last reply is near the end. */
const TAIL=256*1024;
/**
 * The model each session last replied with. The SDK's listing does not say,
 * but every assistant line in the session's file does; the last one that names
 * a real model (not "<synthetic>") is the one a resume continues with.
 * @param {string} configDir @param {string[]} ids @returns {Promise<Map<string,string>>}
 */
export async function claudeModels(configDir,ids) {
  const wanted=new Set(ids),models=new Map();
  let projects=[];
  try {projects=await readdir(join(configDir,"projects"),{withFileTypes:true});} catch {return models;}
  for(const project of projects) {
    if(!project.isDirectory()||!wanted.size)continue;
    let files=[];
    try {files=await readdir(join(configDir,"projects",project.name));} catch {continue;}
    for(const file of files) {
      const id=file.endsWith(".jsonl")?file.slice(0,-6):"";
      if(!wanted.has(id))continue;
      wanted.delete(id);
      const model=await lastModel(join(configDir,"projects",project.name,file));
      if(model)models.set(id,model);
    }
  }
  return models;
}
/** @param {string} path */
export async function lastModel(path) {
  let handle;
  try {
    handle=await open(path,"r");
    const {size}=await handle.stat(),length=Math.min(size,TAIL),buffer=Buffer.alloc(length);
    await handle.read(buffer,0,length,size-length);
    let found;
    for(const match of buffer.toString("utf8").matchAll(/"model":"([^"<\\]{1,128})"/g))found=match[1];
    return found;
  } catch {return undefined;}
  finally {await handle?.close();}
}
/** @param {string} cwd @param {string} configDir @param {string} configEnv */
export async function claudeHistory(cwd,configDir,configEnv,nativeId) {
  process.env[configEnv]=configDir;
  // listSessions is a metadata-only SDK helper; no query() or tool loop is started.
  const { listSessions, getSessionMessages }=await import("@anthropic-ai/claude-agent-sdk");
  if(nativeId) {
    const model=(await claudeModels(configDir,[nativeId])).get(nativeId);
    return {items:[],truncated:false,...(model?{model}:{}),messages:(await getSessionMessages(nativeId,{dir:cwd})).filter(row=>!row.parent_tool_use_id).map(row=>({.../** @type {any} */(row.message), id:row.uuid, role:row.type}))};
  }
  const rows=await listSessions({dir:cwd,includeWorktrees:false,limit:LIMIT+1});
  const items=await normalizeSessions(rows.slice(0,LIMIT),"claude_code",cwd,claudeSession);
  const models=await claudeModels(configDir,items.map(item=>item.id));
  return {items:items.map(item=>models.has(item.id)?{...item,model:models.get(item.id)}:item),truncated:rows.length>LIMIT};
}
