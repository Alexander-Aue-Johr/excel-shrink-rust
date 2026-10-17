'use strict';
// Exact-scope diagnostic protocol tests. No native Excel renderer is exercised.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const web=path.resolve(__dirname,'../_shared/office-addin/web');
const S=require(path.join(web,'safe-capture.js'));
const source=fs.readFileSync(path.join(web,'modes/range-image.js'),'utf8');
function hooks(Excel,session){
 const cut=source.lastIndexOf('  try {\n    await execute();');assert.ok(cut>0);
 const context={Excel,SafeCapture:S,performance,console,Map,Set,window:{},ExperimentUi:{},ExperimentEvents:{post:async()=>{}}};
 vm.createContext(context);vm.runInContext(source.slice(0,cut)+'globalThis.hooks={map:mapPointBoundsToGrid,plans:precomputedPlannedItems,set:s=>{safeSession=s}};}());',context);context.hooks.set(session);return context.hooks;
}
function gridMock({ceiling=Infinity,offset=0}={}){
 const queries=[],events=[];
 const Excel={run:async fn=>fn({workbook:{worksheets:{getItem:()=>({getRangeByIndexes(r,c){
   queries.push({r,c});const top=Math.min(r*10,ceiling);return{top:top+offset,left:c*50,width:50,height:r*10>=ceiling?0:10,rowHidden:r*10>=ceiling,columnHidden:false,load(){}};
 }})}},sync:async()=>{}})};
 const session={config:{capture_options:{diagnostic_level:'detailed'}},state:{},office:async(_,__,fn)=>Excel.run(fn),event:async(stage,detail)=>events.push({stage,detail}),checkpoint:async()=>{}};
 return{Excel,session,queries,events};
}
function entry(){return{sheet:'S',rightColumn:0,bottomRow:10,contentRightColumn:0,contentBottomRow:10,shapeRightPoints:50,shapeBottomPoints:125,contentRightPoints:50,contentBottomPoints:110,rightBoundSource:'content',bottomBoundSource:'drawing',shapeDiagnostics:[{left:0,top:120,width:10,height:5}],columnWidths:Array(16384).fill(50),hasText:false};}
function png(w,h){const b=Buffer.alloc(45);Buffer.from([137,80,78,71,13,10,26,10]).copy(b);b.writeUInt32BE(13,8);b.write('IHDR',12);b.writeUInt32BE(w,16);b.writeUInt32BE(h,20);b.write('IEND',37);return b.toString('base64');}
let count=0;async function test(name,fn){await fn();count++;console.log('PASS '+name)}
(async()=>{
 await test('native-call timeout parameter is validated and reaches session limits',()=>{
  assert.equal(new S.Session('x',{capture_options:{native_call_timeout_ms:150000}}).limits.timeout_ms,150000);
  for(const ms of [1,9999,600001,NaN])assert.throws(()=>new S.Session('x',{capture_options:{native_call_timeout_ms:ms}}));
 });
 await test('empty drawing map issues no native call',async()=>{
  const m=gridMock();await hooks(m.Excel,m.session).map([]);assert.equal(m.queries.length,0);
 });
 await test('successful drawing boundary verifies final point and predecessor',async()=>{
  const m=gridMock(),e=entry();await hooks(m.Excel,m.session).map([e]);assert.equal(e.bottomRow,12);assert.equal(e.mappingError,undefined);
  const audit=m.session.state.drawing_mapping_audits[0];assert.equal(audit.status,'verified-native-bracket');assert.equal(audit.target_points,125);assert.equal(audit.final_checks.length,2);assert.ok(audit.probes.length<=20);
 });
 await test('collapsed hidden tail cannot be accepted as row 1048576',async()=>{
  const m=gridMock({ceiling:110}),e=entry();await hooks(m.Excel,m.session).map([e]);assert.equal(e.bottomRow,10);assert.equal(e.mappingError.code,'DRAWING_BOUND_UNRESOLVED');
  const audit=m.session.state.drawing_mapping_audits[0];assert.equal(audit.status,'unresolved');assert.equal(audit.selected_index_0based,1048575);assert.ok(audit.final_checks.every(x=>x.edge_points===110));
 });
 await test('failed mapping is projected as an explicit per-sheet planning error',async()=>{
  const m=gridMock({ceiling:110}),e=entry(),h=hooks(m.Excel,m.session);await h.map([e]);const p=h.plans([e])[0];assert.equal(p.planning_error.code,'DRAWING_BOUND_UNRESOLVED');assert.equal(p.bottom_row,10);
  assert.ok(source.includes("if(item.planning_error)"));
 });
 await test('drawing-mapping error is eligible only for verified healthy continuation',()=>{
  assert.ok(S.isSheetLocalFailure?S.isSheetLocalFailure(S.fault('DRAWING_BOUND_UNRESOLVED','x')):fs.readFileSync(path.join(web,'safe-capture.js'),'utf8').includes("'DRAWING_BOUND_UNRESOLVED'"));
 });
 await test('hidden-tail policy is not overridden by a drawing search',async()=>{
  const m=gridMock(),e=entry();e.bottomBoundSource='trailing-hidden';await hooks(m.Excel,m.session).map([e]);assert.equal(m.queries.length,0);assert.equal(e.bottomRow,10);
 });
 await test('fixed-width control evidence does not promote calibration to success',async()=>{
  const s=new S.Session('fixed',{capture_options:{diagnostic_level:'detailed'}}),events=[],record={id:'cal0001',status:'failed'};
  const controls=[10,20,30,40].map((h,i)=>({rect:{r0:0,r1:i+1,c0:0,c1:1},geometry:{left:0,right:675.75,top:0,bottom:h}}));
  let geometry=null,capture=0;
  s.geometries=async(_,rects)=>rects.map(rect=>{geometry=controls.find(x=>x.rect.r1===rect.r1).geometry;return geometry});
  s.office=async()=>{capture++;return png(Math.round(geometry.right*4/3)+1,Math.round(geometry.bottom*4/3)+1)};
  s.http=async(_,body)=>({captures:[{relative_png:'fixed/'+body.images[0].id+'.png'}]});s.event=async(stage,data)=>events.push({stage,data});s.checkpoint=async()=>{};
  await s.diagnoseDegenerateControls('Qualitätsbericht',controls,record);
  assert.equal(capture,4);assert.equal(record.status,'failed');assert.equal(record.degenerate_diagnostic.unique_widths,1);assert.equal(record.degenerate_diagnostic.status,'evidence-recorded');
  assert.equal(s.calibrations.size,0);assert.equal(record.degenerate_diagnostic.observations.length,4);
 });
 await test('diagnostic image receipt failure stays recorded and never marks calibration complete',async()=>{
  const s=new S.Session('fixed'),r={id:'cal0001',status:'failed'};const controls=[10,20,30].map((h,i)=>({rect:{r0:0,r1:i+1,c0:0,c1:1},geometry:{left:0,right:20,top:0,bottom:h}}));
  s.geometries=async()=>[controls[0].geometry];s.office=async()=>png(28,15);s.http=async()=>({});s.event=async()=>{};s.checkpoint=async()=>{};
  await s.diagnoseDegenerateControls('S',controls,r);assert.equal(r.status,'failed');assert.equal(r.degenerate_diagnostic.error.code,'PERSIST_NO_RECEIPT');
 });
 await test('focused view changes are restricted to selected sheets',()=>{
  assert.ok(source.includes('CapturePolicy.hideGridlines(safeSession, workbookInfo.sheets.filter(sheet=>sheetNames.includes(sheet.name)))'));
 });
 console.log(count+' focused diagnostic tests passed (protocol mocks).');
})().catch(e=>{console.error(e);process.exitCode=1});
