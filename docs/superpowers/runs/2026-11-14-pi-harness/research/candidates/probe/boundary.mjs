import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createAgentSession,DefaultResourceLoader,SessionManager,SettingsManager} from '@earendil-works/pi-coding-agent';
import {createAssistantMessageEventStream} from '@earendil-works/pi-ai';
const root=path.resolve(import.meta.dirname,'scratch-boundary');fs.mkdirSync(root,{recursive:true});process.env.HOME=path.join(root,'home');process.env.PI_CODING_AGENT_DIR=path.join(root,'agent');
for(const mode of ['turn','settle','error','aborted','abort-during-settle','error-turn','aborted-turn','user-abort','guarded-error']){
 let session,calls=0,starts=0,proposed=false,seenBatch=false;
 const stored=()=>session?.sessionManager.getBranch().filter(e=>e.type==='custom_message').map(e=>({type:e.customType,body:e.content}))??[];
 const log=(event,extra={})=>console.log(JSON.stringify({mode,event,calls,starts,idle:session?.isIdle,stored:stored(),...extra}));
 const batch=[{id:'p3',priority:3},{id:'p0',priority:0},{id:'p1',priority:1}].sort((a,b)=>a.priority-b.priority).map(x=>({type:'custom_message',customType:x.id,content:`exact body ${x.id}`,display:false}));
 const loader=new DefaultResourceLoader({cwd:root,agentDir:process.env.PI_CODING_AGENT_DIR,settingsManager:SettingsManager.inMemory({}),noContextFiles:true,noSkills:true,noThemes:true,noPromptTemplates:true,extensionFactories:[pi=>{
 pi.on('before_agent_start',e=>{starts++;log('before_agent_start',{messageType:e.messageType});return {message:{customType:'startup',content:'exact startup body',display:false}};});
 pi.on('context',e=>{const custom=e.messages.filter(m=>m.role==='custom').map(m=>({type:m.customType,body:m.content}));log('context',{custom});if(custom.some(m=>m.type==='p0')){assert.deepEqual(custom.filter(m=>/^p\d/.test(m.type)),batch.map(m=>({type:m.customType,body:m.content})));assert.deepEqual(stored().filter(m=>/^p\d/.test(m.type)),batch.map(m=>({type:m.customType,body:m.content})));seenBatch=true;}});
 for(const boundary of ['turn_end','agent_before_settle']){
 pi.on(boundary,async(e,ctx)=>{log(boundary+':first',{outcome:e.outcome,entries:e.entries,canContinue:e.context.canContinue});const target=(mode==='turn'||mode.endsWith('-turn'))?'turn_end':'agent_before_settle';if(boundary!==target||proposed)return;if(mode==='guarded-error'&&e.outcome!=='completed'){log('guarded_skip');return;}proposed=true;if(mode==='abort-during-settle')ctx.abort();log('proposing',{boundary});return {entries:[...e.entries,...batch],continue:true};});
 pi.on(boundary,e=>{log(boundary+':second',{outcome:e.outcome,proposedTypes:e.entries.filter(x=>x.type==='custom_message').map(x=>x.customType),previewTypes:e.context.contextMessages.filter(x=>x.role==='custom').map(x=>x.customType)});});
 }
 pi.on('agent_end',()=>log('agent_end'));pi.on('agent_settled',e=>log('agent_settled',{aborted:e.aborted}));
 }]});
 await loader.reload();({session}=await createAgentSession({cwd:root,agentDir:process.env.PI_CODING_AGENT_DIR,settingsManager:SettingsManager.inMemory({}),resourceLoader:loader,sessionManager:SessionManager.create(root,path.join(root,'sessions')),tools:[],model:{id:'fake',name:'Fake',api:'openai-completions',provider:'probe',baseUrl:'http://invalid.local',reasoning:false,input:['text'],cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:10000,maxTokens:1000}}));
 session.modelRuntime.hasConfiguredAuth=()=>true;await session.bindExtensions({mode:'json',onError:e=>log('extension_error',{error:e.error})});
 session.agent.streamFunction=(model,context)=>{calls++;log('provider',{startup:JSON.stringify(context.messages).includes('exact startup body'),batch:JSON.stringify(context.messages).includes('exact body p0')});const stream=createAssistantMessageEventStream();queueMicrotask(()=>{if(mode==='user-abort')session.abort();const reason=mode.includes('error')?'error':mode.includes('aborted')||mode==='user-abort'?'aborted':'stop';const message={role:'assistant',content:[{type:'text',text:'fake'}],api:model.api,provider:model.provider,model:model.id,usage:{input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:reason,timestamp:Date.now(),...(reason==='error'?{errorMessage:'synthetic non-retryable error'}:{})};stream.push(reason==='stop'?{type:'done',reason,message}:{type:'error',reason,error:message});});return stream;};
 await session.prompt('boundary test');log('finished',{seenBatch});
 assert.equal(starts,1,'continuation must not reenter before_agent_start');
 assert.equal(calls,['turn','settle','error','aborted'].includes(mode)?2:1);
 assert.equal(seenBatch,['turn','settle','error','aborted'].includes(mode));
 await session.dispose();
}
