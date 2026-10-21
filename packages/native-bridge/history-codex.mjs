import { realpath } from "node:fs/promises";
import { withoutTendlySecrets } from "./native-spawn.mjs";
import { AppServerClient, AppServerError } from "./app-server.mjs";
import { LIMIT, normalizeSessions } from "./history-common.mjs";

// Codex thread/list always carries cwd. Never label an unscoped/old-client row
// as belonging to this workspace just because it omitted that field.
export const codexSession=row=>typeof row.cwd!=="string"||!row.cwd?undefined:{id:row.id,cwd:row.cwd,time:row.updatedAt*1000};
/** @param {string} cwd @param {string} configDir @param {string} configEnv */
export async function codexHistory(cwd,configDir,configEnv,nativeId) {
  const environment=withoutTendlySecrets(process.env);
  environment[configEnv]=configDir;
  const server=new AppServerClient(process.env.TENDLY_CODEX_BIN||"codex",["app-server"],{cwd,env:environment});
  // A cancelled bridge must not leave its metadata-only app-server behind.
  const cancel=()=>{server.fail("Native history query cancelled");void server.close();};
  process.once("SIGTERM",cancel);process.once("SIGINT",cancel);
  const rpc=async(method,params)=>{
    try{return await server.request(method,params);}
    catch(error){throw error instanceof AppServerError?new Error("Native history API rejected the request; check client version."):error;}
  };
  let timer;
  try {
    return await Promise.race([
      (async()=>{
        await rpc("initialize",{clientInfo:{name:"tendly_history",title:"Tendly history",version:"0.1.0"}});
        server.notify("initialized");
        if(nativeId) {
          const {thread}=await rpc("thread/read",{threadId:nativeId,includeTurns:true});
          if(await realpath(thread.cwd)!==cwd) throw Error("Native session belongs to another workspace");
          return {items:[],truncated:false,messages:(thread.turns??[]).flatMap(turn=>(turn.items??[]).flatMap(item=>item.type==="userMessage"?[{id:item.id,role:"user",content:item.content}]:item.type==="agentMessage"?[{id:item.id,role:"assistant",content:item.text}]:[]))};
        }
        const items=[],seenIds=new Set(),seenCursors=new Set();let cursor=null,pages=0;
        do{
          const page=await rpc("thread/list",{cwd,limit:100,cursor,sortKey:"updated_at",modelProviders:[],sourceKinds:["cli","vscode","exec","appServer","unknown"],useStateDbOnly:true});
          for(const item of await normalizeSessions(Array.isArray(page.data)?page.data:[],"codex",cwd,codexSession)) {
            if(!seenIds.has(item.id)){seenIds.add(item.id);items.push(item);}
          }
          cursor=page.nextCursor??null;
          if(cursor&&seenCursors.has(cursor))throw Error("Native history pagination did not advance");
          if(cursor)seenCursors.add(cursor);
          pages++;
        }while(cursor&&items.length<LIMIT&&pages<10);
        return {items:items.slice(0,LIMIT),truncated:!!cursor||items.length>LIMIT};
      })(),
      new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error("Native history query timed out")),12000);}),
    ]);
  }finally{
    clearTimeout(timer);await server.close();
    process.removeListener("SIGTERM",cancel);process.removeListener("SIGINT",cancel);
  }
}
