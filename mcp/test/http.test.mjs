import test from 'node:test';
import assert from 'node:assert/strict';
import { authorize } from '../src/http.mjs';
const token='a'.repeat(64),port=46123;
const request=()=>({socket:{remoteAddress:'127.0.0.1'},url:'/mcp',headers:{host:`127.0.0.1:${port}`,authorization:`Bearer ${token}`},rawHeaders:['Host',`127.0.0.1:${port}`,'Authorization',`Bearer ${token}`]});
test('native and exact local Origin pass; remote, null, aliases and duplicates fail',()=>{
  authorize(request(),port,token);const local=request();local.headers.origin=`http://127.0.0.1:${port}`;authorize(local,port,token);
  for(const origin of ['null','https://evil.example',`http://localhost:${port}`,`http://127.0.0.1:${port+1}`]){const r=request();r.headers.origin=origin;assert.throws(()=>authorize(r,port,token));}
  for(const change of [r=>r.socket.remoteAddress='::1',r=>r.headers.host='evil.example',r=>r.url='/mcp?token=x',r=>r.headers.authorization='Bearer '+'b'.repeat(64),r=>r.rawHeaders.push('Authorization',`Bearer ${token}`)]){const r=request();change(r);assert.throws(()=>authorize(r,port,token));}
});
