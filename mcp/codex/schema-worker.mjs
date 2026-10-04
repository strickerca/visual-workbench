// Compilation runs outside the IPC event loop under an explicit heap/time cap.
import { parentPort, workerData } from 'node:worker_threads';
import Ajv from 'ajv';
import { json, requireThat } from '../src/bounds.mjs';
const methods = ['initialize','thread/list','thread/read','thread/resume','turn/start','turn/interrupt'];
try {
  const schema = json(Buffer.from(workerData), 4 * 1024 * 1024);
  requireThat(schema.$schema === 'http://json-schema.org/draft-07/schema#' && Array.isArray(schema.oneOf), 'unsupported_schema');
  // No remote loads, custom code, coercion, defaults or data mutation. Unknown
  // formats are annotations; the adapter separately bounds every public field.
  const ajv = new Ajv({ strict:false, validateFormats:false, allErrors:false, addUsedSchema:false, inlineRefs:false });
  const validators = new Map();let inputs,turnParams;
  const resolved=value=>{
    let current=value;const seen=new Set();
    while(current?.$ref){requireThat(!seen.has(current.$ref),'unsupported_schema');seen.add(current.$ref);current=schema.definitions?.[current.$ref.slice('#/definitions/'.length)];}
    requireThat(current&&typeof current==='object','unsupported_schema');return current;
  };
  const kindOf=value=>value?.const??(value?.enum?.length===1?value.enum[0]:undefined);
  for (const method of methods) {
    const variants = schema.oneOf.filter(v => v.properties?.method?.const === method || v.properties?.method?.enum?.length === 1 && v.properties.method.enum[0] === method);
    requireThat(variants.length === 1, 'unsupported_schema');
    validators.set(method, ajv.compile({ $schema:schema.$schema, definitions:schema.definitions, ...variants[0] }));
    if(method==='turn/start'){
      const params=resolved(variants[0].properties.params);turnParams=params;const array=resolved(params.properties?.input);requireThat(array.type==='array','unsupported_schema');
      const item=resolved(array.items);inputs=(item.oneOf??item.anyOf??[item]).map(resolved);
    }
  }
  parentPort.on('message', ({ id, method, value }) => {
    try {
      let valid=validators.get(method)?.(value)===true;
      if(valid&&method==='turn/start')valid=Object.keys(value.params).every(key=>Object.hasOwn(turnParams.properties,key))&&value.params.input.every(input=>{
        const branches=inputs.filter(candidate=>kindOf(candidate.properties?.type)===input.type);
        // Generated serde schemas may allow unknown properties. Merely passing
        // AJV cannot prove that an older runtime understands detail=original.
        // Every sent field must be declared in the exact input-kind schema.
        return branches.length===1&&Object.keys(input).every(key=>Object.hasOwn(branches[0].properties,key));
      });
      parentPort.postMessage({ id, valid });
    }
    catch { parentPort.postMessage({ id, valid:false }); }
  });
  parentPort.postMessage({ ready:true });
} catch { parentPort.postMessage({ refused:true }); }
