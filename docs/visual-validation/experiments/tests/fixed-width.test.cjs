'use strict';
// Deterministic Office/Canvas protocol model, not a native Excel validation.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const {mock,S,G,P,V,web}=require('./fixed-width-model.cjs');
let passed=0;
async function test(label,fn){await fn();passed++;console.log('PASS '+label);}
function fixed(options={}){return mock({width:c=>c===0?675.75:0,zeroNotHidden:true,...options});}
(async()=>{
 for(const width of [675.75,712.5,791.25])for(const factor of [4/3,2]) {
  await test(`full-span controls calibrate Y, not X (${width} pt, factor ${factor})`,async()=>{
   fixed({width:c=>c===0?width:0,sx:factor,sy:factor});const s=new S.Session('x');
   const cal=await s.ensureCalibration('S',S.parse('A1:A176'));
   assert.equal(cal.x.mode,'fixed-span');assert.equal(cal.x.scale,null);
   assert.equal(cal.x.width_px,Math.round(width*factor)+1);assert.equal(cal.y.scale,factor);
   assert.equal(cal.observations.length,4);assert.ok(cal.observations.every(o=>o.rect.c0===0&&o.rect.c1===1));
   assert.ok(cal.observations.every(o=>o.png.width===cal.x.width_px));
   assert.equal(s.axisGeometry.data('S').columns[1].hidden,false);
   assert.throws(()=>S.axisScale(s.limitsFor('S'),'x'),e=>e.code==='UNCALIBRATED_CAPTURE');
  });
 }
 await test('X and Y may differ; fixed width must not reuse Y as X',async()=>{
  fixed({sx:2,sy:4/3});const s=new S.Session('x');const cal=await s.ensureCalibration('S',S.parse('A1:A80'));
  assert.equal(cal.x.width_px,1353);assert.equal(cal.y.scale,4/3);assert.equal(cal.x.scale,null);
 });
 await test('fixed span may start after a zero-width prefix without unhiding it',async()=>{
  fixed({width:c=>c===2?675.75:0});const s=new S.Session('x');const cal=await s.ensureCalibration('S',S.parse('A1:C80'));
  assert.equal(cal.x.mode,'fixed-span');assert.equal(cal.x.c0,0);assert.equal(cal.x.c1,3);
  assert.equal(s.axisGeometry.data('S').columns[0].size,0);
 });
 await test('different PNG width in a fitting control is an error',async()=>{
  fixed({adjustWidth:(r,n)=>n===2?1:0});const s=new S.Session('x');
  await assert.rejects(s.ensureCalibration('S',S.parse('A1:A80')),e=>e.code==='FIXED_WIDTH_PNG_MISMATCH');
  assert.equal(s.calibrations.size,0);assert.equal(s.state.calibrations[0].status,'failed');
 });
 await test('different holdout PNG width is an error',async()=>{
  fixed({adjustWidth:(r,n)=>n===4?1:0});const s=new S.Session('x');
  await assert.rejects(s.ensureCalibration('S',S.parse('A1:A80')),e=>e.code==='FIXED_WIDTH_PNG_MISMATCH');
 });
 await test('changed physical horizontal interval is rejected before image capture',async()=>{
  const m=fixed({width:c=>c===0?675.75:c===1000?10:0});const s=new S.Session('x');
  await s.ensureCalibration('S',S.parse('A1:A80'));const n=m.rendered.length;
  await assert.rejects(s.plan('S',S.parse('A1:ALM80')),e=>['FIXED_WIDTH_SPAN_CHANGED','FIXED_WIDTH_HORIZONTAL_OPERATION'].includes(e.code));
  assert.equal(m.rendered.length,n);
 });
 await test('a single-height pool cannot be promoted to a Y calibration',()=>{
  const g={left:0,right:200,top:0,bottom:12};const c={rect:S.parse('A1:A1'),geometry:g};
  assert.equal(S.fixedWidthControls([c,c,c],c.rect,g),null);
 });
 await test('one narrow control cannot calibrate a different requested width',()=>{
  const pool=[12,24,48].map(h=>({geometry:{left:0,right:10,top:0,bottom:h},rect:S.parse('A1:A2')}));
  assert.equal(S.fixedWidthControls(pool,S.parse('A1:B4'),{left:0,right:20,top:0,bottom:48}),null);
 });
 await test('fixed pixel prediction rejects translations and unequal extents',()=>{
  const limits={...S.DEFAULTS,scale_y:4/3,fixed_x:{left:0,right:675.75,width_px:902}};
  assert.deepEqual(S.pixels({left:0,right:675.75,top:0,bottom:12.75},limits),{width:902,height:18});
  assert.throws(()=>S.pixels({left:10,right:685.75,top:0,bottom:12.75},limits),e=>e.code==='FIXED_WIDTH_SPAN_CHANGED');
  assert.throws(()=>S.xOffset(0,10,limits),e=>e.code==='FIXED_WIDTH_HORIZONTAL_OPERATION');
 });
 await test('context budget packs only full-width row strips',async()=>{
  fixed();const s=new S.Session('x');s.limits.max_pixels=50000;const root=S.parse('A1:A80');
  const tiles=await s.plan('S',root);assert.ok(tiles.length>2);
  for(const t of tiles){assert.equal(t.rect.c0,0);assert.equal(t.rect.c1,1);assert.equal(t.predicted.width,902);assert.ok(t.predicted.width*t.predicted.height<=50000);}
  for(let i=1;i<tiles.length;i++)assert.ok(tiles[i].capture_rect.r0<tiles[i].rect.r0);
 });
 await test('fixed width preserves all pixels during context cropping and exact composition',async()=>{
  fixed();const s=new S.Session('x',{capture_options:{context_validation:'first-seam'}});s.limits.max_pixels=60000;
  const root=S.parse('A1:A80'),tiles=await s.plan('S',root);
  const images=await s.capturePlan('S',tiles,'c00001');
  assert.equal(s.state.context_validations[0].status,'complete');
  assert.equal(s.state.context_validations[0].full_comparison.different_pixels,0);
  for(const im of images){assert.equal(im.png_width,902);assert.equal(im.core_box_px.width,902);assert.equal(im.core_box_px.x,0);}
 });
 await test('horizontal recovery splitting is not permitted even with extra zero-width columns',()=>{
  const l={...S.DEFAULTS,scale_y:4/3,fixed_x:{left:0,right:600,width_px:801}};
  assert.throws(()=>S.split(S.parse('A1:D1'),{left:0,right:600,top:0,bottom:100000},l),e=>e.code==='FIXED_WIDTH_HORIZONTAL_OPERATION');
 });
 await test('native export cannot silently narrow a fixed width',async()=>{
  const m=fixed(),s=new S.Session('x');const tile={id:'one',png_width:902,png_height:18,core_box_px:{x:0,y:0,width:902,height:18},raster_profile:{fixed_x:{width_px:902}}};
  await assert.rejects(P.exportTile(s,tile,800,'A1:A1'),e=>e.code==='FIXED_WIDTH_HORIZONTAL_OPERATION');assert.equal(m.posts.length,0);
 });
 await test('no pixel-width extrapolation for a requested right overflow probe',async()=>{
  fixed();const s=new S.Session('x');await s.ensureCalibration('S',S.parse('A1:A80'));
  await assert.rejects(P.probeBuffer(s,{sheet:'S',address:'A1:A80',base_right_column:0,column_widths:Array(16384).fill(0)},1920),e=>e.code==='FIXED_WIDTH_HORIZONTAL_OPERATION');
 });
 await test('regular captures do not accept a one-pixel fixed-width drift',async()=>{
  fixed({adjustWidth:(r,n)=>n>4?1:0});const s=new S.Session('x');const plan=await s.plan('S',S.parse('A1:A20'));
  await assert.rejects(s.capturePlan('S',plan,'c00001'),e=>e.code==='FIXED_WIDTH_PNG_MISMATCH');
  assert.ok(s.state.attempts[0].relative_png);assert.equal(s.state.captures.length,0);
 });
 await test('normal two-axis calibration remains the preferred unchanged path',async()=>{
  mock();const s=new S.Session('x');const c=await s.ensureCalibration('S',S.parse('A1:H50'));
  assert.equal(c.x.scale,4/3);assert.equal(c.fixed_span,undefined);assert.equal(s.limitsFor('S').fixed_x,null);
 });
 await test('fixed-span failures may be isolated after the existing health check',async()=>{
  fixed();const s=new S.Session('x');assert.equal(await s.continueAfterSheetFailure('S',S.fault('FIXED_WIDTH_PNG_MISMATCH','test',{},true)),true);
 });
 await test('Strategy F uses full width in final export and zero horizontal offset',async()=>{
  const m=fixed(),s=new S.Session('x',{strategy:'F'});
  const source=fs.readFileSync(path.join(web,'modes/range-image.js'),'utf8');const cut=source.lastIndexOf('  try {\n    await execute();');assert.ok(cut>0);
  const code=source.slice(0,cut)+`globalThis.hooks={run:renderPrecomputedPlannedBatch,set:s=>{safeSession=s}};}());`;
  const ctx={window:{fetch:global.fetch},SafeCapture:S,CapturePolicy:P,Excel:global.Excel,performance,console,atob,Map,Uint8Array,Number,ExperimentUi:new Proxy({},{get:()=>()=>{}}),ExperimentEvents:{post:async()=>{}}};
  vm.createContext(ctx);vm.runInContext(code,ctx);ctx.hooks.set(s);
  const result=await ctx.hooks.run('x',[{id:'c00001',sheet:'S',sheet_index:3,address:'A1:A176',bottom_row:175,base_right_column:0,end_column:0,overflow_probe:false,column_widths:[675.75,...Array(16383).fill(0)]}],1,1,{strategy:'F'});
  assert.equal(s.state.sheets[0].status,'complete');assert.equal(result.captures.length,1);const c=result.captures[0];
  assert.equal(c.png_width,902);assert.equal(c.raster_profile.scale_x,null);assert.equal(c.sheet_offset_px.x,0);assert.equal(c.address,'A1:A176');
 });
 console.log(`${passed} fixed-width tests passed (renderer/protocol model, not native Excel).`);
})().catch(e=>{console.error(e);process.exitCode=1});
