import { Worker } from 'node:worker_threads';
import { createHash } from 'node:crypto';
import { Refused, encoded, json, requireThat } from '../src/bounds.mjs';
export const sha = bytes => createHash('sha256').update(bytes).digest('hex');
export class SchemaGate {
  #worker; #closed=false; #closing; #next=0; #pending=new Map();
  static async open(bytes) {
    // Preflight before Worker structured-copy allocation. Reject references
    // outside this one document, executable regex patterns and giant unions.
    const root=json(bytes,4*1024*1024); let count=0; const todo=[root];
    while(todo.length){const value=todo.pop();requireThat(++count<=32768,'schema_limit');if(value&&typeof value==='object'){
      requireThat(!Object.hasOwn(value,'pattern')&&!Object.hasOwn(value,'patternProperties'),'unsupported_schema_pattern');
      if(Object.hasOwn(value,'$ref'))requireThat(typeof value.$ref==='string'&&/^#\/definitions\/[A-Za-z0-9_]{1,128}$/.test(value.$ref),'unsupported_schema_ref');
      for(const item of Object.values(value))if(item&&typeof item==='object')todo.push(item);
    }}
    const gate=new SchemaGate(); gate.hash=sha(bytes);
    gate.#worker=new Worker(new URL('./schema-worker.mjs',import.meta.url),{workerData:bytes,resourceLimits:{maxOldGenerationSizeMb:192,maxYoungGenerationSizeMb:16,stackSizeMb:4}});
    try {
      await new Promise((resolve,reject)=>{
        const timer=setTimeout(()=>reject(new Refused('schema_timeout')),10_000);
        gate.#worker.once('message',m=>{clearTimeout(timer);m.ready?resolve():reject(new Refused('unsupported_schema'));});
        gate.#worker.once('error',()=>{clearTimeout(timer);reject(new Refused('unsupported_schema'));});
        gate.#worker.once('exit',()=>{clearTimeout(timer);reject(new Refused('unsupported_schema'));});
      });
      gate.#worker.on('message',m=>{const pending=gate.#pending.get(m.id);if(pending){gate.#pending.delete(m.id);clearTimeout(pending.timer);pending.resolve(m.valid===true);}});
      gate.#worker.on('error',()=>{void gate.close();});gate.#worker.on('exit',()=>{void gate.close();});return gate;
    } catch(error){await gate.close();throw error;}
  }
  async accepts(method,value){
    requireThat(!this.#closed&&this.#pending.size<2,'schema_busy'); encoded(value,2*1024*1024);
    const id=++this.#next;
    return new Promise((resolve,reject)=>{
      const timer=setTimeout(()=>{this.#pending.delete(id);reject(new Refused('schema_timeout'));void this.close();},2000);
      this.#pending.set(id,{resolve,reject,timer});
      try{this.#worker.postMessage({id,method,value});}catch{clearTimeout(timer);this.#pending.delete(id);reject(new Refused('closed'));void this.close();}
    });
  }
  async close(){if(this.#closing)return this.#closing;this.#closed=true;for(const p of this.#pending.values()){clearTimeout(p.timer);p.reject(new Refused('closed'));}this.#pending.clear();this.#closing=this.#worker?.terminate()??Promise.resolve();return this.#closing;}
}
export function exactKeys(value,keys){requireThat(value&&typeof value==='object'&&!Array.isArray(value)&&Object.keys(value).length===keys.length&&keys.every(k=>Object.hasOwn(value,k)),'invalid_fields');}
export function imageGate(profile,binaryHash,schemaHash,model,images,now=Date.now()){
  requireThat(profile,'unverified_image_preprocessing');
  exactKeys(profile,['version','binarySha256','schemaSha256','model','detail','maxDimension','patchSize','maxPatches','verifiedAtMs','expiresAtMs','evidenceSha256']);
  requireThat(profile.version===1&&profile.binarySha256===binaryHash&&profile.schemaSha256===schemaHash&&profile.model===model,'unverified_image_preprocessing');
  requireThat(/^[0-9a-f]{64}$/.test(profile.evidenceSha256)&&Number.isSafeInteger(profile.verifiedAtMs)&&profile.verifiedAtMs<=now&&Number.isSafeInteger(profile.expiresAtMs)&&profile.expiresAtMs>now&&profile.expiresAtMs-profile.verifiedAtMs<=31*86400000,'expired_image_preprocessing');
  requireThat(['high','original','omitted'].includes(profile.detail)&&Number.isInteger(profile.maxDimension)&&profile.maxDimension>=1&&profile.maxDimension<=16384&&Number.isInteger(profile.patchSize)&&profile.patchSize>=1&&profile.patchSize<=1024&&Number.isInteger(profile.maxPatches)&&profile.maxPatches>=1&&profile.maxPatches<=1000000,'invalid_image_profile');
  requireThat(images.length>=1&&images.length<=66,'image_limit');
  for(const image of images){requireThat(Number.isInteger(image.width)&&Number.isInteger(image.height)&&image.width>0&&image.height>0&&image.width<=profile.maxDimension&&image.height<=profile.maxDimension&&Math.ceil(image.width/profile.patchSize)*Math.ceil(image.height/profile.patchSize)<=profile.maxPatches,'image_would_resize');}
  return profile.detail;
}
