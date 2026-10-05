import fs from 'node:fs';
const dir=new URL('.',import.meta.url);
const read=n=>fs.readFileSync(new URL(n,dir),'utf8');
const write=(n,x)=>fs.writeFileSync(new URL(n,dir),typeof x==='string'?x:JSON.stringify(x,null,2));
const args=JSON.parse(read('roast-design-1-args.json'));
const results=JSON.parse(read('roast-design-1-results.json'));
const AsyncFunction=Object.getPrototypeOf(async function(){}).constructor;
const script=read('roast-design-1-engine.txt');
let missing=[];
const agent=async(prompt,opts)=>{
 const name=opts.label.replaceAll(':','-').replaceAll('#','-');
 write('roast-design-1-prompt-'+name+'.txt',prompt);
 if(!(opts.label in results)){missing.push(opts.label);throw Error('Missing actual native agent result '+opts.label);}
 return results[opts.label];
};
const parallel=async thunks=>Promise.all(thunks.map(async f=>{try{return await f();}catch(e){return null;}}));
let out;
try{out=await new AsyncFunction('args','agent','parallel','log','phase',script)(args,agent,parallel,()=>{},()=>{});}catch(e){if(!missing.length)throw e;}
write('roast-design-1-missing.json',[...new Set(missing)]);
if(out){write('roast-design-1-computed.json',out);write('roast-design-1-coverage.json',out.coverage);}
console.log(JSON.stringify({missing:[...new Set(missing)],coverage:out?.coverage,verdict:out?.verdict}));
