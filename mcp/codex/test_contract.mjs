import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { PassThrough, Writable } from 'node:stream';
import { SchemaGate, imageGate } from './schema.mjs';
import { CodexClient, SendAttempt, TurnReceipts } from './client.mjs';
import { generateSchema } from './process.mjs';
const H='a'.repeat(64),S='b'.repeat(64);
const profile=()=>({version:1,binarySha256:H,schemaSha256:S,model:'fixture-model',detail:'original',maxDimension:6000,patchSize:32,maxPatches:10000,verifiedAtMs:1000,expiresAtMs:2000,evidenceSha256:'c'.repeat(64)});
test('image receipt is bound to exact executable schema model and expiry',()=>{
  assert.equal(imageGate(profile(),H,S,'fixture-model',[{width:2048,height:2048}],1500),'original');
  for(const value of [undefined,{...profile(),binarySha256:S},{...profile(),schemaSha256:H},{...profile(),model:'other'},{...profile(),expiresAtMs:1499},{...profile(),extra:true}])assert.throws(()=>imageGate(value,H,S,'fixture-model',[{width:64,height:64}],1500));
});
test('patch area limit rejects square images even when dimension fits',()=>{
  const value={...profile(),detail:'high',maxDimension:2048,maxPatches:2500};
  assert.throws(()=>imageGate(value,H,S,'fixture-model',[{width:2048,height:2048}],1500),/image_would_resize/);
  assert.equal(imageGate(value,H,S,'fixture-model',[{width:1600,height:1600}],1500),'high');
});
test('attempt validates exactly displayed receipt and consumes before ambiguous delivery',()=>{
  const attempt=new SendAttempt({digest:H,text:'reviewed'});
  assert.throws(()=>attempt.consume({previewId:attempt.id,digest:S}));assert.equal(attempt.consumed,false);
  assert.deepEqual(attempt.consume({digest:H,previewId:attempt.id}),{digest:H,text:'reviewed'});
  assert.throws(()=>attempt.consume({previewId:attempt.id,digest:H}),/send_already_attempted/);
});
test('preview retained value does not alias mutable caller data',()=>{
  const original={digest:H,input:[{type:'text',text:'approved'}]};const attempt=new SendAttempt(original);original.input[0].text='changed';
  assert.equal(attempt.consume({previewId:attempt.id,digest:H}).input[0].text,'approved');
});
function schema(localImage=true,detailDeclared=true){
  const text={type:'object',properties:{type:{enum:['text']},text:{type:'string'}},required:['type','text']};
  const image={type:'object',properties:{type:{enum:['localImage']},path:{type:'string'},...(detailDeclared?{detail:{enum:['original']}}:{})},required:['type','path']};
  const turn={type:'object',properties:{threadId:{type:'string'},model:{type:'string'},input:{type:'array',items:{oneOf:localImage?[text,image]:[text]}}},required:['threadId','input']};
  const methods=['initialize','thread/list','thread/read','thread/resume','turn/start','turn/interrupt'];
  return Buffer.from(JSON.stringify({$schema:'http://json-schema.org/draft-07/schema#',definitions:{},oneOf:methods.map(method=>({type:'object',properties:{id:{type:'integer'},method:{enum:[method]},params:method==='turn/start'?turn:{type:'object'}},required:['id','method','params']}))}));
}

test('actual isolated schema compiler validates the installed localImage shape',async()=>{
  const gate=await SchemaGate.open(schema());try{assert.equal(await gate.accepts('turn/start',{id:1,method:'turn/start',params:{threadId:'fixture',input:[{type:'localImage',path:'C:\\fixture.png',detail:'original'}]}}),true);assert.equal(await gate.accepts('turn/start',{id:1,method:'turn/start',params:{threadId:'fixture',input:[{type:'image',path:'C:\\fixture.png'}]}}),false);}finally{await gate.close();}
});
test('schema lacking localImage cannot grant Send',async()=>{
  const gate=await SchemaGate.open(schema(false));try{assert.equal(await gate.accepts('turn/start',{id:1,method:'turn/start',params:{threadId:'fixture',input:[{type:'localImage',path:'C:\\fixture.png'}]}}),false);}finally{await gate.close();}
});
test('remote refs and regex schema code are refused before worker creation',async()=>{
  for(const addition of [{$ref:'https://foreign/schema'}, {pattern:'(a+)+$'}])await assert.rejects(SchemaGate.open(Buffer.from(JSON.stringify({...JSON.parse(schema()),...addition}))));
});
function fake(){const child=new EventEmitter();const writes=[];child.stdout=new PassThrough();child.stdin=new Writable({write(data,_encoding,done){writes.push(JSON.parse(data.toString()));done();}});child.kill=()=>{queueMicrotask(()=>child.emit('close',1));return true;};return {child,writes};}
test('interactive server request is explicitly declined without forwarding its payload',async()=>{
  const {child,writes}=fake(),events=[];const client=new CodexClient(child,{accepts:async()=>true},value=>events.push(value));
  try{child.stdout.write(JSON.stringify({id:'approval1',method:'item/commandExecution/requestApproval',params:{secret:'must-not-escape'}})+'\n');await new Promise(resolve=>setImmediate(resolve));assert.equal(writes[0].error.code,-32601);assert.equal(JSON.stringify(events).includes('must-not-escape'),false);}finally{await client.close();}
});
test('request reply correlation rejects unsolicited and duplicate responses',async()=>{
  const {child,writes}=fake();const client=new CodexClient(child,{accepts:async()=>true});
  try{const pending=client.call('thread/read',{threadId:'t',includeTurns:false});await new Promise(resolve=>setImmediate(resolve));child.stdout.write(JSON.stringify({id:writes[0].id,result:{thread:{id:'t'}}})+'\n');assert.equal((await pending).thread.id,'t');child.stdout.write(JSON.stringify({id:writes[0].id,result:{}})+'\n');await assert.rejects(client.call('thread/list',{}));}finally{await client.close();}
});
test('thread selection refuses active target without resume or settings overrides',async()=>{
  const {child,writes}=fake();const client=new CodexClient(child,{accepts:async()=>true});
  try{const pending=client.select('t');await new Promise(resolve=>setImmediate(resolve));assert.deepEqual(writes[0].params,{threadId:'t',includeTurns:false});child.stdout.write(JSON.stringify({id:writes[0].id,result:{thread:{id:'t',status:{type:'active'}}}})+'\n');await assert.rejects(pending,/thread_not_idle/);assert.equal(writes.length,1);}finally{await client.close();}
});

