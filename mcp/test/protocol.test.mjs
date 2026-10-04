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
    try{await client.connect(wire);assert.equal(client.getProtocolEra(),era);const tools=await client.listTools();assert.equal(tools.tools.length,6);
      const result=await client.callTool({name:'list_packages',arguments:{limit:1}});assert.equal(result.isError,undefined);assert.deepEqual(result.structuredContent,{packages:[]});
      const capture=await client.callTool({name:'capture_window',arguments:{selector:'window1'}});assert.equal(capture.isError,true);
      const invalid=await client.callTool({name:'list_packages',arguments:{limit:0}});assert.equal(invalid.isError,true);
    }finally{await client.close();await server.close();}
  });
  test(`official SDK ${version} through authenticated stateless HTTP`,{timeout:10_000},async()=>{
    const token='f'.repeat(64);const server=await listenHttp(make(),token,0);
    const client=new Client({name:'vw-fixture',version:'0.1.0'},{supportedProtocolVersions:[version],versionNegotiation:{mode}});
    try{await client.connect(new StreamableHTTPClientTransport(new URL(`http://127.0.0.1:${server.port}/mcp`),{requestInit:{headers:{Authorization:`Bearer ${token}`}}}));
      assert.equal(client.getProtocolEra(),era);assert.equal((await client.listTools()).tools.length,6);
      assert.deepEqual((await client.callTool({name:'list_packages',arguments:{}})).structuredContent,{packages:[]});
    }finally{await client.close();await server.close();}
  });
}
