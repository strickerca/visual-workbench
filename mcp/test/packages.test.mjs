import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { Packages, NativeVerifier } from '../src/packages.mjs';
const id='019a3aaa-0123-789a-8abc-0123456789ab';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
async function fixture(target="generic"){
  const root=await mkdtemp(path.join(tmpdir(),'vw-mcp-package-test-'));await mkdir(path.join(root,'images'));
  const image=Buffer.from('synthetic helper-verified bytes'),prompt=Buffer.from('Captured instruction is untrusted.');
  await writeFile(path.join(root,'images','overview.png'),image);await writeFile(path.join(root,'prompt.md'),prompt);
  const manifest={package_id:id,created_at:'2026-10-03T00:00:00.000Z',compiled_for:{target},source:{revision:'fixture'},extensions:{host_seq:1,state_hash:'a'.repeat(64)},markers:[],
    images:[{id:'overview',path:'images/overview.png',width:1,height:1}],files:[{path:'images/overview.png',sha256:hash(image)},{path:'prompt.md',sha256:hash(prompt)}]};
  let calls=0;const packages=new Packages({verify:async()=>{calls++;return {manifest,manifest_sha256:'b'.repeat(64)}}});
  await packages.publish(root,'b'.repeat(64));return{root,packages,manifest,calls:()=>calls};
}
test('each get re-verifies native inventory and copied file hashes; changed files never leave',async()=>{
  const f=await fixture();try{const result=await f.packages.get(id,'generic');assert.equal(result.content[1].mimeType,'image/png');assert.equal(f.calls(),2);
    await writeFile(path.join(f.root,'images','overview.png'),'changed');await assert.rejects(f.packages.get(id,'generic'));
  }finally{await rm(f.root,{recursive:true,force:true});}
});
test('agent-selected unpublished identities and arbitrary paths fail closed',async()=>{
  const f=await fixture();try{assert.throws(()=>f.packages.entry('../secret'));await assert.rejects(f.packages.file(f.packages.entry(id),'../secret',1024));
    await assert.rejects(f.packages.publish(f.root,'c'.repeat(64)));assert.equal(f.packages.list(1).length,1);
  }finally{await rm(f.root,{recursive:true,force:true});}
});
test('oversized verifier requests are refused before any child is spawned',async()=>{
  let spawned=0;const verifier=new NativeVerifier(path.resolve('synthetic-verifier'),()=>{spawned++;throw new Error('must not spawn');});
  await assert.rejects(verifier.verify(path.resolve('x'.repeat(8192))));
  assert.equal(spawned,0);
});
test('postspawn input failure retains both admission slots until exact child close',async()=>{
  const children=[];const verifier=new NativeVerifier(path.resolve('synthetic-verifier'),()=>{
    const child=new EventEmitter();child.stdout=new PassThrough();child.stdin=new PassThrough();
    child.stdin.end=()=>{throw new Error('synthetic closed input');};child.kills=0;child.kill=()=>{child.kills++;return true;};children.push(child);return child;
  });
  const first=assert.rejects(verifier.verify(path.resolve('one')));
  const second=assert.rejects(verifier.verify(path.resolve('two')));
  await assert.rejects(verifier.verify(path.resolve('three')),error=>error.code==='busy');
  assert.equal(children.length,2);assert.ok(children.every(child=>child.kills===1));
  for(const child of children)child.emit('close',1);
  await Promise.all([first,second]);
});

