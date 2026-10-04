import test from 'node:test';
import assert from 'node:assert/strict';
import { PassThrough, Writable } from 'node:stream';
import { json, identifier, Slots } from '../src/bounds.mjs';
import { Lines, Writer, PipeTransport } from '../src/frames.mjs';
import { Grants } from '../src/grants.mjs';
import { Owner } from '../src/owner.mjs';
test('canonical project UUIDs are accepted; device IDs and unsafe numeric revision are not', () => {
  assert.equal(identifier('019a3aaa-0123-789a-8abc-0123456789ab'),'019a3aaa-0123-789a-8abc-0123456789ab');
  assert.throws(()=>identifier('01ARZ3NDEKTSV4RRFFQ69G5FAV'));
  assert.throws(()=>json(Buffer.from('{"host_seq":9007199254740993}')));
});
test('deep and broad JSON fails before typed allocation; quoted delimiters are data', () => {
  assert.throws(()=>json(Buffer.from('['.repeat(33)+'0'+']'.repeat(33))));
  assert.throws(()=>json(Buffer.from('['+'0,'.repeat(32769)+'0]')));
  assert.deepEqual(json(Buffer.from('{"x":"[\\\"{"}')),{x:'["{'});
});
test('unterminated and truncated frames are bounded and delivered at most once', () => {
  const received=[],failures=[];const lines=new Lines(v=>received.push(v),e=>failures.push(e.code),8);
  lines.push(Buffer.from('{}\n{}\n'));assert.equal(received.length,2);
  lines.push(Buffer.from('123456789'));lines.push(Buffer.from('{}\n'));assert.deepEqual(failures,['frame_too_large']);assert.equal(received.length,2);
  const truncated=new Lines(()=>assert.fail(),e=>failures.push(e.code));truncated.push(Buffer.from('{'));truncated.end();assert.equal(failures.at(-1),'truncated_frame');
});
test('grant expiry, revoke, replacement and connection reuse invalidate in-flight authority', () => {
  let now=100;const grants=new Grants(()=>now);grants.open('a');assert.throws(()=>grants.admit('a','window1'));
  grants.grant('a',['window1'],100);const check=grants.admit('a','window1');assert.throws(()=>grants.admit('a','window2'));
  grants.revoke('a');assert.equal(check.signal.aborted,true);assert.throws(check);
  grants.grant('a',['window1'],100);const expires=grants.admit('a','window1');now=201;assert.throws(expires);
  grants.close('a');grants.open('a');assert.throws(()=>grants.admit('a','window1'));
});
test('worker capacity follows actual completion after caller abandons result', async () => {
  const slots=new Slots(1);let finish;const work=slots.run(()=>new Promise(resolve=>finish=resolve));
  await assert.rejects(slots.run(async()=>{}));assert.equal(slots.active,1);finish();await work;assert.equal(slots.active,0);
});
test('owner cancellation cannot publish a late response as success', async () => {
  const messages=[];const owner=new Owner({send:async v=>messages.push(v)});const cancel=new AbortController();
  const pending=owner.call('capture',{session:'a',selector:'window1'},cancel.signal);cancel.abort();await assert.rejects(pending);
  owner.reply({id:messages[0].id,ok:true,result:{lossless:true}});assert.equal(messages[1].kind,'owner/cancel');owner.close();
});
test('pipe transport enforces request slots and exact terminal retirement', async () => {
  const messages=[];const transport=new PipeTransport('a',{send:async v=>messages.push(v)},()=>{});transport.onmessage=()=>{};
  for(let id=0;id<8;id++)transport.deliver({jsonrpc:'2.0',id,method:'tools/list'});
  assert.throws(()=>transport.deliver({id:8,method:'tools/list'}));
  await transport.send({jsonrpc:'2.0',id:0,result:{tools:[]}});transport.deliver({id:8,method:'tools/list'});await transport.close();
  assert.equal(messages.at(-1).kind,'pipe/close');
});
test('writer refuses queue overflow while first actual stream write is blocked', async () => {
  let finish;const stream=new Writable({write(_chunk,_encoding,callback){finish=callback}});const writer=new Writer(stream,32);
  const one=writer.send({x:'a'.repeat(20)});await Promise.resolve();const two=writer.send({x:'b'.repeat(20)});
  assert.throws(()=>writer.send({x:'c'.repeat(20)}));finish();await one;await Promise.resolve();finish();await two;writer.close();stream.destroy();
});

test('cancelled owner calls retain admission and prevent deletion until late native settlement',async()=>{
  const messages=[];const owner=new Owner({send:async v=>messages.push(v)});const stops=[];
  for(let i=0;i<8;i++){const stop=new AbortController();stops.push(stop);const pending=owner.call('submit_result',{fixture:i},stop.signal);stop.abort();await assert.rejects(pending,e=>e.code==='cancelled');}
  await assert.rejects(owner.call('submit_result',{}),e=>e.code==='busy');await assert.rejects(owner.drain(1),e=>e.code==='owner_busy');
  for(const request of messages.filter(v=>v.kind==='owner/request'))owner.reply({id:request.id,ok:false});await owner.drain(10);owner.close();
});
test('closing an owner never turns uncertain callbacks into a successful drain',async()=>{
  const owner=new Owner({send:async()=>{}});const pending=owner.call('submit_result',{});const refused=assert.rejects(pending,e=>e.code==='closed');const drain=assert.rejects(owner.drain(100),e=>e.code==='closed');owner.close();await Promise.all([refused,drain]);
});
