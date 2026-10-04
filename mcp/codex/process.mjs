import { spawn } from 'node:child_process';
import { lstat, open, readdir, realpath } from 'node:fs/promises';
import path from 'node:path';
import { Refused, requireThat } from '../src/bounds.mjs';
import { createHash } from 'node:crypto';
export async function exactFile(file,cap){
  requireThat(path.isAbsolute(file)&&await realpath(file)===path.normalize(file),'path');let p=path.parse(file).root;
  for(const part of file.slice(p.length).split(path.sep)){p=path.join(p,part);requireThat(!(await lstat(p)).isSymbolicLink(),'redirect');}
  const handle=await open(file,'r');try{const stat=await handle.stat();requireThat(stat.isFile()&&stat.size>0&&stat.size<=cap,'file_limit');const data=Buffer.alloc(stat.size);let at=0;while(at<data.length){const got=await handle.read(data,at,data.length-at,at);requireThat(got.bytesRead>0,'file_changed');at+=got.bytesRead;}const extra=Buffer.alloc(1);requireThat((await handle.read(extra,0,1,at)).bytesRead===0,'file_changed');return data;}finally{await handle.close();}
}
// This helper executes only under vw-codex-host's kill-on-close Windows Job.
// No shell, user argument string, token override or environment credential is used.
export function startChild(executable,args,spawnProcess=spawn){return spawnProcess(executable,args,{windowsHide:true,stdio:['pipe','pipe','ignore'],env:process.env});}
export async function generateSchema(executable,directory,spawnProcess=spawn,timing={run:30_000,cleanup:5000}){
  const child=startChild(executable,['app-server','generate-json-schema','--out',directory],spawnProcess);
  // A failed kill must not leave initialization waiting forever while the Node
  // heartbeat keeps the native Job alive. Refusal after the cleanup deadline
  // exits this owned runtime; its native supervisor then closes the whole Job.
  let failed=false,bytes=0,deadline,cleanup;
  const code=await new Promise((resolve,reject)=>{
    const stop=()=>{if(failed)return;failed=true;try{child.kill();}catch{}
      cleanup=setTimeout(()=>reject(new Refused('schema_cleanup_uncertain')),timing.cleanup);};
    child.once('error',stop);child.stdin.on('error',stop);
    child.once('close',value=>{clearTimeout(deadline);clearTimeout(cleanup);resolve(value);});
    child.stdout.on('data',chunk=>{bytes+=chunk.length;if(bytes>64*1024)stop();});
    deadline=setTimeout(stop,timing.run);
    try{child.stdin.end();}catch{stop();}
  }).finally(()=>{clearTimeout(deadline);clearTimeout(cleanup);});
  requireThat(!failed&&code===0,'schema_generation_failed');
  const todo=[directory];let count=0,total=0;let selected;
  while(todo.length){const base=todo.pop();for(const item of await readdir(base,{withFileTypes:true})){
    requireThat(++count<=2048&&!item.isSymbolicLink(),'schema_inventory');const full=path.join(base,item.name);
    if(item.isDirectory()){requireThat(path.relative(directory,full).split(path.sep).length<=4,'schema_inventory');todo.push(full);}
    else{requireThat(item.isFile()&&item.name.endsWith('.json'),'schema_inventory');const size=(await lstat(full)).size;total+=size;requireThat(total<=64*1024*1024&&size<=4*1024*1024,'schema_inventory');if(item.name==='ClientRequest.json'){requireThat(!selected,'schema_inventory');selected=full;}}
  }}
  requireThat(selected,'unsupported_schema');return exactFile(selected,4*1024*1024);
}
export async function verifyBinary(executable,expected){
  requireThat(/^[0-9a-f]{64}$/.test(expected),'runtime_changed');
  requireThat(path.isAbsolute(executable)&&await realpath(executable)===path.normalize(executable),'path');
  const handle=await open(executable,'r');try{
    const stat=await handle.stat();requireThat(stat.isFile()&&stat.size>0&&stat.size<=256*1024*1024,'file_limit');
    const digest=createHash('sha256'),buffer=Buffer.alloc(65536);let at=0;
    while(true){const got=await handle.read(buffer,0,buffer.length,at);if(!got.bytesRead)break;at+=got.bytesRead;requireThat(at<=stat.size,'runtime_changed');digest.update(buffer.subarray(0,got.bytesRead));}
    requireThat(at===stat.size&&digest.digest('hex')===expected,'runtime_changed');
  }finally{await handle.close();}
}
