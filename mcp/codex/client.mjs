import { randomUUID } from 'node:crypto';
import { Refused, encoded, requireThat, boundedString } from '../src/bounds.mjs';
import { Lines, Writer } from '../src/frames.mjs';
import { startChild } from './process.mjs';
export const idText=value=>{boundedString(value,128);requireThat(/^[A-Za-z0-9._:-]+$/.test(value),'invalid_id');return value;};
export const safeText=(value,cap)=>{boundedString(value,cap);requireThat(!/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(value),'invalid_text');return value;};
/** The App Server is a child of the native kill-on-close Job. No response text,
 * auth objects, prompts or approval parameters are forwarded to logs. */
export class CodexClient {
  #pending=new Map();#next=0;#failure;#child;#writer;#done;#events;#closed;
  constructor(child,gate,onEvent=()=>{}){
    this.gate=gate;this.#child=child;this.#events=onEvent;this.#writer=new Writer(child.stdin,4*1024*1024);
    this.#done=new Promise(resolve=>child.once('close',()=>{this.fail('runtime_closed');resolve();}));
    child.once('error',()=>this.fail('runtime_closed'));child.stdin.on('error',()=>this.fail('runtime_closed'));
    const lines=new Lines(value=>this.receive(value),()=>this.fail('runtime_protocol'),4*1024*1024);
    child.stdout.on('data',bytes=>lines.push(bytes));child.stdout.once('end',()=>lines.end());
  }
  static async open(executable,gate,onEvent,spawnProcess){
    const client=new CodexClient(startChild(executable,['app-server'],spawnProcess),gate,onEvent);
    try {await client.call('initialize',{clientInfo:{name:'visual_workbench',version:'0.1.0',title:'Visual Workbench owner handoff'}});await client.#writer.send({method:'initialized',params:{}});return client;}
    catch(error){await client.close();throw error;}
  }
  fail(code){if(this.#failure)return;this.#failure=new Refused(code);for(const pending of this.#pending.values()){clearTimeout(pending.timer);pending.reject(this.#failure);}this.#pending.clear();try{this.#child.kill();}catch{}this.#events({kind:'runtime/refused',code});}
  receive(message){
    try {
      requireThat(message&&typeof message==='object'&&!Array.isArray(message),'runtime_protocol');
      if(typeof message.method==='string'){
        if(message.id!==undefined){
          // Explicit refusal, never approve an agent action, enter a password,
          // or echo an untrusted server request into an owner command.
          requireThat(typeof message.id==='string'&&message.id.length<=128||Number.isSafeInteger(message.id),'runtime_protocol');
          void this.#writer.send({id:message.id,error:{code:-32601,message:'This handoff client does not grant approvals or answer interactive requests.'}}).catch(()=>this.fail('runtime_protocol'));
          this.#events({kind:'approval/refused'});return;
        }
        if(message.method==='turn/completed'){
          const p=message.params;idText(p?.threadId);idText(p?.turn?.id);requireThat(['completed','failed','interrupted'].includes(p.turn.status),'runtime_protocol');
          this.#events({kind:'turn/completed',threadId:p.threadId,turnId:p.turn.id,status:p.turn.status});
        }
        return; // Bounded lines are discarded; agent content is not a log.
      }
      const pending=this.#pending.get(message.id);requireThat(pending,'runtime_protocol');
      requireThat((message.error!==undefined)!==Object.hasOwn(message,'result'),'runtime_protocol');
      this.#pending.delete(message.id);clearTimeout(pending.timer);
      if(message.error!==undefined)pending.reject(new Refused('runtime_request_refused'));else{requireThat(Object.hasOwn(message,'result'),'runtime_protocol');pending.resolve(message.result);}
    }catch{this.fail('runtime_protocol');}
  }
  async call(method,params){
    requireThat(!this.#failure&&!this.#closed&&this.#pending.size<2,'runtime_busy');const id=++this.#next;const message={id,method,params};
    requireThat(await this.gate.accepts(method,message),'unsupported_runtime_schema');requireThat(!this.#failure&&!this.#closed&&this.#pending.size<2,'closed');
    let response;const result=new Promise((resolve,reject)=>{const timer=setTimeout(()=>{this.#pending.delete(id);reject(new Refused('runtime_reply_uncertain'));this.fail('runtime_reply_uncertain');},30_000);response={resolve,reject,timer};this.#pending.set(id,response);});
    // Attach rejection before a potentially slow pipe write.
    result.catch(()=>{});
    try{await this.#writer.send(message);}catch{this.fail('runtime_reply_uncertain');}
    return result;
  }
  async list(cursor=null){
    requireThat(cursor===null||typeof cursor==='string'&&cursor.length<=4096,'cursor');
    const result=await this.call('thread/list',{limit:16,cursor,archived:false});requireThat(Array.isArray(result?.data)&&result.data.length<=16,'runtime_protocol');
    const data=result.data.map(item=>{idText(item.id);safeText(item.preview??'',4096);const status=item.status?.type;requireThat(['notLoaded','idle','active','systemError'].includes(status),'runtime_protocol');return {id:item.id,preview:item.preview??'',status};});
    requireThat(result.nextCursor===null||result.nextCursor===undefined||typeof result.nextCursor==='string'&&result.nextCursor.length<=4096,'runtime_protocol');
    return {data,nextCursor:result.nextCursor??null};
  }
  async select(threadId){
    idText(threadId);const read=await this.call('thread/read',{threadId,includeTurns:false});
    requireThat(read?.thread?.id===threadId&&['idle','notLoaded'].includes(read.thread.status?.type),'thread_not_idle');
    // Loading a selected existing thread is distinct from sending a turn. No
    // path/history/model/security/configuration override is supplied.
    const result=await this.call('thread/resume',{threadId,excludeTurns:true});
    requireThat(result?.thread?.id===threadId&&result.thread.status?.type==='idle','thread_not_idle');
    return {threadId,model:safeText(result.model,256),cwd:safeText(result.cwd,4096)};
  }
  async idle(threadId){const result=await this.call('thread/read',{threadId,includeTurns:false});requireThat(result?.thread?.id===threadId&&result.thread.status?.type==='idle','thread_not_idle');}
  async close(){
    if(this.#closed)return this.#closed;
    this.#closed=(async()=>{this.fail('closed');let timer;try{const timed=new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Refused('runtime_cleanup_uncertain')),5000);});await Promise.race([this.#done,timed]);}finally{clearTimeout(timer);}})();return this.#closed;
  }
}
/** One immutable owner preview, consumed before any possible turn/start write.
 * Cancellation/ambiguous response never re-arms the same attempt. */
export class SendAttempt {
  constructor(preview){this.preview=JSON.parse(encoded(preview,2*1024*1024));this.id=randomUUID();this.consumed=false;}
  consume(displayed){requireThat(!this.consumed,'send_already_attempted');requireThat(displayed?.previewId===this.id&&displayed?.digest===this.preview.digest&&Object.keys(displayed).length===2,'send_preview_changed');this.consumed=true;return this.preview;}
}

/** Correlates only this owner's turn, including completion-before-start-reply.
 * Older notifications for the same thread never become an accepted receipt. */
export class TurnReceipts {
  active=null;
  begin(threadId){requireThat(!this.active,'turn_active');this.active={threadId,awaiting:true,early:new Map()};}
  observe(event){
    const current=this.active;if(!current||current.threadId!==event.threadId)return false;
    if(current.awaiting){requireThat(current.early.has(event.turnId)||current.early.size<8,'runtime_protocol');current.early.set(event.turnId,event.status);return false;}
    if(current.turnId!==event.turnId)return false;
    this.active=null;return true;
  }
  accepted(turn){
    idText(turn?.id);requireThat(['inProgress','completed','failed','interrupted'].includes(turn.status)&&this.active?.awaiting,'runtime_protocol');
    const current=this.active;const status=current.early.get(turn.id)??turn.status;
    this.active=status==='inProgress'?{threadId:current.threadId,turnId:turn.id}:null;return status;
  }
  uncertain(threadId){this.active={threadId,uncertain:true};}
}