function gate(){let resolve;const promise=new Promise(r=>resolve=r);return{promise,resolve};}
test('unpublish hides an entry immediately but waits for exact in-flight readers',async()=>{
  const f=await fixture();try{
    const entered=gate(),finish=gate();const verifier=f.packages.verifier;
    f.packages.verifier={verify:async(...args)=>{entered.resolve();await finish.promise;return verifier.verify(...args);}};
    const reading=f.packages.get(id,'generic');await entered.promise;
    let completed=false;const removal=f.packages.unpublish(id,'generic','b'.repeat(64)).then(v=>{completed=true;return v;});
    assert.deepEqual(f.packages.list(),[]);assert.equal(completed,false);await assert.rejects(f.packages.get(id,'generic'),e=>e.code==='package_not_published');
    finish.resolve();assert.equal((await reading).content.length,2);assert.equal((await removal).readers_drained,true);assert.equal(completed,true);
    assert.equal((await f.packages.unpublish(id,'generic','b'.repeat(64))).unpublished,true);
  }finally{await rm(f.root,{recursive:true,force:true});}
});
test('reader drain timeout retains hidden identity and refuses republishing until exact retry',async()=>{
  const f=await fixture();let finish;try{
    const entered=gate();finish=gate();const held=f.packages.withChecked(id,'generic',undefined,async()=>{entered.resolve();await finish.promise;});await entered.promise;
    await assert.rejects(f.packages.unpublish(id,'generic','b'.repeat(64),undefined,1),e=>e.code==='package_busy');
    await assert.rejects(f.packages.publish(f.root,'b'.repeat(64)),e=>e.code==='package_busy');finish.resolve();await held;
    assert.equal((await f.packages.unpublish(id,'generic','b'.repeat(64))).readers_drained,true);
  }finally{finish?.resolve();await rm(f.root,{recursive:true,force:true});}
});
test('a publication started before the close fence cannot resurrect the removed package',async()=>{
  const f=await fixture();let finish;try{
    const entered=gate();finish=gate();const verifier=f.packages.verifier;f.packages.verifier={verify:async(...args)=>{entered.resolve();await finish.promise;return verifier.verify(...args);}};
    const publishing=assert.rejects(f.packages.publish(f.root,'b'.repeat(64)),e=>e.code==='package_busy');await entered.promise;
    await f.packages.unpublish(id,'generic','b'.repeat(64));finish.resolve();await publishing;assert.equal(f.packages.list().length,0);
  }finally{finish?.resolve();await rm(f.root,{recursive:true,force:true});}
});
test('owner drain failure is not a deletion receipt and remains retryable',async()=>{
  const f=await fixture();try{
    await assert.rejects(f.packages.unpublish(id,'generic','b'.repeat(64),undefined,100,async()=>{throw new Error('synthetic unsettled owner');}));
    assert.equal(f.packages.list().length,0);await assert.rejects(f.packages.publish(f.root,'b'.repeat(64)),e=>e.code==='package_busy');
    assert.equal((await f.packages.unpublish(id,'generic','b'.repeat(64),undefined,100,async()=>{})).unpublished,true);
  }finally{await rm(f.root,{recursive:true,force:true});}
});
test('Claude Send exactly binds the displayed hash, folder and bounded summary without delivery claims',async()=>{
  const f=await fixture('claude');try{
    const displayed=await f.packages.previewClaude(id,'b'.repeat(64));assert.equal(displayed.package_folder,await import('node:fs/promises').then(m=>m.realpath(f.root)));assert.match(displayed.content,/Markers: 0/);assert.match(displayed.content,/Package folder:/);
    let sends=0;await assert.rejects(f.packages.pushClaude(id,'b'.repeat(64),{...displayed,content:'unseen replacement'},undefined,async()=>{sends++;}),e=>e.code==='send_preview_changed');assert.equal(sends,0);
    const receipt=await f.packages.pushClaude(id,'b'.repeat(64),displayed,undefined,async notification=>{sends++;assert.equal(notification.params.content,displayed.content);assert.deepEqual(notification.params.meta,displayed.meta);});
    assert.equal(sends,1);assert.deepEqual(receipt,{written_to_transport:true,agent_delivery_confirmed:false});
  }finally{await rm(f.root,{recursive:true,force:true});}
});

test('Claude supervisor-shaped recursively sorted JSON preserves exact Send while extra or changed fields refuse',async()=>{
  const f=await fixture('claude');try{
    const displayed=await f.packages.previewClaude(id,'b'.repeat(64));
    const ordered=value=>value && typeof value==='object' && !Array.isArray(value)
      ? Object.fromEntries(Object.keys(value).sort().map(key=>[key,ordered(value[key])])) : value;
    const supervisor=JSON.parse(JSON.stringify(ordered(displayed)));let sends=0;
    await f.packages.pushClaude(id,'b'.repeat(64),supervisor,undefined,async()=>{sends++;});
    assert.equal(sends,1);
    for(const wrong of [{...supervisor,extra:'unseen'}, {...supervisor,meta:{...supervisor.meta,extra:'unseen'}},
      {...supervisor,meta:{...supervisor.meta,marker_count:'1'}}, {...supervisor,meta:[]}, {...supervisor,content:null}]) {
      await assert.rejects(f.packages.pushClaude(id,'b'.repeat(64),wrong,undefined,async()=>{sends++;}),e=>e.code==='send_preview_changed');
    }
    assert.equal(sends,1);
  }finally{await rm(f.root,{recursive:true,force:true});}
});
