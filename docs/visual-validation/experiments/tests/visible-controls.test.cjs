'use strict';
// Protocol/model tests only. These do not emulate or validate Excel's renderer.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const web=path.resolve(__dirname,'../_shared/office-addin/web');
const S=require(path.join(web,'safe-capture.js')),P=require(path.join(web,'capture-policy.js'));
function png(w,h){const b=Buffer.alloc(45);Buffer.from([137,80,78,71,13,10,26,10]).copy(b);b.writeUInt32BE(13,8);b.write('IHDR',12);b.writeUInt32BE(w,16);b.writeUInt32BE(h,20);b.write('IEND',37);return b.toString('base64');}
function mock(options={}) {
 const events=[],images=[],queries=[],posts=[];let sheet='S',imageFail=false;
 const cw=(s,c)=>options.width?options.width(s,c):52.5;
 const rh=(s,r)=>options.height?options.height(s,r):12.75;
 const sum=(fn,s,n)=>{let v=0;for(let i=0;i<n;i++)v+=fn(s,i);return v;};
 global.fetch=async(url,init)=>{const data=JSON.parse(init.body);posts.push({url,data});if(url==='/client-event')events.push(data);
   if(url==='/image-batch')return{ok:true,json:async()=>({captures:data.images.map(x=>{const p=S.png(x.base64);return{id:x.id,sheet:x.sheet,address:x.address,relative_png:'mock/'+x.id+'.png',png_width:p.width,png_height:p.height,sha256:'a'.repeat(64)}})})};
   return{ok:true,json:async()=>({ok:true})};};
 global.Excel={run:async fn=>{const queue=[];
  function range(s,r){return{load(p){queries.push({sheet:s,rect:r,props:p})},
    get top(){return sum(rh,s,r.r0)},get left(){return sum(cw,s,r.c0)},
    get width(){return sum(cw,s,r.c1)-sum(cw,s,r.c0)},get height(){return sum(rh,s,r.r1)-sum(rh,s,r.r0)},
    get columnHidden(){return cw(s,r.c0)===0},get rowHidden(){return rh(s,r.r0)===0},
    format:{load(){},get rowHeight(){return rh(s,r.r0)}},
    getImage(){const out={};queue.push(()=>{images.push({sheet:s,rect:r});if(imageFail)throw new Error('mock renderer');
      out.value=png(Math.round((sum(cw,s,r.c1)-sum(cw,s,r.c0))*4/3)+1,Math.round((sum(rh,s,r.r1)-sum(rh,s,r.r0))*4/3)+1)});return out;}};}
  return fn({workbook:{worksheets:{getItem(name){sheet=name;return{name,load(){if(options.healthFail)throw new Error('health');},getRangeByIndexes:(r,c,n,m)=>range(name,{r0:r,r1:r+n,c0:c,c1:c+m}),getRange:a=>range(name,S.parse(a))}}}},trace(){},sync:async()=>{while(queue.length)queue.shift()();}});
 }};
 return{events,images,queries,posts,setImageFail(v){imageFail=v;}};
}
let n=0;async function test(name,fn){await fn();n++;console.log('PASS '+name)}
(async()=>{
 await test('zero-width and too-wide controls have explicit, different rejection reasons',()=>{
   assert.equal(S.controlReason({left:0,right:0,top:0,bottom:20}),'zero-visible-width');
   assert.equal(S.controlReason({left:0,right:1200,top:0,bottom:20}),'bootstrap-edge-budget');
   assert.equal(S.controlReason({left:0,right:500,top:0,bottom:200}),'bootstrap-area-budget');
 });
 await test('hidden A plus wide C/D can recover a visible control pool without widening/unhiding',async()=>{
   const m=mock({width:(s,c)=>[0,39.75,504.75,312,222.75][c]??60});const s=new S.Session('kern.xlsx');
   await s.ensureCalibration('71717-50',S.parse('A1:O865'));
   const c=s.state.calibrations[0];assert.equal(c.status,'complete');
   assert.ok(c.control_search.length>1);assert.notEqual(c.control_selection,'original-origin');
   assert.equal(c.control_search[0].examined.find(x=>x.address==='A1:A1').reason,'zero-visible-width');
   assert.equal(s.state.geometry_diagnostics[0].columns.find(x=>x.column==='A').hidden,true);
   assert.ok(m.images.every(x=>x.rect.c1>x.rect.c0 && x.rect.r1>x.rect.r0));
   assert.equal(s.limitsFor('71717-50').scale_x,4/3);
 });
 await test('narrow columns are not mislabeled hidden; search skips the insufficient span',async()=>{
   mock({width:(s,c)=>c<16?0.01:52.5});const s=new S.Session('narrow');
   await s.ensureCalibration('S',S.parse('A1:D20'));
   assert.equal(s.state.geometry_diagnostics[0].columns[0].hidden,false);
   assert.equal(s.state.calibrations[0].status,'complete');
 });
 await test('a hidden prefix longer than the old sixteen-column window is handled by offset controls',async()=>{
   mock({width:(s,c)=>c<32?0:52.5});const s=new S.Session('hidden-prefix');
   await s.ensureCalibration('S',S.parse('A1:AZ80'));
   assert.equal(s.state.calibrations[0].control_search.length,3);
   assert.equal(s.state.calibrations[0].status,'complete');
 });
 await test('orthogonal controls span both axes without requiring each image to vary on both',()=>{
   const c=[[10,10],[20,10],[30,10],[10,20],[10,30]].map(([x,y],i)=>({rect:{r0:i,r1:i+1,c0:0,c1:1},geometry:{left:0,right:x,top:0,bottom:y}}));
   assert.throws(()=>S.chooseControls(c));const selected=S.independentControls(c);
   assert.equal(selected.length,6);
   for(const axis of ['x','y'])assert.ok(S.fitAxis(selected.slice(0,-1).map(x=>({...x,png:{width:x.geometry.right*2+1,height:x.geometry.bottom*2+1}})),axis).scale===2);
 });
 await test('exhausted bounded search fails honestly and does not make a zero-sized PNG call',async()=>{
   const m=mock({width:()=>0});const s=new S.Session('invisible');
   await assert.rejects(s.ensureCalibration('S',S.parse('A1:D8')),e=>e.code==='CALIBRATION_NO_SAFE_CONTROLS');
   assert.equal(m.images.length,0);assert.equal(s.state.calibrations[0].control_search.length,3);
   assert.equal(s.state.calibrations[0].status,'failed');
 });
 await test('sheet-local model failure is preserved while a healthy session continues',async()=>{
   const m=mock();const s=new S.Session('x');const e=S.fault('CALIBRATION_MODEL_REJECTED','e',{},true);
   assert.equal(await s.continueAfterSheetFailure('S',e),true);assert.equal(s.dead,false);
   assert.equal(s.state.sheet_continuations[0].error.code,e.code);assert.ok(m.events.some(x=>x.stage==='sheet_failure_isolated'));
   assert.equal(m.images.length,0);
 });
 await test('fatal host/persistence failures and poisoned native sessions never continue',async()=>{
   const m=mock();const s=new S.Session('x');assert.equal(await s.continueAfterSheetFailure('S',S.fault('HOST_HTTP_ERROR','e',{},true)),false);
   s.dead=true;assert.equal(await s.continueAfterSheetFailure('S',S.fault('CALIBRATION_MODEL_REJECTED','e',{},true)),false);
   assert.equal(m.events.length,0);assert.equal(m.queries.length,0);
 });
 await test('failed health check stops the session instead of continuing blindly',async()=>{
   mock({healthFail:true});const s=new S.Session('x');assert.equal(await s.continueAfterSheetFailure('S',S.fault('PNG_GEOMETRY_MISMATCH','e',{},true)),false);assert.equal(s.dead,true);
 });
 await test('read-only geometry diagnostics have two anchors and no screenshots',async()=>{
   const m=mock({width:(s,c)=>c===0?0:52.5});const s=new S.Session('x');const d=await s.diagnoseGeometry('S','TEST',S.parse('A11:D40'));
   assert.equal(d.status,'complete');assert.ok(d.rows.some(x=>x.anchor_column==='A'));assert.ok(d.rows.some(x=>x.anchor_column==='B'));
   assert.ok(d.rows.some(x=>x.row_1based===41));assert.equal(m.images.length,0);
   const before=m.events.length;await s.diagnoseGeometry('S','TEST',S.parse('A11:D40'));assert.equal(m.events.length,before);
 });
 await test('periodic snapshots are coalesced but failures and final state are forced',async()=>{
   const m=mock();const s=new S.Session('x');await s.checkpoint('tile_render_started');await s.checkpoint('tile_complete');await s.checkpoint('tile_render_started');
   await s.checkpoint('tile_failed');await s.checkpoint('workbook_finished');
   assert.equal(m.posts.filter(x=>x.url==='/capture-progress').length,3);assert.equal(s.state.checkpoint_policy.coalesced,2);
 });
 await test('sheet selection preserves whitespace, workbook order and original catalog identity',()=>{
   const catalog=[{name:'A'},{name:'Erläuterung zu CSV-Tabellen '},{name:'B'}];
   assert.deepEqual(S.selectSheetNames(catalog,['B','Erläuterung zu CSV-Tabellen ']),['Erläuterung zu CSV-Tabellen ','B']);
   assert.throws(()=>S.selectSheetNames(catalog,['Erläuterung zu CSV-Tabellen']),e=>e.code==='SHEET_SELECTION_MISSING');
   assert.throws(()=>S.selectSheetNames(catalog,[]));assert.throws(()=>S.selectSheetNames(catalog,['B','B']));
 });
 await test('F integration completes second worksheet after first calibration failure',async()=>{
   mock();const s=new S.Session('x',{strategy:'F'});const real=s.ensureCalibration.bind(s);
   s.ensureCalibration=async(sheet,rect)=>{if(sheet==='Broken')throw S.fault('CALIBRATION_NO_SAFE_CONTROLS','fixture',{},true);return real(sheet,rect)};
   const src=fs.readFileSync(path.join(web,'modes/range-image.js'),'utf8'),cut=src.lastIndexOf('  try {\n    await execute();');
   const code=src.slice(0,cut)+`globalThis.hooks={run:renderPrecomputedPlannedBatch,set:s=>{safeSession=s}};}());`;
   const ui=new Proxy({},{get:()=>()=>{}}),ctx={window:{fetch:global.fetch},SafeCapture:S,CapturePolicy:P,Excel:global.Excel,performance,console,atob,Map,Uint8Array,Number,ExperimentUi:ui,ExperimentEvents:{post:async()=>{}}};
   vm.createContext(ctx);vm.runInContext(code,ctx);ctx.hooks.set(s);
   const items=['Broken','Good'].map((sheet,i)=>({id:'c0000'+(i+1),sheet,sheet_index:i+1,address:'A1:D5',bottom_row:4,base_right_column:3,end_column:3,overflow_probe:false,column_widths:Array(16384).fill(52.5)}));
   const result=await ctx.hooks.run('x',items,1,1,{strategy:'F'});
   assert.equal(s.state.sheets[0].status,'failed');assert.equal(s.state.sheets[1].status,'complete');assert.equal(result.captures.length,1);
   assert.equal(s.state.sheet_continuations[0].status,'healthy-continue');
 });
 console.log(`${n} visible-control/isolation tests passed (mocks, not native Excel).`);
})().catch(e=>{console.error(e);process.exitCode=1});
