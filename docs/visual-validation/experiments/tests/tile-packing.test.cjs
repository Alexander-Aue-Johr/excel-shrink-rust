'use strict';
// Algorithm/protocol mocks only: no actual Excel/Canvas rendering is simulated here.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const web=path.resolve(__dirname,'../_shared/office-addin/web');
const S=require(path.join(web,'safe-capture.js'));
function image(w,h){const b=Buffer.alloc(45);Buffer.from([137,80,78,71,13,10,26,10]).copy(b);b.writeUInt32BE(13,8);b.write('IHDR',12);b.writeUInt32BE(w,16);b.writeUInt32BE(h,20);b.write('IEND',37);return b.toString('base64');}
function install({row=()=>12.75,col=()=>52.5,fail=()=>false,hang=false,sx=()=>4/3,sy=()=>4/3}={}) {
  const events=[],requests=[],calls=[];
  const sum=(f,n)=>{let x=0;for(let i=0;i<n;i++)x+=f(i);return x;};
  global.fetch=async(url,init={})=>{
    const data=init.body?JSON.parse(init.body):{};requests.push({url,data});
    if(url==='/client-event')events.push(data);
    if(url==='/image-batch')return {ok:true,json:async()=>({captures:data.images.map(x=>{const p=S.png(x.base64);return{id:x.id,sheet:x.sheet,address:x.address,png_width:p.width,png_height:p.height,relative_png:'mock/'+x.id+'.png'};})})};
    return {ok:true,json:async()=>({ok:true})};
  };
  global.Excel={run:async fn=>{
    if(hang)return new Promise(()=>{});
    const queue=[];
    const range=r=>({load(){},get top(){return sum(row,r.r0)},get left(){return sum(col,r.c0)},
      get height(){return sum(row,r.r1)-sum(row,r.r0)},get width(){return sum(col,r.c1)-sum(col,r.c0)},
      getImage(){const result={};queue.push(()=>{calls.push(r);if(fail(r))throw Object.assign(new Error('mock native failure'),{code:'GeneralException'});
        result.value=image(Math.round((sum(col,r.c1)-sum(col,r.c0))*sx(r))+1,Math.round((sum(row,r.r1)-sum(row,r.r0))*sy(r))+1);});return result;}});
    const sheet={getRangeByIndexes:(r,c,n,m)=>range({r0:r,r1:r+n,c0:c,c1:c+m}),getRange:a=>range(S.parse(a))};
    return fn({workbook:{worksheets:{getItem:()=>sheet}},trace(){},sync:async()=>{while(queue.length)queue.shift()();}});
  }};
  return {events,requests,calls};
}

let n=0;async function test(name,fn){await fn();n++;console.log('PASS '+name);}
(async()=>{
 for(const scale of [4/3,2])await test(`budget packing at ${scale} fills more of each tile than balanced`,async()=>{
   install({col:()=>11579.25/180,sx:()=>scale,sy:()=>scale});
   const a=new S.Session('x',{capture_options:{tile_packing:'balanced'}}),b=new S.Session('x',{capture_options:{tile_packing:'budget'}});
   const r=S.parse('A1:FX2791'),old=await a.plan('s',r),fresh=await b.plan('s',r);
   assert.ok(fresh.length<old.length);assert.ok(fresh.every(t=>S.fits(t.geometry,b.limitsFor('s'))));
   assert.equal(fresh.reduce((sum,t)=>sum+t.rect.r1-t.rect.r0,0),2791);
   assert.ok(fresh.slice(0,-1).every(t=>t.packing.area_utilization>.95));
   console.log(JSON.stringify({scale,old_tiles:old.length,budget_tiles:fresh.length,simulated_uniform_rows:true}));
 });
 await test('32 and 64 MP remain opt-in and obey both safety limits',async()=>{
   install({col:()=>11579.25/180});const counts=[];
   for(const mp of [16,32,64]){const s=new S.Session('x',{capture_options:{tile_megapixels:mp}}),plan=await s.plan('s',S.parse('A1:FX2791'));counts.push(plan.length);assert.ok(plan.every(t=>S.actualFits(S.pixels(t.geometry,s.limitsFor('s')),s.limitsFor('s'))));}
   assert.ok(counts[0]>counts[1]&&counts[1]>counts[2]);
 });
 await test('invalid capture budgets are rejected',()=>{assert.throws(()=>new S.Session('x',{capture_options:{tile_megapixels:512}}));assert.throws(()=>new S.Session('x',{capture_options:{tile_packing:'unsafe'}}));});
 await test('measured packing retains source context and handles terminal boundaries',async()=>{
   install({row:()=>12,col:()=>10});const s=new S.Session('x');const r=S.parse('XFA1048500:XFD1048576');const plan=await s.plan('s',r);
   assert.equal(plan[0].rect.r0,r.r0);assert.equal(plan.at(-1).rect.r1,r.r1);assert.ok(plan.every(t=>t.rect.c0===r.c0&&t.rect.c1===r.c1));
 });
 console.log(`${n} packing mock tests passed.`);
})().catch(e=>{console.error(e);process.exitCode=1;});
