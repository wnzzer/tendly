// Entry point: one workspace's saved native sessions, listed by the client's
// own history adapter (see clients.mjs).
import { clientFor } from "./clients.mjs";
import { clean } from "./history-common.mjs";
import { realpath, stat } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { StringDecoder } from "node:string_decoder";

export function transcriptEvents(messages) {
  return messages.flatMap((message,index)=>{
    if(!["user","assistant"].includes(message.role))return [];
    const content=message.content;
    const text=typeof content==="string"?content:Array.isArray(content)?content.filter(block=>["text","input_text","output_text"].includes(block?.type)).map(block=>block.text??"").join("\n"):"";
    return text?[{type:"message",id:`native-${message.id??index}`,role:message.role,text}]:[];
  });
}
export async function discover(job) {
  const cwd=await realpath(job.cwd),configDir=await realpath(job.config_dir);
  if(!(await stat(cwd)).isDirectory()||!(await stat(configDir)).isDirectory())throw Error("History source is not a directory");
  const client=clientFor(job.provider);
  if(!client)throw Error("Unsupported history provider");
  if(job.native_id && !/^[a-zA-Z0-9][a-zA-Z0-9_-]{0,127}$/.test(job.native_id))throw Error("Invalid native session ID");
  const result=await client.history(cwd,configDir,client.configEnv,job.native_id);
  const {messages,...metadata}=result;
  return {...metadata,...(job.native_id?{events:transcriptEvents(messages??[])}:{})};
}
if(process.argv[1] && resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  try {let input="",bytes=0;const decoder=new StringDecoder("utf8");for await(const chunk of process.stdin){bytes+=chunk.length;if(bytes>32768)throw Error("Job too large");input+=decoder.write(chunk);}input+=decoder.end();
    const job=JSON.parse(input);const result=await discover(job);process.stdout.write(JSON.stringify(result));
  }catch(error){process.stdout.write(JSON.stringify({error:clean(error instanceof Error?error.message:"Native history query failed")}));process.exitCode=1;}
}
