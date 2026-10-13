// Node only orchestrates fixtures/assertions. Every store operation runs in Rust.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import path from 'node:path';
import { rustExe } from './rust-target.js';
export class RustStore {
  pending = new Map(); seq = 0;
  constructor(file) {
    const exe=process.env.MC_CONTRACT_EXE || rustExe('mc-contract');
    this.process=spawn(exe,[path.dirname(file)],{windowsHide:true,stdio:['pipe','pipe','pipe']});
    this.readyPromise=new Promise((resolve,reject)=>{this.resolveReady=resolve;this.rejectReady=reject;});
    this.errors='';this.process.stderr.on('data',c=>this.errors+=c.toString().slice(0,2000));
    createInterface({input:this.process.stdout}).on('line',line=>{
      const r=JSON.parse(line);if(r.ready)return this.resolveReady();
      const p=this.pending.get(r.id);if(!p)return;this.pending.delete(r.id);clearTimeout(p.timer);
      if(r.ok)p.resolve(r.result);else p.reject(Object.assign(new Error(r.error.message),r.error,r.error.extra));
    });
    const fail=e=>{this.rejectReady(e);for(const p of this.pending.values()){clearTimeout(p.timer);p.reject(e);}this.pending.clear();};
    this.process.on('error',fail);this.process.on('exit',code=>{if(code!==0||this.pending.size)fail(new Error(`Rust contract exit ${code}: ${this.errors}`));});
  }
  ready(){return this.readyPromise;}
  call(fn,...args){return new Promise((resolve,reject)=>{
    const id=++this.seq;const timer=setTimeout(()=>{this.pending.delete(id);reject(new Error(`Rust contract timeout: ${fn}`));},120000);
    this.pending.set(id,{resolve,reject,timer});this.process.stdin.write(JSON.stringify({id,fn,args})+'\n');
  });}
  async close(){this.process.stdin.end();if(this.process.exitCode!=null)return;await new Promise(resolve=>this.process.once('exit',resolve));}
}
