'use strict';
// Pure policy / orchestration tests. Real Excel view rendering and native PNG codecs require Windows.
const assert=require('node:assert/strict'),path=require('node:path'),fs=require('node:fs');
const web=path.resolve(__dirname,'../_shared/office-addin/web');
global.SafeCapture=require(path.join(web,'safe-capture.js'));
const P=require(path.join(web,'capture-policy.js'));
const S=global.SafeCapture;
let tests=0;
async function test(name,action){await action();tests++;console.log('ok '+tests+' - '+name);}
function session(extra={}){
 const log=[];return Object.assign({workbook:'W.xlsx',state:{},timings:{},log,
 checkpoint:async(stage,detail)=>log.push({kind:'checkpoint',stage,detail}),event:async(stage,detail)=>log.push({kind:'event',stage,detail})},extra);
}
function gridSession(values,{reject=false,fail=false}={}){
 const log=[],refs=Object.entries(values).map(([name,start])=>{
  let value=start;return {name,get showGridlines(){return value;},set showGridlines(v){log.push('set:'+name+':'+v);if(!reject)value=v;},
    load(fields){log.push('load:'+name+':'+fields);},get initial(){return start;}};
 });
 const s=session({office:async(stage,detail,action)=>{log.push('office:'+stage);if(fail)throw new Error('native error');
   return action({workbook:{worksheets:{getItem:name=>refs.find(r=>r.name===name)}},sync:async()=>{log.push('sync');}});}});
 s.gridlog=log;s.refs=refs;return s;
}
const base={complete:true,checked:true,base:5,end:20,probe:true};
(async()=>{
 await test('known no-overflow removes only the probe margin',()=>assert.deepEqual(P.outputPolicy(base),{crop:true,end:5,reason:'no-overflow-remove-buffer'}));
 await test('positive evidence retains the entire captured buffer, not just the last ink column',()=>{
  assert.deepEqual(P.outputPolicy({...base,detected:true}),{crop:false,end:20,reason:'overflow-detected-retain-buffer'});
 });
 await test('unresolved, unchecked and no-probe states never authorize cropping',()=>{
  for(const flags of [{complete:false},{checked:false},{probe:false},{complete:false,detected:true}]){
   const p=P.outputPolicy({...base,...flags});assert.equal(p.crop,false);assert.equal(p.end,20);
  }
 });
 await test('invalid rectangle policy is rejected',()=>{
  for(const v of [{base:-1},{end:3},{end:3.2}])assert.throws(()=>P.outputPolicy({...base,...v}),/Invalid base/);
 });
 await test('gridlines are changed in one batch and then verified before capture',async()=>{
  const s=gridSession({' First':true,Second:false});const catalog=[{name:' First',sheet_index:1,show_gridlines:true},{name:'Second',sheet_index:2,show_gridlines:false}];
  await P.hideGridlines(s,catalog);
  assert.deepEqual(s.gridlog,['office:capture_view','set: First:false','set:Second:false','sync','load: First:name,showGridlines','load:Second:name,showGridlines','sync']);
  assert.equal(s.state.capture_view.status,'confirmed');assert.equal(catalog[0].show_gridlines,false);assert.equal(catalog[0].original_show_gridlines,true);
  assert.equal(s.log[0].stage,'capture_view_pending');assert.equal(s.state.capture_view.before[0].show_gridlines,true);assert.doesNotThrow(()=>JSON.stringify(s.state));
 });
 await test('unconfirmed setting is fatal, not silently used',async()=>{
  const s=gridSession({S:true},{reject:true});await assert.rejects(P.hideGridlines(s,[{name:'S',show_gridlines:true}]),e=>e.code==='GRIDLINES_NOT_DISABLED'&&e.fatal);
  assert.equal(s.state.capture_view.status,'failed');assert.equal(s.state.capture_view.before[0].show_gridlines,true);assert.doesNotThrow(()=>JSON.stringify(s.state));
 });
 await test('view-setting native error is retained and aborts',async()=>{
  const s=gridSession({S:true},{fail:true});await assert.rejects(P.hideGridlines(s,[{name:'S',show_gridlines:true}]),e=>e.fatal===true);
  assert.equal(s.state.capture_view.status,'failed');assert.ok(s.state.capture_view.error);
 });
 for(const scale of [4/3,2])await test(`pixel buffer uses measured scale ${scale} and skips hidden columns`,async()=>{
  const widths=Array(16384).fill(75);widths[3]=0;let called=0;
  const s=session({ensureCalibration:async()=>called++,limitsFor:()=>({scale_x:scale}),geometries:async(name,rects)=>rects.map(r=>({left:widths.slice(0,r.c0).reduce((a,b)=>a+b,0),right:widths.slice(0,r.c1).reduce((a,b)=>a+b,0)}))});
  const p=await P.probeBuffer(s,{sheet:'S',address:'A1:J100',base_right_column:1,column_widths:widths},1920);
  assert.equal(called,1);assert.ok(p.actual_pixels>=1920);assert.ok(p.actual_pixels-75*scale<1920);assert.equal(p.limited,false);assert.equal(p.end_column,1+p.extra_visible_columns+1);
 });
 await test('probe budget cap is reported rather than widening without bound',async()=>{
  const s=session({ensureCalibration:async()=>{},limitsFor:()=>({scale_x:1}),geometries:async(n,rs)=>rs.map(r=>({left:r.c0,right:r.c1}))});
  const p=await P.probeBuffer(s,{sheet:'S',address:'A1:B1',base_right_column:0,column_widths:Array(16384).fill(1)},1920);
  assert.equal(p.limited,true);assert.equal(p.extra_visible_columns,64);assert.equal(p.actual_pixels,64);
 });
 await test('no visible probe columns is a recorded terminal case',async()=>{
  const s=session({ensureCalibration:async()=>{},limitsFor:()=>({scale_x:2}),geometries:async()=>{throw Error('must not query');}});
  const p=await P.probeBuffer(s,{sheet:'S',address:'A1:B1',base_right_column:1,column_widths:Array(16384).fill(0)},1920);
  assert.equal(p.end_column,1);assert.equal(p.extra_visible_columns,0);assert.equal(p.limited,true);
 });
 const tile={id:'c001-p1-t00001',sheet:'S',address:'A10:J20',png_width:100,png_height:50,sha256:'a'.repeat(64),relative_png:'W.xlsx/c001-p1-t00001.png'};
 await test('retained PNG needs no extra upload/crop request',async()=>{
  const s=session({http:async()=>{throw Error('not needed');}});const result=await P.exportTile(s,tile,100,'A10:J20');
  assert.equal(result.relative_png,tile.relative_png);assert.equal(result.export_kind,'retained-original');assert.equal(s.log.length,0);
 });
 await test('native export keeps immutable source evidence and consumes host hash/dimensions',async()=>{
  const original=JSON.stringify(tile);let request;
  const s=session({http:async(url,payload)=>{request={url,payload};assert.equal(s.log[0].stage,'final_png_pending');
   return {crop_ms:3,capture:{id:tile.id+'-final',relative_png:'W.xlsx/final.png',png_width:30,png_height:50,png_bytes:21,sha256:'b'.repeat(64)}};}});
  const result=await P.exportTile(s,tile,30,'A10:C20');assert.equal(request.url,'/finalize-image');assert.equal(request.payload.source_sha256,tile.sha256);
  assert.equal(request.payload.address,'A10:C20');assert.equal(result.raw_png,tile.relative_png);assert.equal(result.capture_address,'A10:J20');
  assert.equal(result.png_width,30);assert.equal(result.raw_png_width,100);assert.equal(result.sha256,'b'.repeat(64));assert.equal(JSON.stringify(tile),original);
  assert.equal(s.state.final_exports[0].status,'complete');assert.equal(s.log.at(-1).stage,'final_png_finished');assert.ok(s.timings.finalize_ms>=0);
 });
 await test('export failure leaves source intact and persists failure phase',async()=>{
  const s=session({http:async()=>{throw Error('encoder failed');}});await assert.rejects(P.exportTile(s,tile,30,'A10:C20'),/encoder failed/);
  assert.equal(s.state.final_exports[0].status,'failed');assert.equal(tile.png_width,100);assert.equal(s.log.at(-1).stage,'final_png_finished');
 });
 await test('mismatched native crop receipt is fatal',async()=>{
  const s=session({http:async()=>({capture:{png_width:50,png_height:10}})});await assert.rejects(P.exportTile(s,tile,30,'A10:C20'),e=>e.code==='FINAL_PNG_RECEIPT_MISMATCH'&&e.fatal);assert.doesNotThrow(()=>JSON.stringify(s.state));
 });
 await test('invalid crop width rejected before any work',async()=>{
  for(const width of [0,101,1.2]){const s=session();await assert.rejects(P.exportTile(s,tile,width,'A10:C20'));assert.equal(s.log.length,0);}
 });
 await test('range-image applies view policy before planning while control fixtures stay untouched',()=>{
  const code=fs.readFileSync(path.join(web,'modes/range-image.js'),'utf8');
  assert.ok(code.indexOf('CapturePolicy.hideGridlines')<code.indexOf("safeSession.checkpoint('planning_started'"));
  for(const mode of ['range-image-calibration.js','range-image-height-limit.js'])assert.equal(fs.readFileSync(path.join(web,'modes',mode),'utf8').includes('hideGridlines'),false);
  assert.ok(fs.readFileSync(path.join(web,'shell.js'),'utf8').includes("await load('/capture-policy.js?run=' + run)"));
 });
 console.log(`${tests} capture policy tests passed. Native rendering and codec tests require Windows.`);
})().catch(e=>{console.error(e);process.exitCode=1;});
