'use strict';
// Protocol/render mocks only. PNGs below are synthetic header fixtures, not Excel samples.
const assert=require('node:assert/strict'),path=require('node:path');
const web=path.join(__dirname,'../_shared/office-addin/web');
const S=require(path.join(web,'safe-capture.js'));global.SafeCapture=S;
const X=require(path.join(web,'selective-overflow.js'));
let count=0;async function test(name,fn){await fn();count++;console.log('PASS '+name);}
function png(w,h,key){const b=Buffer.alloc(48);Buffer.from([137,80,78,71,13,10,26,10]).copy(b);b.writeUInt32BE(13,8);b.write('IHDR',12);b.writeUInt32BE(w,16);b.writeUInt32BE(h,20);b.writeUInt32BE(key,29);b.write('IEND',40);return b.toString('base64');}
const white=(w,h)=>{const data=new Uint8ClampedArray(w*h*4);data.fill(255);return{width:w,height:h,data};};
function mark(im,x,y,c=0){const o=(y*im.width+x)*4;im.data[o]=im.data[o+1]=im.data[o+2]=c;}
function fixture(options={}){
 let counter=0,mode='before';const registry=new Map(),values=new Map(),phases=[],saved=[],writes=[];
 const store=im=>{const b=png(im.width,im.height,++counter);registry.set(b,im);return b;};
 const geometry=r=>({left:r.c0*10,right:r.c1*10,top:r.r0*10,bottom:r.r1*10});
 function raster(r){const im=white((r.c1-r.c0)*10+1,(r.r1-r.r0)*10+1);for(let x=21;x<im.width-1;x++)mark(im,x,5,160);
   if(options.text&&mode!=='blocked')for(let y=7;y<9;y++)for(let x=23;x<30;x++)mark(im,x,y,0);
   if(options.nonlocal&&mode==='blocked')mark(im,3,3);
   if(options.unstable&&mode==='restored')mark(im,25,8);
   return im;
 }
 global.Image=class {set src(v){if(!v)return;this.im=registry.get(v.slice(v.indexOf(',')+1));if(!this.im)throw Error('unknown image');queueMicrotask(()=>this.onload());}};
 global.document={createElement(){const c={width:0,height:0,data:null,getContext(){return{drawImage(img){c.data=img.im.data.slice();},getImageData(){return{data:c.data}},createImageData(w,h){return{data:new Uint8ClampedArray(w*h*4)}},putImageData(im){c.data=im.data.slice();}}},toDataURL(){return'data:image/png;base64,'+store({width:c.width,height:c.height,data:c.data});}};return c;}};
 const cell=r=>({load(){},get values(){return[[values.get(r.r0)??'']]},set values(v){writes.push({rect:r,values:v});for(let i=0;i<v.length;i++)values.set(r.r0+i,v[i][0]);mode=v[0][0]===' '?'blocked':'restored';},get formulas(){return[[values.get(r.r0)??'']]},valueTypes:[['Empty']],hasSpill:false,
   ...geometry(r),get height(){return geometry(r).bottom-geometry(r).top},get width(){return geometry(r).right-geometry(r).left},
   format:{load(){},textOrientation:options.rotated?45:0},getMergedAreasOrNullObject(){return{load(){},isNullObject:!options.merged,areas:{items:options.merged?[{rowIndex:r.r0,rowCount:2}]:[]}}},getTables(){return{load(){},items:options.table?[{}]:[]}},getPivotTables(){return{load(){},items:[]}},
   getImage(){return{value:store(raster(r))};}});
 const ws={getRangeByIndexes:(r,c,n,m)=>cell({r0:r,r1:r+n,c0:c,c1:c+m}),getRange:a=>cell(S.parse(a)),protection:{load(){},protected:!!options.protected}};
 const context={workbook:{worksheets:{getItem:()=>ws}},sync:async()=>{}};
 const session={workbook:'mock.xlsx',dead:false,state:{sparse_overflow_checks:[]},timings:{persist_ms:0},
  limitsFor(){return{...S.DEFAULTS,scale_x:1,scale_y:1,border_px:1,max_pixels:16000000,max_edge_px:30000}},
  geometries:async(sh,rs)=>rs.map(geometry),event:async(stage)=>{phases.push(stage)},checkpoint:async(stage)=>{phases.push(stage)},
  http:async(url,body)=>{if(url!=='/image-batch')return{};saved.push(body.images[0]);return{captures:body.images.map(im=>({id:im.id,relative_png:'mock/'+im.id+'.png',...S.png(im.base64)}))}},
  office:async(stage,detail,fn)=>{phases.push(stage);if(options.hang&&stage==='sparse_render'&&detail.role==='blocked'){session.dead=true;throw S.fault('NATIVE_TIMEOUT','mock timeout',{},true)}
    if(options.failWrite&&stage==='sparse_write'){await fn(context);throw S.fault('GeneralException','write partially applied')}
    if(options.failBlocked&&stage==='sparse_render'&&detail.role==='blocked')throw S.fault('GeneralException','render failed');
    return fn(context);
  }};
 const tile={id:'t1',sheet:'Sheet1',rect:{r0:0,r1:2,c0:0,c1:5},geometry:geometry({r0:0,r1:2,c0:0,c1:5}),raster_profile:{scale_x:1,scale_y:1,border_px:1}};
 return{session,tile,phases,saved,values,writes,original:raster(tile.rect)};
}
(async()=>{
 await test('raw candidate bands include thin rules before A filtering',()=>{const im=white(51,21);mark(im,24,5,160);mark(im,24,6,160);const c=X.scanCandidates(im,[0,10,20,30,40,50],1,4,[255,255,255]);assert.deepEqual(c.pixel_bands,[[5,6]]);});
 await test('sparse rows coalesce without inserting unrelated rows',()=>assert.deepEqual(X.groupedRows([1,2,5,6,7],2),[[1,2],[5,6],[7]]));
 await test('blank/formula/spill/merged/table guards fail closed',()=>{const c={value_type:'Empty',value:'',formula:'',has_spill:false,merged:false,table_count:0,pivot_count:0};assert.ok(X.guardsOk(c));for(const patch of [{formula:'=""'},{value:'x'},{has_spill:null},{merged:true},{table_count:1},{pivot_count:1},{value_type:'String'}])assert.equal(X.guardsOk({...c,...patch}),false);});
 await test('pixel y bands map through measured row tops, not fixed row height',async()=>{const heights=[10,0,20,10],tops=[0,10,10,30,40],session={geometries:async(s,rs)=>rs.map(r=>({top:tops[r.r0]}))};const tile={rect:{r0:0,r1:4,c0:0,c1:1},geometry:{top:0,bottom:40},raster_profile:{scale_y:2}};const rows=await X.pixelRowsToSheetRows(session,'s',tile,[[40,42]]);assert.deepEqual(rows,[2]);});
 await test('an unchanged line is a verified false positive for text overflow',async()=>{const f=fixture();const c=await X.verifyGroup(f.session,'Sheet1',f.tile,[0],2,[0,10,20,30,40,50],f.original);assert.equal(c.verdict,'false-positive-for-text-overflow');assert.equal(c.restoration,'verified');assert.equal(f.values.get(0),'');assert.deepEqual(c.artifacts.map(a=>a.role),['before','blocked','restored','xor']);assert.equal(f.writes[0].rect.r1-f.writes[0].rect.r0,1);});
 await test('text plus a line remains a blocker-sensitive signal',async()=>{const f=fixture({text:true});const c=await X.verifyGroup(f.session,'Sheet1',f.tile,[0],2,[0,10,20,30,40,50],f.original);assert.equal(c.verdict,'blocker-sensitive-signal');assert.equal(c.difference.changed_pixels,14);assert.equal(c.difference.outside_changed_pixels,0);});
 await test('nonlocal changes cannot certify overflow or no overflow',async()=>{const f=fixture({nonlocal:true});const c=await X.verifyGroup(f.session,'Sheet1',f.tile,[0],2,[0,10,20,30,40,50],f.original);assert.equal(c.status,'inconclusive');assert.equal(c.error.code,'SPARSE_NONLOCAL_CHANGE');assert.equal(f.values.get(0),'');});
 await test('changed restored image is inconclusive',async()=>{const f=fixture({unstable:true});const c=await X.verifyGroup(f.session,'Sheet1',f.tile,[0],2,[0,10,20,30,40,50],f.original);assert.equal(c.status,'inconclusive');assert.equal(c.error.code,'SPARSE_UNSTABLE_ROUNDTRIP');});
 await test('a blocked capture failure still clears the exact targets',async()=>{const f=fixture({failBlocked:true});const c=await X.verifyGroup(f.session,'Sheet1',f.tile,[0],2,[0,10,20,30,40,50],f.original);assert.equal(c.status,'inconclusive');assert.equal(f.values.get(0),'');assert.ok(f.phases.includes('sparse_restore'));});
 await test('partially applied writes are restored even if sync fails',async()=>{const f=fixture({failWrite:true});const c=await X.verifyGroup(f.session,'Sheet1',f.tile,[0],2,[0,10,20,30,40,50],f.original);assert.equal(c.status,'inconclusive');assert.equal(f.values.get(0),'');});
 await test('native timeout stops all subsequent native work; pending restoration is explicit',async()=>{const f=fixture({hang:true});await assert.rejects(X.verifyGroup(f.session,'Sheet1',f.tile,[0],2,[0,10,20,30,40,50],f.original));assert.equal(f.session.dead,true);assert.ok(!f.phases.includes('sparse_restore'));assert.equal(f.session.state.sparse_overflow_checks[0].restoration,'unknown-native-timeout');});
 await test('protected, table, merged and rotated contexts are not modified',async()=>{for(const option of ['protected','table','merged','rotated']){const f=fixture({[option]:true});const c=await X.verifyGroup(f.session,'Sheet1',f.tile,[0],2,[0,10,20,30,40,50],f.original);assert.equal(c.status,'inconclusive');assert.equal(f.writes.length,0);}});
 await test('narrow crop must reproduce original candidate pixels',async()=>{const f=fixture();mark(f.original,28,8);const c=await X.verifyGroup(f.session,'Sheet1',f.tile,[0],2,[0,10,20,30,40,50],f.original);assert.equal(c.error.code,'SPARSE_CROP_MISMATCH');assert.equal(f.writes.length,0);});
 await test('B integration removes a line signal but retains real text extent',async()=>{global.Office={context:{requirements:{isSetSupported:()=>true}}};for(const hasText of [false,true]){const f=fixture({text:hasText});const result=await X.verify(f.session,{sheet:'Sheet1',base_right_column:1},f.tile,f.original,{background_rgba:[255,255,255]},[0,10,20,30,40,50],4);assert.equal(result.done,true);assert.equal(result.keep_column,hasText?2:1);assert.equal(result.unresolved,undefined);}});
 await test('unsupported API does not mutate or report a false absence',async()=>{global.Office={context:{requirements:{isSetSupported:()=>false}}};const f=fixture();const r=await X.verify(f.session,{sheet:'Sheet1',base_right_column:1},f.tile,f.original,{background_rgba:[255,255,255]},[0,10,20,30,40,50],4);assert.equal(r.unresolved,true);assert.equal(f.writes.length,0);});

 await test('wide sixteen-row evidence groups are planned below four MP before capture',async()=>{
  const session={limitsFor:()=>({...S.DEFAULTS,scale_x:4/3,scale_y:4/3,border_px:1,max_pixels:64000000}),
    geometries:async(s,rs)=>rs.map(r=>({left:0,right:11579.25,top:r.r0*12.75,bottom:r.r1*12.75}))};
  const tile={rect:{r0:0,r1:1000,c0:0,c1:180}};
  const p=await X.planGroups(session,'S',tile,Array.from({length:16},(_,i)=>i));
  assert.equal(p.groups.length,2);assert.equal(p.groups[0].rows.length,15);
  assert.equal(p.limits.max_pixels,4000000);
  assert.deepEqual(p.groups.flatMap(g=>g.rows),Array.from({length:16},(_,i)=>i));
  assert.ok(p.groups.every(g=>g.predicted.width*g.predicted.height<=4000000));
 });
 await test('more than 128 candidates are mapped but never written in one group',async()=>{
  const session={limitsFor:()=>({...S.DEFAULTS,scale_x:1,scale_y:1,border_px:1}),
   geometries:async(s,rs)=>rs.map(r=>({left:0,right:5000,top:r.r0*17,bottom:r.r1*17}))};
  const tile={rect:{r0:0,r1:160,c0:0,c1:80},geometry:{top:0,bottom:2720},raster_profile:{scale_y:1}};
  const rows=await X.pixelRowsToSheetRows(session,'S',tile,[[1,2718]]);
  assert.equal(rows.length,160);
  const p=await X.planGroups(session,'S',tile,rows);assert.ok(p.groups.every(g=>g.rows.length<=16));
  assert.equal(p.groups.flatMap(g=>g.rows).length,160);
 });
 await test('an indivisible evidence row is rejected before any write or render',async()=>{
  const session={limitsFor:()=>({...S.DEFAULTS,scale_x:1,scale_y:1,border_px:1}),geometries:async()=>[{left:0,right:20000,top:0,bottom:300}]};
  await assert.rejects(X.planGroups(session,'S',{rect:{c0:0,c1:300,r0:0,r1:1}},[0]),e=>e.code==='SPARSE_INDIVISIBLE_ROW');
 });
 await test('pixel-aware groups preserve gaps and never insert unselected rows',async()=>{
  const session={limitsFor:()=>({...S.DEFAULTS,scale_x:2,scale_y:2,border_px:1}),geometries:async(s,rs)=>rs.map(r=>({left:0,right:5000,top:r.r0*30,bottom:r.r1*30}))};
  const rows=[0,1,5,6,7,10];const p=await X.planGroups(session,'S',{rect:{r0:0,r1:20,c0:0,c1:40}},rows);
  assert.deepEqual(p.groups.flatMap(g=>g.rows),rows);assert.ok(p.groups.every(g=>g.rows.length===g.rect.r1-g.rect.r0));
 });
 await test('raw gridlines stay candidates; this correction does not hide their cost',()=>{
  const im=white(51,21);for(let y=1;y<20;y++)mark(im,30,y,224);
  const c=X.scanCandidates(im,[0,10,20,30,40,50],1,4,[248,248,248]);assert.equal(c.pixel_rows,19);
 });
 await test('confirmed blocker counters and scan records reflect actual work',async()=>{
  global.Office={context:{requirements:{isSetSupported:()=>true}}};const f=fixture({text:true});
  const r=await X.verify(f.session,{sheet:'Sheet1',base_right_column:1},f.tile,f.original,{background_rgba:[255,255,255]},[0,10,20,30,40,50],4);
  assert.equal(f.session.state.sparse_overflow_scans[0].status,'complete');
  assert.equal(f.session.state.sparse_overflow_scans[0].id,r.scan_id);
  assert.ok(r.verification_group_count>0);
  for(const check of f.session.state.sparse_overflow_checks){
    assert.equal(check.write_confirmed_cells,check.rows.length);assert.equal(check.restore_confirmed_cells,check.rows.length);
  }
  const unsafe=fixture({protected:true});const c=await X.verifyGroup(unsafe.session,'Sheet1',unsafe.tile,[0],2,[0,10,20,30,40,50],unsafe.original);
  assert.equal(c.write_confirmed_cells,0);assert.equal(c.write_attempted_cells,0);
 });
 await test('unresolved scan without an intervention is retained in structured results',async()=>{
  global.Office={context:{requirements:{isSetSupported:()=>false}}};const f=fixture();
  const r=await X.verify(f.session,{sheet:'Sheet1',base_right_column:1},f.tile,f.original,{background_rgba:[255,255,255]},[0,10,20,30,40,50],4);
  assert.equal(r.unresolved,true);assert.equal(f.session.state.sparse_overflow_checks.length,0);
  assert.equal(f.session.state.sparse_overflow_scans[0].reason,'SPARSE_API_UNAVAILABLE');
 });
 console.log(`${count} selective overflow mock tests passed. No Excel execution.`);
})().catch(e=>{console.error(e);process.exitCode=1;});
