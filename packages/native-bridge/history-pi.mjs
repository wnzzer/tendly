import { open, readdir, stat, readFile, realpath } from "node:fs/promises";
import { join } from "node:path";
import { LIMIT, normalizeSessions } from "./history-common.mjs";

/**
 * Pi keeps each session as `sessions/--<cwd>--/<time>_<id>.jsonl`, whose first
 * line names the session, its directory and when it began. The first thing
 * the person typed titles it.
 */
const HEAD_BYTES=64*1024;
async function header(file) {
  const handle=await open(file,"r");
  try {
    const {buffer,bytesRead}=await handle.read(Buffer.alloc(HEAD_BYTES),0,HEAD_BYTES,0);
    const lines=buffer.subarray(0,bytesRead).toString("utf8").split("\n").slice(0,-1);
    let session,firstPrompt;
    for(const line of lines){
      let entry;try{entry=JSON.parse(line);}catch{continue;}
      if(entry?.type==="session")session=entry;
      else if(entry?.type==="message"&&entry.message?.role==="user"&&firstPrompt===undefined){
        const content=entry.message.content;
        firstPrompt=typeof content==="string"?content:Array.isArray(content)?content.filter(block=>block?.type==="text").map(block=>block.text??"").join(" "):undefined;
      }
      if(session&&firstPrompt!==undefined)break;
    }
    return session&&{...session,firstPrompt};
  } finally { await handle.close(); }
}
export const piSession=row=>typeof row.cwd!=="string"||!row.cwd?undefined:{id:row.id,cwd:row.cwd,time:row.updatedAt};
/** @param {string} cwd @param {string} configDir */
export async function piHistory(cwd,configDir,_configEnv,nativeId) {
  const root=join(configDir,"sessions");
  let directories;try{directories=await readdir(root,{withFileTypes:true});}catch{return {items:[],truncated:false};}
  const files=[];
  for(const directory of directories.filter(entry=>entry.isDirectory())){
    for(const name of await readdir(join(root,directory.name)).catch(()=>[])){
      if(name.endsWith(".jsonl"))files.push(join(root,directory.name,name));
    }
  }
  const dated=await Promise.all(files.map(async file=>({file,time:(await stat(file).catch(()=>undefined))?.mtimeMs??0})));
  dated.sort((a,b)=>b.time-a.time);
  const rows=[];
  for(const {file,time} of dated.slice(0,LIMIT*4)){
    const session=await header(file).catch(()=>undefined);
    if(session && nativeId && session.id===nativeId && await realpath(session.cwd)===cwd) {
      if((await stat(file)).size>4*1024*1024)throw Error("Native transcript exceeds limit");
      const entries=(await readFile(file,"utf8")).split("\n").filter(Boolean).map(line=>JSON.parse(line));
      // Follow the active leaf's parents rather than showing abandoned branches.
      const byId=new Map(entries.filter(entry=>entry.id).map(entry=>[entry.id,entry]));
      const chain=[];let entry=entries.at(-1);const seen=new Set();
      while(entry&&!seen.has(entry.id)){seen.add(entry.id);chain.push(entry);entry=byId.get(entry.parentId);}
      return {items:[],truncated:false,messages:chain.reverse().filter(entry=>entry.type==="message").map(entry=>({...entry.message,id:entry.id}))};
    }
    if(session)rows.push({...session,updatedAt:time});
  }
  if(nativeId)throw Error("Native session not found");
  const items=await normalizeSessions(rows,"pi",cwd,piSession);
  return {items:items.slice(0,LIMIT),truncated:items.length>LIMIT||dated.length>LIMIT*4};
}
