import test from 'node:test';
import assert from 'node:assert/strict';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { serveStdio } from '@modelcontextprotocol/server/stdio';
import { factory } from '../src/server.mjs';
import { listenHttp } from '../src/http.mjs';
import { Grants } from '../src/grants.mjs';
const make=()=>factory({packages:{list:()=>[]},owner:{call:()=>assert.fail('read-only fixture must not invoke owner')},grants:new Grants(),principal:'test:no-grant'});
function pair(){const a={},b={};for(const [one,two] of [[a,b],[b,a]]){one.start=async()=>{};one.send=async message=>queueMicrotask(()=>two.onmessage?.(structuredClone(message)));one.close=async()=>{one.onclose?.();two.onclose?.();};}return[a,b];}
for(const [version,era,mode] of [['2025-11-25','legacy','legacy'],['2026-07-28','modern',{pin:'2026-07-28'}]]){
  test(`official SDK ${version} through the actual stdio-era router`,{timeout:10_000},async()=>{
    const [server,wire]=pair();await serveStdio(make(),{transport:server,legacy:'serve',onerror:()=>{}});
    const client=new Client({name:'vw-fixture',version:'0.1.0'},{supportedProtocolVersions:[version],versionNegotiation:{mode}});
    try{await client.connect(wire);assert.equal(client.getProtocolEra(),era);const tools=await client.listTools();assert.equal(tools.tools.length,8);assert.ok(tools.tools.every(tool=>tool.description.length>=120&&tool.description!==tool.name.replaceAll('_',' ')));assert.match(client.getInstructions()??'',/untrusted data/);
      const result=await client.callTool({name:'list_packages',arguments:{limit:1}});assert.equal(result.isError,undefined);assert.deepEqual(result.structuredContent,{packages:[]});
      const capture=await client.callTool({name:'capture_window',arguments:{selector:'window1'}});assert.equal(capture.isError,true);
      const invalid=await client.callTool({name:'list_packages',arguments:{limit:0}});assert.equal(invalid.isError,true);
    }finally{await client.close();await server.close();}
  });
  test(`official SDK ${version} through authenticated stateless HTTP`,{timeout:10_000},async()=>{
    const token='f'.repeat(64);const server=await listenHttp(make(),token,0);
    const client=new Client({name:'vw-fixture',version:'0.1.0'},{supportedProtocolVersions:[version],versionNegotiation:{mode}});
    try{await client.connect(new StreamableHTTPClientTransport(new URL(`http://127.0.0.1:${server.port}/mcp`),{requestInit:{headers:{Authorization:`Bearer ${token}`}}}));
      assert.equal(client.getProtocolEra(),era);assert.equal((await client.listTools()).tools.length,8);
      assert.deepEqual((await client.callTool({name:'list_packages',arguments:{}})).structuredContent,{packages:[]});
      // HTTP has no authenticated bridge identity: refused by name before any package or owner work.
      const submit=await client.callTool({name:'submit_result',arguments:{package_id:'019a3aaa-0123-789a-8abc-0123456789ab',text:'result',note:'note'}});
      assert.equal(submit.isError,true);assert.deepEqual(submit.content,[{type:'text',text:'bridge_connection_required'}]);
    }finally{await client.close();await server.close();}
  });
}

test('every tool tells an agent what it is for, and the set matches the registered tools', async () => {
  const { DESCRIPTIONS, SERVER_INSTRUCTIONS } = await import('../src/descriptions.mjs');
  const names = Object.keys(DESCRIPTIONS).sort();
  assert.deepEqual(names, ['capture_window', 'get_marker', 'get_marker_comparison', 'get_package', 'get_semantic',
    'list_packages', 'recapture_package', 'submit_result']);
  for (const [name, description] of Object.entries(DESCRIPTIONS)) {
    assert.ok(description.length >= 120 && description.length <= 600, name);
    assert.notEqual(description.toLowerCase(), name.replaceAll('_', ' '));
    assert.ok(/[.]$/.test(description), name);
  }
  for (const name of ['get_package', 'get_semantic']) assert.match(DESCRIPTIONS[name], /untrusted/);
  assert.match(SERVER_INSTRUCTIONS, /untrusted data/);
  assert.ok(SERVER_INSTRUCTIONS.length <= 1200);
});
