import { spawn } from 'node:child_process';
import path from 'node:path';
import { rustExe } from './rust-target.js';

// The Rust executable serves HTTP itself; Node only owns the isolated fixture.
// review: automatic review of replies (the server ships with it OFF by default; the review
// tests turn it on explicitly).
export async function startRust({ dataDir, upstream, routerDb, localUrl, maxGenerations=4, review=true }) {
  const exe=process.env.MC_SERVER_EXE || rustExe('modularchat-server');
  const child=spawn(exe,[],{windowsHide:true,stdio:['ignore','pipe','pipe'],env:{...process.env,
    DATA_DIR:dataDir,PORT:'0',HOST:'127.0.0.1',MC_RUST_MODE:'experimental',REQUIRE_PAIRING:'1',
    MC_RUST_UPSTREAM_ENABLED:'1',NINEROUTER_URL:upstream,NINEROUTER_API_KEY:'sk-test-key',NINEROUTER_DB:routerDb || path.join(dataDir,'absent-router.db'),
    MC_LOCAL_MODELS_ENABLED:'0',MC_TEST_LOCAL_URL:localUrl || '',LOCAL_CHAT_PORT:'20591',LOCAL_EMBED_PORT:'20592',MC_MAX_GENERATIONS:String(maxGenerations),
    MC_REVIEW:review?'1':'0',
  }});
  let log='';child.stderr.on('data',c=>log=(log+c.toString()).slice(-10000));
  let pairingCode;
  const base=await new Promise((resolve,reject)=>{
    let out='';const timer=setTimeout(()=>{child.kill();reject(new Error(`Rust startup timeout: ${log}`));},30000);
    child.stdout.on('data',chunk=>{out+=chunk.toString();const match=out.match(/em (http:\/\/127\.0\.0\.1:\d+)/);if(match){pairingCode=out.match(/pareamento=(\d{6})/)?.[1];clearTimeout(timer);resolve(match[1]);}});
    child.once('error',e=>{clearTimeout(timer);reject(e);});child.once('exit',code=>{clearTimeout(timer);reject(new Error(`Rust startup exit ${code}: ${log}`));});
  });
  let closing;
  return {base,child,pairingCode,close(){
    if(closing)return closing;
    if(child.exitCode!==null||child.signalCode!==null)return Promise.resolve();
    closing=new Promise((resolve,reject)=>{
      const timer=setTimeout(()=>reject(new Error('Rust fixture did not exit within 5 seconds')),5000);
      child.once('exit',()=>{clearTimeout(timer);resolve();});
      if(!child.kill()){clearTimeout(timer);reject(new Error('Rust fixture could not be stopped'));}
    });return closing;
  }};
}