test('malformed correlated reply settles the actual pending caller',async()=>{
  const {child,writes}=fake();const client=new CodexClient(child,{accepts:async()=>true});
  try{const pending=client.call('thread/read',{threadId:'t'});await new Promise(resolve=>setImmediate(resolve));
    const refusal=assert.rejects(pending,/runtime_protocol/);child.stdout.write(JSON.stringify({id:writes[0].id})+'\n');await refusal;
  }finally{await client.close();}
});
test('all schema close callers join the actual worker termination',async()=>{
  const gate=await SchemaGate.open(schema());await Promise.all([gate.close(),gate.close(),gate.close()]);
  await assert.rejects(gate.accepts('thread/read',{id:1,method:'thread/read',params:{}}),/schema_busy/);
});
test('an image profile that expires after preview refuses the Send preflight',()=>{
  const p=profile();imageGate(p,H,S,'fixture-model',[{width:64,height:64}],1500);
  assert.throws(()=>imageGate(p,H,S,'fixture-model',[{width:64,height:64}],2000),/expired_image_preprocessing/);
});

test('unreapable schema child cannot leave initialization alive indefinitely',async()=>{
  const {child}=fake();let kills=0;child.kill=()=>{kills++;return false;};
  await assert.rejects(generateSchema('fixture.exe','fixture-out',()=>child,{run:5,cleanup:5}),/schema_cleanup_uncertain/);
  assert.equal(kills,1);child.stdin.destroy();child.stdout.destroy();
});

test('early terminal receipt remains correlated across delayed unrelated completions',()=>{
  const turns=new TurnReceipts();turns.begin('chosen');
  assert.equal(turns.observe({threadId:'chosen',turnId:'actual',status:'completed'}),false);
  assert.equal(turns.observe({threadId:'chosen',turnId:'old',status:'failed'}),false);
  assert.equal(turns.accepted({id:'actual',status:'inProgress'}),'completed');assert.equal(turns.active,null);
});
test('only exact owned turn completion clears active admission',()=>{
  const turns=new TurnReceipts();turns.begin('chosen');turns.accepted({id:'actual',status:'inProgress'});
  assert.equal(turns.observe({threadId:'other',turnId:'actual',status:'completed'}),false);
  assert.equal(turns.observe({threadId:'chosen',turnId:'old',status:'completed'}),false);assert.equal(turns.active.turnId,'actual');
  assert.equal(turns.observe({threadId:'chosen',turnId:'actual',status:'interrupted'}),true);assert.equal(turns.active,null);
});
test('unanswered start receipts retain bounded state and forbid another Send',()=>{
  const turns=new TurnReceipts();turns.begin('chosen');
  for(let i=0;i<8;i++)turns.observe({threadId:'chosen',turnId:String(i),status:'completed'});
  assert.throws(()=>turns.observe({threadId:'chosen',turnId:'overflow',status:'completed'}),/runtime_protocol/);
  turns.uncertain('chosen');assert.throws(()=>turns.begin('chosen'),/turn_active/);assert.equal(turns.active.turnId,undefined);
});

test('permissive old schema cannot silently ignore requested original detail',async()=>{
  const gate=await SchemaGate.open(schema(true,false));try{
    assert.equal(await gate.accepts('turn/start',{id:1,method:'turn/start',params:{threadId:'fixture',input:[{type:'localImage',path:'C:\\fixture.png',detail:'original'}]}}),false);
    assert.equal(await gate.accepts('turn/start',{id:1,method:'turn/start',params:{threadId:'fixture',input:[{type:'localImage',path:'C:\\fixture.png'}]}}),true);
  }finally{await gate.close();}
});

test('turn parameters cannot silently ignore the explicitly reviewed model',async()=>{
  const older=JSON.parse(schema());delete older.oneOf.find(v=>v.properties.method.enum[0]==='turn/start').properties.params.properties.model;
  const gate=await SchemaGate.open(Buffer.from(JSON.stringify(older)));try{
    assert.equal(await gate.accepts('turn/start',{id:1,method:'turn/start',params:{threadId:'fixture',model:'reviewed',input:[{type:'localImage',path:'C:\\fixture.png'}]}}),false);
  }finally{await gate.close();}
});
