'use strict';
// Real orchestration against an explicit deterministic renderer model. This is not
// an Excel conformance test; the readiness gate performs that on the user's host.
const assert=require('node:assert/strict'),path=require('node:path'),fs=require('node:fs'),vm=require('node:vm');
const web=path.resolve(__dirname,'../_shared/office-addin/web');
const G=require(path.join(web,'capture-geometry.js')),S=require(path.join(web,'safe-capture.js'));
const P=require(path.join(web,'capture-policy.js')),V=require(path.join(web,'context-validation.js'));
function mock(o={}){
 let nonce=0;const frames=new Map(),byId=new Map(),posts=[],queries=[],rendered=[];
 const xs=c=>o.width?o.width(c):45,ys=r=>o.height?o.height(r):12.75;
 const sx=o.sx||4/3,sy=o.sy||4/3;
 const sum=(fn,n)=>{let x=0;for(let i=0;i<n;i++)x+=fn(i);return x;};
 function png(width,height,frame){const b=Buffer.alloc(57);Buffer.from([137,80,78,71,13,10,26,10]).copy(b);b.writeUInt32BE(13,8);b.write('IHDR',12);b.writeUInt32BE(width,16);b.writeUInt32BE(height,20);b.writeUInt32BE(++nonce,32);b.write('IEND',49);const s=b.toString('base64');frames.set(s,{width,height,...frame});return s;}
 global.fetch=async(url,init)=>{const body=JSON.parse(init.body);posts.push({url,body});let value={ok:true};
  if(url==='/image-batch'){value={captures:body.images.map(x=>{const h=S.png(x.base64);byId.set(x.id,frames.get(x.base64));return{id:x.id,sheet:x.sheet,address:x.address,relative_png:'mock/'+x.id+'.png',png_width:h.width,png_height:h.height,sha256:'a'.repeat(64)}})};}
  if(url==='/finalize-image'){const f=byId.get(body.source_id),box={x:body.x||0,y:body.y||0,width:body.width,height:body.height||f.height};value={capture:{id:body.source_id+'-final',sheet:body.sheet,address:body.address,relative_png:'mock/'+body.source_id+'-final.png',png_width:box.width,png_height:box.height,sha256:'b'.repeat(64)}};}
  return{ok:true,json:async()=>value};};
 global.Image=class{set src(s){if(!s)return;this.frame=frames.get(s.split(',')[1]);if(!this.frame)throw new Error('Unknown fake PNG');this.naturalWidth=this.frame.width;this.naturalHeight=this.frame.height;queueMicrotask(()=>this.onload());}};
 global.document={createElement:()=>({width:0,height:0,getContext(){let frame=null;return{drawImage(im){frame=im.frame;},getImageData(x0,y0,w,h){const out=new Uint8ClampedArray(w*h*4);for(let y=0;y<h;y++)for(let x=0;x<w;x++){
    const px=x+x0,py=y+y0,a=(y*w+x)*4,gy=frame.top+py,gx=frame.left+px;
    // A vertical cell border and nonuniform interior; the first row of a
    // range starting below row 1 lacks the line (the incident under test).
    const border=gx===60 && !(py===0&&frame.r0>0);
    out[a]=border?0:(gx*3+gy)%251;out[a+1]=border?0:(gy*7+gx)%251;out[a+2]=border?0:123;out[a+3]=255;
    if(gx===60&&!border){out[a]=out[a+1]=out[a+2]=255;}
   }return{data:out};}};}})};
 global.Excel={run:async fn=>{const queue=[];
  function range(sheet,r){return{load(props){queries.push({r,props});},get top(){return sum(ys,r.r0)*(o.nativeTopFactor||1);},get left(){return sum(xs,r.c0);},
    get height(){return sum(ys,r.r1)-sum(ys,r.r0);},get width(){return sum(xs,r.c1)-sum(xs,r.c0);},get rowHidden(){return ys(r.r0)===0;},get columnHidden(){return xs(r.c0)===0;},
    getMergedAreasOrNullObject(){const merges=(o.merges||[]).filter(m=>m.r0<r.r1&&m.r1>r.r0&&m.c0<r.c1&&m.c1>r.c0);return{load(){},isNullObject:!merges.length,areas:{items:merges.map(m=>({rowIndex:m.r0,rowCount:m.r1-m.r0,columnIndex:m.c0,columnCount:m.c1-m.c0}))}};},
    getImage(){const result={};queue.push(()=>{rendered.push(r);if(o.renderError&&o.renderError(r))throw new Error('Renderer rejected');
      const width=Math.round((sum(xs,r.c1)-sum(xs,r.c0))*sx)+1,height=Math.round((sum(ys,r.r1)-sum(ys,r.r0))*sy)+1;
      result.value=png(width,height,{top:Math.round(sum(ys,r.r0)*sy),left:Math.round(sum(xs,r.c0)*sx),r0:r.r0});});return result;}};}
  return fn({workbook:{worksheets:{getItem(sheet){return{name:sheet,load(){},getRange:a=>range(sheet,S.parse(a)),getRangeByIndexes:(r,c,n,m)=>range(sheet,{r0:r,r1:r+n,c0:c,c1:c+m})};}}},trace(){},sync:async()=>{while(queue.length)queue.shift()();}});
 }};
 return{posts,queries,rendered,frames};
}
let n=0;async function test(label,fn){await fn();n++;console.log('PASS '+label);}
(async()=>{
 await test('prefix sums preserve hidden zeros',()=>assert.deepEqual(G.prefix([{size:3},{size:0},{size:5}]),[0,3,3,8]));
 await test('guard skips zero-height rows instead of counting indices',()=>{
  const rows=[{size:9},{size:0},{size:0},{size:12},{size:12},{size:0},{size:7}];
  const g=G.neighbour(rows,3,-1,0,7,4/3);assert.equal(g.boundary,0);assert.equal(g.visible_cells,1);assert.equal(g.zero_extent_cells,2);
 });
 await test('thin visible neighbours accumulate at least two device pixels',()=>{
  const g=G.neighbour([{size:2},{size:.1},{size:0},{size:.1}],4,-1,0,4,1);assert.equal(g.boundary,0);assert.equal(g.visible_cells,3);
 });
 await test('a physical zero-only outer tail is explicit',()=>assert.equal(G.neighbour([{size:0},{size:0}],0,1,0,2,2).physical_outer_edge,true));
 await test('all four internal sides receive actual visible neighbours',()=>{
  const d={rows:Array.from({length:6},()=>({size:10})),columns:Array.from({length:6},()=>({size:10}))};
  const g=G.guardBox(d,{r0:2,r1:4,c0:2,c1:4},{r0:0,r1:6,c0:0,c1:6},{scale_x:1,scale_y:1});
  assert.deepEqual(g.capture_rect,{r0:1,r1:5,c0:1,c1:5});
 });
 await test('crop guards reject out-of-bounds and phase mismatch',()=>{
  assert.throws(()=>G.cropBox({left:0,right:20,top:0,bottom:20},{left:0,right:20,top:0,bottom:20},{left:0,top:0},{scale_x:1,scale_y:1},{width:10,height:10}));
 });
 await test('non-A1 origin keeps a correctly measured interior crop',()=>{
  const b=G.cropBox({left:50,right:100,top:30,bottom:60},{left:40,right:110,top:20,bottom:70},{left:0,top:0},{scale_x:2,scale_y:2},{width:141,height:101});
  assert.deepEqual(b,{x:20,y:20,width:101,height:61});
 });
 await test('ledger ignores erroneous native top; PNG calibrates at actual common factor',async()=>{
  mock({nativeTopFactor:.75});const s=new S.Session('x');await s.ensureCalibration('S',S.parse('A1:H50'));
  assert.equal(s.limitsFor('S').scale_y,4/3);assert.ok(s.state.axis_geometry[0].max_native_top_delta_points>0);
  const g=(await s.geometries('S',[S.parse('A1:A10')]))[0];assert.equal(g.bottom,127.5);
 });
 await test('hidden first column is never used as positive padding',async()=>{
  mock({width:c=>c===0?0:45});const s=new S.Session('x');await s.ensureCalibration('S',S.parse('A1:H20'));
  assert.equal(s.axisGeometry.data('S').columns[0].size,0);assert.equal(s.axisGeometry.data('S').anchor,1);
 });
 await test('repeated cached geometry requests do not perform repeated Office reads',async()=>{
  const m=mock();const s=new S.Session('x');await s.geometries('S',[S.parse('A1:H50')]);const q=m.queries.length;
  for(let i=0;i<100;i++)await s.geometries('S',[S.parse('A2:G49')]);assert.equal(m.queries.length,q);
 });
 await test('prefix scans are bounded and reject million-row requests before render',async()=>{
  const m=mock();const s=new S.Session('x');await assert.rejects(s.geometries('S',[S.parse('A1:A1048576')]),e=>e.code==='AXIS_GEOMETRY_BUDGET');assert.equal(m.rendered.length,0);
 });
 await test('refresh detects changed individual heights even if sums would compensate',async()=>{
  let changed=false;mock({height:r=>changed?(r===0?13.5:r===1?12:12.75):12.75});const s=new S.Session('x');await s.geometries('S',[S.parse('A1:H5')]);changed=true;
  await assert.rejects(s.geometries('S',[S.parse('A1:H5')],true),e=>e.code==='AXIS_GEOMETRY_CHANGED');
 });
 await test('contextual packing budgets include top and bottom neighbours',async()=>{
  mock();const s=new S.Session('x');s.limits.max_pixels=60000;const root=S.parse('A1:H30');const tiles=await s.plan('S',root);
  assert.ok(tiles.length>2);assert.equal(tiles[0].rect.r0,0);assert.equal(tiles.at(-1).rect.r1,30);
  for(let i=0;i<tiles.length;i++){const t=tiles[i];assert.ok(t.predicted.width*t.predicted.height<=60000);if(i)assert.ok(t.capture_rect.r0<t.rect.r0);if(i+1<tiles.length)assert.ok(t.capture_rect.r1>t.rect.r1);}
 });
 await test('merged cells crossing a cut expand actual context, not source content bounds',async()=>{
  mock({merges:[{r0:4,r1:8,c0:1,c1:2}]});const s=new S.Session('x');await s.ensureCalibration('S',S.parse('A1:H20'));
  const d=await s.axisGeometry.descriptor('S',{r0:0,r1:6,c0:0,c1:8},S.parse('A1:H20'),s.limitsFor('S'),true);
  assert.equal(d.rect.r1,6);assert.equal(d.capture_rect.r1,8);assert.equal(d.merge_context.length,1);
 });
 await test('native PNG is padded, analysis receives core metadata, final native crop removes guards once',async()=>{
  const m=mock();const s=new S.Session('x');s.limits.max_pixels=60000;const tiles=await s.plan('S',S.parse('A1:H30'));
  let callback=null;const [c]=await s.capture('S',tiles[1],'c00001',async(b,c)=>{callback={png:S.png(b),core:c.core_box_px};return{};});
  assert.ok(callback.core.y>0);assert.ok(callback.png.height>callback.core.height);
  const f=await P.exportTile(s,c,c.core_box_px.width,S.address(c.rect));assert.equal(f.png_height,c.core_box_px.height);
  const crop=m.posts.find(x=>x.url==='/finalize-image').body;assert.equal(crop.y,callback.core.y);assert.equal(crop.source_height,callback.png.height);
 });
 await test('whole vs padded pieces matches despite simulated native edge border loss',async()=>{
  mock();const s=new S.Session('x',{capture_options:{context_validation:'first-seam'}});s.limits.max_pixels=60000;
  const tiles=await s.plan('S',S.parse('A1:H30'));await V.verify(s,'S',tiles,'c00001');
  const v=s.state.context_validations[0];assert.equal(v.status,'complete');assert.equal(v.overlap_mismatches,0);assert.equal(v.full_comparison.equal,true);assert.equal(v.artifacts.length,4);
 });
 await test('whole vs context reference also works with hidden rows across the cut',async()=>{
  mock({height:r=>[3,4,5,9,10].includes(r)?0:12.75});const s=new S.Session('x');s.limits.max_pixels=60000;
  const tiles=await s.plan('S',S.parse('A1:H30'));await V.verify(s,'S',tiles,'c00001');assert.equal(s.state.context_validations[0].status,'complete');
 });
 await test('normal 150% scale remains empirical and both axes calibrated',async()=>{
  mock({sx:2,sy:2});const s=new S.Session('x');await s.ensureCalibration('S',S.parse('A1:H50'));assert.equal(s.limitsFor('S').scale_x,2);assert.equal(s.limitsFor('S').scale_y,2);
 });
 await test('one changed interior pixel fails exact reference comparison',()=>{
  const a={width:2,height:2,data:new Uint8ClampedArray(16)},b={...a,data:new Uint8ClampedArray(16)};b.data[8]=1;
  const d=V.difference(a,b);assert.equal(d.equal,false);assert.equal(d.different_pixels,1);
 });
 await test('overlapping strips preserve conflicting pixels as an error, not a tolerance',()=>{
  const a={width:1,height:2,data:new Uint8ClampedArray([0,0,0,255,0,0,0,255])},b={...a,data:new Uint8ClampedArray([255,255,255,255,0,0,0,255])};
  assert.equal(V.compose(a,b,1,3).overlap_mismatches,1);
 });
 await test('progress snapshots do not serialize the large immutable plan',async()=>{
  const m=mock();const s=new S.Session('x');s.state.precomputed_wsl_rust_visual_plan={large:'x'.repeat(100000)};
  await s.checkpoint('workbook_finished');assert.equal(m.posts.at(-1).body.precomputed_wsl_rust_visual_plan,undefined);
 });
 await test('F mode executes actual contextual module and retains logical final rows',async()=>{
  mock();const s=new S.Session('x',{strategy:'F'});s.limits.max_pixels=60000;
  const src=fs.readFileSync(path.join(web,'modes/range-image.js'),'utf8'),cut=src.lastIndexOf('  try {\n    await execute();');
  const code=src.slice(0,cut)+`globalThis.hooks={run:renderPrecomputedPlannedBatch,set:s=>{safeSession=s}};}());`;
  const ui=new Proxy({},{get:()=>()=>{}}),ctx={window:{fetch:global.fetch},SafeCapture:S,CaptureGeometry:G,ContextValidation:V,CapturePolicy:P,Excel:global.Excel,performance,console,atob,Map,Uint8Array,Number,ExperimentUi:ui,ExperimentEvents:{post:async()=>{}}};
  vm.createContext(ctx);vm.runInContext(code,ctx);ctx.hooks.set(s);
  const result=await ctx.hooks.run('x',[{id:'c00001',sheet:'S',sheet_index:1,address:'A1:H30',bottom_row:29,base_right_column:7,end_column:7,overflow_probe:false,column_widths:Array(16384).fill(45)}],1,1,{strategy:'F'});
  assert.equal(s.state.sheets[0].status,'complete');assert.ok(result.captures.length>1);assert.equal(result.captures[1].address,S.address(result.captures[1].rect));
  assert.ok(result.captures[1].crop_rect_px.y>0);assert.equal(result.captures.at(-1).rect.r1,30);
 });
 await test('an entirely hidden logical range queues no calibration picture',async()=>{
  const m=mock({width:()=>0});const s=new S.Session('x');const plan=await s.plan('S',S.parse('A1:H30'));
  assert.equal(plan.length,1);assert.equal(plan[0].invisible,true);assert.equal(m.rendered.length,0);
 });
 await test('an unrestored intervention prevents sheet-local geometry continuation',async()=>{
  mock();const s=new S.Session('x');s.state.sparse_overflow_checks.push({write_attempted_cells:1,restoration:'unknown'});
  const result=await s.continueAfterSheetFailure('S',S.fault('AXIS_GEOMETRY_CHANGED','bad',{},true));
  assert.equal(result,false);
 });
 console.log(`${n} context / additive geometry / exact validation tests passed (modeled renderer, not Excel).`);
})().catch(e=>{console.error(e);process.exitCode=1;});
