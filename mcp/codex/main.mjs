import { mkdir, readdir } from 'node:fs/promises';
import path from 'node:path';
import { Refused, requireThat, encoded, json } from '../src/bounds.mjs';
import { Lines, Writer } from '../src/frames.mjs';
import { SchemaGate, exactKeys, imageGate } from './schema.mjs';
import { generateSchema, verifyBinary } from './process.mjs';
import { CodexClient, SendAttempt, TurnReceipts, idText } from './client.mjs';
import { preparePackage } from './package.mjs';
const output=new Writer(process.stdout,4*1024*1024);let gate,client,start,selected,attempt,busy=false,initializing=false,stages=0;
let pinWait,closed=false;const schemaDirectory='schema';const turns=new TurnReceipts();
// Last-resort process deadlines retain child ownership through native Job exit.
const startupDeadline=setTimeout(()=>process.exit(1),80_000);
const send=value=>output.send(value).catch(()=>process.exit(1));
const safeError=error=>error instanceof Refused&&/^[a-z_]{1,64}$/.test(error.code??error.message)?(error.code??error.message):'codex_refused';
async function shutdown(){if(closed)return;closed=true;try{await client?.close();await gate?.close();}finally{process.exit(0);}}
async function pin(stage){requireThat(!pinWait,'pin_busy');await new Promise((resolve,reject)=>{const timer=setTimeout(()=>{pinWait=undefined;reject(new Refused('pin_timeout'));},5000);pinWait={stage,resolve:()=>{clearTimeout(timer);pinWait=undefined;resolve();}};void send({kind:'stage/pin',directory:stage});});}
async function initialize(value){
  exactKeys(value,['kind','executable','binarySha256','work','verifier','profiles']);
  requireThat(value.kind==='start'&&path.isAbsolute(value.executable)&&path.isAbsolute(value.work)&&path.isAbsolute(value.verifier)&&Array.isArray(value.profiles)&&value.profiles.length<=16,'start');
  requireThat((await readdir(value.work)).length===0,'work_not_empty');start=value;
  await verifyBinary(value.executable,value.binarySha256);const directory=path.join(value.work,schemaDirectory);await mkdir(directory);
  const schema=await generateSchema(value.executable,directory);await verifyBinary(value.executable,value.binarySha256);gate=await SchemaGate.open(schema);
  client=await CodexClient.open(value.executable,gate,event=>{
    if(event.kind==='turn/completed'&&!turns.observe(event))return;
    void send(event);
  });
  clearTimeout(startupDeadline);
  await send({kind:'ready',binarySha256:value.binarySha256,schemaSha256:gate.hash,imageProfiles:value.profiles.length});
}
async function action(message){
  requireThat(client&&!closed,'not_ready');idText(message.id);
  switch(message.kind){
    case 'threads': exactKeys(message,['kind','id','cursor']);return client.list(message.cursor);
    case 'select': exactKeys(message,['kind','id','threadId']);requireThat(!turns.active,'turn_active');selected=await client.select(message.threadId);attempt=null;return selected;
    case 'preview': {
      exactKeys(message,['kind','id','directory','manifestSha256']);requireThat(selected&&!turns.active&&stages<2,'preview_limit');
      const profile=start.profiles.find(p=>p.binarySha256===start.binarySha256&&p.schemaSha256===gate.hash&&p.model===selected.model);
      requireThat(profile,'unverified_image_preprocessing');stages++;
      const preview=await preparePackage({directory:message.directory,manifestHash:message.manifestSha256,work:start.work,verifier:start.verifier,profile,binaryHash:start.binarySha256,schemaHash:gate.hash,selected,pin});
      requireThat(await gate.accepts('turn/start',{id:1,method:'turn/start',params:{threadId:selected.threadId,model:preview.model,input:preview.input}}),'unsupported_local_image');
      attempt=new SendAttempt(preview);const {input,...display}=attempt.preview;return {...display,previewId:attempt.id};
    }
    case 'send': {
      exactKeys(message,['kind','id','displayed']);requireThat(attempt&&!turns.active,'no_reviewed_preview');
      // No automatic retry, including a dropped owner response or runtime error.
      const preview=attempt.consume(message.displayed);
      const profile=start.profiles.find(p=>p.binarySha256===start.binarySha256&&p.schemaSha256===gate.hash&&p.model===preview.model);
      imageGate(profile,start.binarySha256,gate.hash,preview.model,preview.images);
      await client.idle(preview.threadId);
      turns.begin(preview.threadId);
      try{
        const result=await client.call('turn/start',{threadId:preview.threadId,model:preview.model,input:preview.input});
        const status=turns.accepted(result?.turn);
        return {attemptId:attempt.id,threadId:preview.threadId,turnId:result.turn.id,status,accepted:true};
      }catch(error){turns.uncertain(preview.threadId);throw error;}

    }
    case 'interrupt': exactKeys(message,['kind','id']);requireThat(turns.active?.turnId,'no_known_turn');await client.call('turn/interrupt',{threadId:turns.active.threadId,turnId:turns.active.turnId});return {requested:true,completed:false};
    default:throw new Refused('unsupported_command');
  }
}
const lines=new Lines(message=>{
  if(message.kind==='shutdown'){void shutdown();return;}
  if(message.kind==='stage/pinned'){if(pinWait?.stage===message.directory)pinWait.resolve();else void shutdown();return;}
  if(!start&&!initializing){initializing=true;void initialize(message).catch(async error=>{await send({kind:'refused',code:safeError(error)});await shutdown();});return;}
  if(busy||!client){void send({kind:'reply',id:message.id,ok:false,code:'busy'});return;}
  busy=true;const actionDeadline=setTimeout(()=>process.exit(1),120_000);void action(message).then(result=>send({kind:'reply',id:message.id,ok:true,result}),error=>send({kind:'reply',id:message.id,ok:false,code:safeError(error)})).finally(()=>{clearTimeout(actionDeadline);busy=false;});
},()=>{void shutdown();},2*1024*1024);
process.stdin.on('data',data=>lines.push(data));process.stdin.on('end',()=>{lines.end();void shutdown();});process.stdin.on('error',()=>{void shutdown();});
setInterval(()=>{void send({kind:'heartbeat'});},5000).unref();
