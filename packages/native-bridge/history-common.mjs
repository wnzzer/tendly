import { realpath } from "node:fs/promises";
import { resolve } from "node:path";

export const LIMIT=500;
export const clean=(value,max=240)=>typeof value==="string"?value.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g," ").slice(0,max):"";
/**
 * Each client names a session's fields its own way, so `fields` reads one row
 * as {id, cwd, time (epoch ms)}, or undefined for a row that must be skipped,
 * and the model and reasoning effort it last ran with where the client says.
 * A missing cwd means the client's listing was already scoped to `cwd`.
 * @param {any[]} rows @param {string} provider @param {string} cwd
 * @param {(row: any) => { id: any, cwd?: any, time: number, model?: any, effort?: any } | undefined} fields
 */
export async function normalizeSessions(rows,provider,cwd,fields) {
  const result=[];
  for(const row of rows) {
    const session=fields(row);if(!session)continue;
    const id=session.id;
    if(typeof id!=="string"||!/^[a-zA-Z0-9][a-zA-Z0-9_-]{0,127}$/.test(id))continue;
    let directory=cwd;
    if(typeof session.cwd==="string") {try {directory=await realpath(session.cwd);}catch {directory=resolve(session.cwd);}}
    if(directory!==cwd)continue;
    const date=new Date(session.time);if(!Number.isFinite(date.getTime()))continue;
    const model=clean(session.model,128),effort=clean(session.effort,32);
    result.push({id,provider,title:clean(row.customTitle||row.name||row.summary||row.preview||row.firstPrompt)||id,cwd:directory,updated_at:date.toISOString(),...(model?{model}:{}),...(effort?{effort}:{})});
  }
  return result;
}
