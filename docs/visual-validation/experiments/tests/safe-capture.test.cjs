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
let count=0;
async function test(name,fn){await fn();count++;console.log('PASS '+name);}
(async()=>{
 await test('rectangular address roundtrip, non-A1 origin and sheet terminal',()=>{
   for(const a of ['A1:FX2791','AZ301:FP420','XFD1048576:XFD1048576'])assert.equal(S.address(S.parse(a)),a);
   assert.throws(()=>S.parse('A0:A1'));assert.throws(()=>S.parse('A1:XFE3'));
 });
 await test('PNG validation rejects empty, bad signature and missing end marker',()=>{
   assert.throws(()=>S.png(''));assert.throws(()=>S.png(Buffer.alloc(50).toString('base64')));
   assert.throws(()=>S.png(image(2,3).slice(0,-4)));assert.equal(S.png(image(15440,32766)).height,32766);
 });
 await test('hospital-size plan stays below BOTH area and dimension budgets with complete cell coverage',async()=>{
   install({col:()=>11579.25/180});const s=new S.Session('hospital.xlsx');
   const r=S.parse('A1:FX2791'),tiles=await s.plan(' KHV_2022',r);
   assert.ok(tiles.length>2);assert.ok(tiles.every(t=>S.fits(t.geometry,s.limitsFor(' KHV_2022'))));
   assert.ok(tiles.every(t=>t.rect.c0===0 && t.rect.c1===180));
   assert.equal(tiles.reduce((n,t)=>n+(t.rect.r1-t.rect.r0)*(t.rect.c1-t.rect.c0),0),2791*180);
   for(let i=1;i<tiles.length;i++)assert.equal(tiles[i-1].rect.r1,tiles[i].rect.r0);
 });
 await test('two-dimensional split and irregular/hidden geometry cover every requested cell exactly once',async()=>{
   install({col:i=>i%9===0?0:100,row:i=>i%13===0?0:12.75+(i%8)});
   const s=new S.Session('x');s.limits.max_pixels=30000;s.limits.max_edge_px=300;
   const r={r0:0,r1:73,c0:0,c1:25},tiles=await s.plan('S',r);
   const seen=new Uint8Array(73*25);
   for(const t of tiles){if(!t.invisible)assert.ok(S.fits(t.geometry,s.limitsFor('S')));for(let y=t.rect.r0;y<t.rect.r1;y++)for(let x=t.rect.c0;x<t.rect.c1;x++)seen[y*25+x]++;}
   assert.ok(seen.every(x=>x===1));
 });
 await test('a tile is saved before an analysis callback can fail',async()=>{
   const m=install();const s=new S.Session('x');const plan=await s.plan('S',S.parse('A1:D2'));
   await assert.rejects(s.capturePlan('S',plan,'c00001',async()=>{assert.ok(m.requests.some(x=>x.url==='/image-batch' && x.data.images.some(i=>i.id.startsWith('c00001-t'))));throw S.fault('TEST','analysis failed',{},true)}));
   assert.equal(s.state.attempts[0].status,'fatal');assert.ok(s.state.attempts[0].relative_png);
   assert.ok(m.requests.some(x=>x.url==='/capture-progress' && x.data.current.stage==='tile_failed'));
 });
 await test('native errors trigger small control then subdivision, with failure records retained',async()=>{
   const m=install({fail:r=>r.r1-r.r0>32});const s=new S.Session('x');const plan=await s.plan('S',S.parse('A1:D128'));
   const done=await s.capturePlan('S',plan,'c');assert.equal(done.length,4);
   assert.ok(s.state.attempts.some(x=>x.status==='failed'));assert.ok(m.events.some(x=>x.stage==='recovery_control_started'));
 });
 await test('failed control stops native session',async()=>{
   let fail=false;const m=install({fail:()=>fail});const s=new S.Session('x');const plan=await s.plan('S',S.parse('A1:D8'));const before=m.calls.length;fail=true;
   await assert.rejects(s.capturePlan('S',plan,'c'));assert.equal(s.dead,true);assert.equal(m.calls.length-before,2);
 });
 await test('a native timeout is not retried or labelled an image-size threshold',async()=>{
   install({hang:true});const s=new S.Session('x');s.limits.timeout_ms=5;
   await assert.rejects(s.plan('S',S.parse('A1:D8')),e=>e.code==='NATIVE_TIMEOUT');assert.equal(s.dead,true);
   await assert.rejects(s.plan('S',S.parse('A1:A1')),e=>e.code==='NATIVE_SESSION_STOPPED');
 });
 await test('wholly hidden area produces metadata but no unsafe empty image call',async()=>{
   const m=install({row:()=>0});const s=new S.Session('x');const plan=await s.plan('S',S.parse('A1:D8'));
   assert.ok(plan.every(x=>x.invisible));assert.deepEqual(await s.capturePlan('S',plan,'c'),[]);assert.equal(m.calls.length,0);
 });
 for(const factor of [4/3,2]) await test(`F integration at ${factor}: raw PNG, calibrated crop and offsets, no Canvas encoding`,async()=>{
   install({col:()=>15,row:()=>12.75,sx:()=>factor,sy:()=>factor});const session=new S.Session('x',{strategy:'F'});
   const code=fs.readFileSync(path.join(web,'modes/range-image.js'),'utf8');
   const cut=code.lastIndexOf('  try {\n    await execute();');assert.ok(cut>0);
   const hooked=code.slice(0,cut)+`globalThis.hooks={run:renderPrecomputedPlannedBatch,set:s=>{safeSession=s}}; }());`;
   class Img { set src(s){if(!s)return;const p=S.png(s.split(',')[1]);this.width=this.naturalWidth=p.width;this.height=this.naturalHeight=p.height;queueMicrotask(()=>this.onload());} }
   const ui=new Proxy({}, {get:()=>()=>{}});
   const win={fetch:global.fetch,setTimeout,SafeCapture:S};
   const ctx={window:win,SafeCapture:S,Excel:global.Excel,performance,console,atob,Map,Uint8Array,Number,Image:Img,
      ExperimentUi:ui,ExperimentEvents:{post:async()=>{}},document:{createElement(){return {width:0,height:0,getContext(){return{drawImage(){},getImageData(x,y,w,h){const data=new Uint8ClampedArray(w*h*4);data.fill(255);return{data}}}},toDataURL(){throw new Error('Canvas encoding must not be called')}};}}};
   vm.createContext(ctx);vm.runInContext(hooked,ctx);ctx.hooks.set(session);
   const item={id:'c00001',sheet:'S',address:'A1:F5',bottom_row:4,base_right_column:2,end_column:5,probe_extra:8,overflow_probe:true,column_widths:new Array(16384).fill(15)};
   const out=await ctx.hooks.run('x',[item],1,1,{strategy:'F'});
   assert.equal(session.state.sheets[0].status,'complete');assert.equal(item.address,'A1:C5');
   assert.equal(out.captures.length,1);assert.ok(out.captures[0].valid_rect_px.width<out.captures[0].png_width);
   assert.equal(out.captures[0].valid_rect_px.width,Math.round(45*factor)+1);
   assert.equal(out.captures[0].sheet_offset_px.x,0);assert.equal(out.captures[0].sheet_offset_px.y,0);
 });

 await test('representative real review observations are explained by a measured 2 px/pt',()=>{
   const observations=[
     {geometry:{left:0,top:0,right:970,bottom:625},png:{width:1941,height:1251}},
     {geometry:{left:0,top:0,right:715.5,bottom:318},png:{width:1432,height:637}},
     {geometry:{left:0,top:0,right:963,bottom:245},png:{width:1927,height:491}}
   ];
   assert.equal(S.fitAxis(observations,'x').scale,2);assert.equal(S.fitAxis(observations,'y').scale,2);
 });
 await test('live controls calibrate 2 px/pt independently of an unrelated browser DPR',async()=>{
   global.devicePixelRatio=1;const m=install({sx:()=>2,sy:()=>2});const s=new S.Session('144dpi.xlsx');
   const plan=await s.plan('S',S.parse('A1:D20'));
   assert.equal(s.limitsFor('S').scale_x,2);assert.equal(s.limitsFor('S').scale_y,2);
   const captures=await s.capturePlan('S',plan,'c');assert.equal(captures.length,1);
   assert.equal(captures[0].raster_profile.scale_x,2);
   assert.equal(s.state.calibrations[0].observations.length,4);
   assert.equal(s.state.attempts.length,1);assert.equal(m.calls.length,5);
   assert.equal(s.state.captures.length,1);assert.ok(m.events.some(e=>e.stage==='raster_calibrated'));
 });
 await test('anisotropic scale is measured rather than assuming x equals y',async()=>{
   install({sx:()=>2,sy:()=>4/3});const s=new S.Session('x');const plan=await s.plan('S',S.parse('A1:D20'));
   assert.equal(s.limitsFor('S').scale_x,2);assert.ok(Math.abs(s.limitsFor('S').scale_y-4/3)<1e-12);
   assert.equal((await s.capturePlan('S',plan,'c')).length,1);
 });
 await test('a calibrated scale mismatch fails fast without single-cell subdivision',async()=>{
   let scale=4/3;const m=install({sx:()=>scale,sy:()=>scale});const s=new S.Session('x');
   const plan=await s.plan('S',S.parse('A1:D64'));const before=m.calls.length;scale=2;
   await assert.rejects(s.capturePlan('S',plan,'c'),e=>e.code==='PNG_GEOMETRY_MISMATCH' && e.fatal);
   assert.equal(m.calls.length-before,1);assert.equal(s.state.attempts.length,1);
   assert.ok(!m.events.some(e=>e.stage==='tile_recovery_split'));
   assert.ok(s.state.attempts[0].relative_png);
 });
 await test('nonlinear or capped small controls do not teach a false raster scale',()=>{
   const values=[10,40,100].map(x=>({geometry:{left:0,top:0,right:x,bottom:x},png:{width:Math.min(81,2*x+1),height:Math.min(81,2*x+1)}}));
   assert.throws(()=>S.fitAxis(values,'x'),e=>e.code==='CALIBRATION_MODEL_REJECTED');
 });
 await test('wide hospital-scale captures stay full-width with actual area <=16M at 2 px/pt',async()=>{
   const m=install({col:()=>12200/180,sx:()=>2,sy:()=>2});const s=new S.Session('hospital');
   const rect=S.parse('A1:FX2791'),plan=await s.plan(' KHV_2022',rect);
   assert.ok(plan.every(t=>t.rect.c0===0 && t.rect.c1===180));
   assert.equal(plan.reduce((n,t)=>n+t.rect.r1-t.rect.r0,0),2791);
   const images=await s.capturePlan(' KHV_2022',plan,'c');
   assert.ok(images.every(c=>S.actualFits({width:c.png_width,height:c.png_height},s.limitsFor(' KHV_2022'))));
   assert.ok(images.length>1);assert.equal(s.state.attempts.length,images.length);
 });
 await test('calibration is cached per worksheet; different worksheets may have different scales',async()=>{
   let scale=4/3;const m=install({sx:()=>scale,sy:()=>scale});const s=new S.Session('x');
   await s.plan('One',S.parse('A1:D20'));assert.equal(m.calls.length,4);
   await s.plan('One',S.parse('A1:D40'));assert.equal(m.calls.length,4);
   scale=2;await s.plan('Two',S.parse('A1:D20'));assert.equal(m.calls.length,8);
   assert.equal(s.limitsFor('Two').scale_x,2);assert.ok(Math.abs(s.limitsFor('One').scale_x-4/3)<1e-12);
 });

 await test('nonfatal analysis failure never recaptures a successfully persisted tile',async()=>{
   const m=install();const s=new S.Session('x');const plan=await s.plan('S',S.parse('A1:D8'));
   const before=m.calls.length;
   await assert.rejects(s.capturePlan('S',plan,'analysis',async()=>{
     await new Promise(resolve=>setTimeout(resolve,2));throw S.fault('CANVAS_BUDGET','analysis budget mismatch',{},false);
   }),e=>e.code==='CANVAS_BUDGET');
   assert.equal(m.calls.length-before,1);assert.equal(s.state.attempts.length,1);
   assert.equal(s.state.attempts[0].failure_stage,'analysis');assert.ok(s.state.attempts[0].relative_png);
   assert.ok(s.timings.analysis_ms>0);assert.ok(!m.events.some(e=>e.stage==='recovery_control_started'));
 });
 await test('overflow decoder uses selected 16/32/64 MP before allocation and releases failed canvases',async()=>{
   const code=fs.readFileSync(path.join(web,'modes/range-image.js'),'utf8');
   const cut=code.lastIndexOf('  try {\n    await execute();');
   const hook=code.slice(0,cut)+`globalThis.decode=decodePngForOverflow; }());`;
   let allocations=0,canvas=null;
   class Img{set src(value){if(!value)return;const h=S.png(value.split(',')[1]);this.naturalWidth=h.width;this.naturalHeight=h.height;queueMicrotask(()=>this.onload());}}
   const context={Image:Img,SafeCapture:S,window:{fetch:async()=>({})},performance,console,atob,Uint8Array,
     document:{createElement(){allocations++;canvas={width:0,height:0,getContext(){return{drawImage(){},getImageData(){return{data:new Uint8ClampedArray(4)}}}}};return canvas;}}};
   vm.createContext(context);vm.runInContext(hook,context);
   for(const [mp,h]of [[16,1036],[32,2072],[64,4142]]){
     const p=await context.decode(image(15440,h),{...S.DEFAULTS,max_pixels:mp*1000000});
     assert.equal(p.width,15440);assert.equal(p.height,h);p.canvas.width=0;p.canvas.height=0;
   }
   const before=allocations;await assert.rejects(context.decode(image(15440,2072),{...S.DEFAULTS,max_pixels:16000000}),e=>e.code==='CANVAS_BUDGET');
   assert.equal(allocations,before); // header guard precedes Image/Canvas work
   context.document.createElement=()=>{canvas={width:0,height:0,getContext:()=>null};return canvas;};
   await assert.rejects(context.decode(image(100,100),S.DEFAULTS),e=>e.code==='CANVAS_UNAVAILABLE');
   assert.equal(canvas.width,0);assert.equal(canvas.height,0);
 });
 console.log(`${count} safe-capture tests passed (algorithm/protocol mocks, not Excel).`);
})().catch(e=>{console.error(e);process.exitCode=1});
