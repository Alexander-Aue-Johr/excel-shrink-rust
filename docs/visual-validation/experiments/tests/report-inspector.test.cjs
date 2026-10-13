// Pure projections and conservative overflow-rule checks. No native Excel simulation.
'use strict';
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const base=path.resolve(__dirname,'..');
const report=require(path.join(base,'_shared/report/inspector.js'));
let count=0;function test(name,fn){fn();count++;console.log('PASS '+name);}
const file='artifacts/range-image/f/dotnet-b1/Book.xlsx/tile.png',source='data/range-image/f/dotnet-b1.json';
const fixture=()=>({meta:{id:'x'},documents:[{path:source,workbooks:[{workbook:'Book.xlsx',safe_capture:{
  sheet_catalog:[{name:' Leading name',sheet_index:5,office_position:4}],
  captures:[{id:'tile',sheet:' Leading name',relative_png:'Book.xlsx/tile.png',address:'A1:D4',png_width:280,png_height:80}],
  attempts:[{id:'tile',sheet:' Leading name',relative_png:'Book.xlsx/tile.png',render_ms:5}],
  sheets:[{sheet:' Leading name',sheet_index:5,status:'complete',requested_address:'A1:D4',address:'A1:C4',tiles:[{id:'tile',relative_png:'Book.xlsx/tile.png',address:'A1:D4',valid_rect_px:{x:0,y:0,width:210,height:80}}]}]
 }}]}],files:[{path:file,bytes:1000,width:280,height:80}]});
test('relative links reject traversal, protocols, control characters and ambiguous URL fragments',()=>{
 for(const bad of ['../secret','file:///a','/absolute','a\\b','https://a','a/../b','a/#b','a?b','x\n.png'])assert.equal(report.safePath(bad),null);
 assert.equal(report.safePath('artifacts/Ü & x.png'),'artifacts/%C3%9C%20%26%20x.png');
});
test('A1 parsing includes terminal grid and preserves non-A1 origins',()=>{
 assert.deepEqual(report.range('B2:XFD1048576'),{r0:1,r1:1048576,c0:1,c1:16384});
 assert.equal(report.range('A1:XFE4'),null);assert.equal(report.envelope(2790,171),'A1:FP2791');
});
test('exact names, real indices and final rectangles survive the projection',()=>{
 const m=report.buildModel(fixture()),s=m.sessions[0].sheets[0];assert.equal(s.name,' Leading name');assert.equal(s.index,5);
 assert.equal(s.images.length,1);assert.equal(s.images[0].kind,'final');assert.equal(s.images[0].render_ms,5);
 assert.equal(s.images[0].valid_rect_px.width,210);assert.equal(m.unassigned.length,0);
});
test('final payload supersedes its progress snapshot without duplicating images',()=>{
 const x=fixture();x.documents.push({path:'artifacts/range-image/f/dotnet-b1/_progress/Book.xlsx.progress.json',progress:{...x.documents[0].workbooks[0].safe_capture,workbook:'Book.xlsx'}});
 const m=report.buildModel(x);assert.equal(m.sessions.length,1);assert.equal(m.sessions[0].sheets[0].images.length,1);
});
test('a progress-only failed run still shows raw images and noncaptured sheets',()=>{
 const x=fixture(),w=x.documents[0].workbooks[0];w.safe_capture.sheet_catalog.push({name:'Never started',sheet_index:7});
 x.documents=[{path:'artifacts/range-image/f/dotnet-b1/_progress/Book.xlsx.progress.json',progress:{...w.safe_capture,workbook:w.workbook}}];
 const m=report.buildModel(x);assert.equal(m.sessions[0].sheets.length,2);assert.equal(m.sessions[0].sheets[1].status,'not-captured');assert.equal(m.unassigned.length,0);
});
test('journal-ahead-of-checkpoint requests remain visible and are not duplicated',()=>{
 const x=fixture(),event={stage:'overflow_pass_requested',timestamp_utc:'2026-10-04T21:00:00Z',workbook:'Book.xlsx',sheet:' Leading name',address:'A1:FX2791',pass:2};
 x.events=[{stage:'bounds_stage',source_file:source+'.events.jsonl',detail:event}];
 assert.equal(report.buildModel(x).sessions[0].sheets[0].history[0].address,event.address);
 x.documents[0].workbooks[0].safe_capture.bounds_history=[event];assert.equal(report.buildModel(x).sessions[0].sheets[0].history.length,1);
 x.events[0].source_file='data/range-image/f/rust-b1.json.events.jsonl';x.documents[0].workbooks[0].safe_capture.bounds_history=[];
 assert.equal(report.buildModel(x).sessions[0].sheets[0].history.length,0);
});
test('missing files and ambiguous duplicate filenames are not silently reassigned',()=>{
 const x=fixture();x.files=[{path:'artifacts/other/a/Book.xlsx/tile.png'},{path:'artifacts/other/b/Book.xlsx/tile.png'}];
 const m=report.buildModel(x);assert.equal(m.sessions[0].sheets[0].images.length,0);assert.equal(m.unassigned.length,2);
});
test('legacy Rust visible indices are explicitly labelled as legacy, not actual worksheet positions',()=>{
 const x=fixture(),w=x.documents[0].workbooks[0];w.safe_capture.sheet_catalog=[];delete w.safe_capture.sheets[0].sheet_index;
 x.plans=[{document:{workbooks:[{workbook:'Book.xlsx',sheets:[{sheet:' Leading name',sheet_index:4,base_right_column:171,bottom_row:2790,content_right_column:171,content_bottom_row:2790}]}]}}];
 const s=report.buildModel(x).sessions[0].sheets[0];assert.equal(s.index,4);assert.match(s.index_basis,/legacy/);assert.equal(s.bounds[0].address,'A1:FP2791');
});
test('diagnostic-only screenshots are labelled captured, not falsely not-captured or complete',()=>{
 const x=fixture(),w=x.documents[0].workbooks[0];w.captures=w.safe_capture.captures;delete w.safe_capture;
 const sh=report.buildModel(x).sessions[0].sheets[0];assert.equal(sh.status,'captured');assert.equal(sh.images[0].kind,'diagnostic');
});
function overflowFunctions(){let s=fs.readFileSync(path.join(base,'_shared/office-addin/web/modes/range-image.js'),'utf8');
 const end=s.lastIndexOf('\n  try {\n    await execute();');assert.ok(end>0);
 s=s.slice(0,end)+'\n globalThis.F={analyzeRenderOverflow,horizontalRuleMask};\n}());';const context={console};vm.runInNewContext(s,context);return context.F;}
const F=overflowFunctions();
function image(){const width=201,height=81,data=new Uint8Array(width*height*4);data.fill(255);return{width,height,data};}
function pixel(im,x,y,r=224,g=r,b=r){const o=(y*im.width+x)*4;im.data[o]=r;im.data[o+1]=g;im.data[o+2]=b;im.data[o+3]=255;}
function verdict(im,end=3){const w=Array(16384).fill(50);return F.analyzeRenderOverflow(im,w,1,end,2);}
test('interrupted horizontal gridline does not trigger growth through an empty probe band',()=>{
 const im=image();for(let x=1;x<200;x++)pixel(im,x,17);pixel(im,10,17,0);
 const result=verdict(im);assert.equal(result.done,true);assert.equal(result.keep_column,1);assert.equal(result.ignored_probe_rule_rows,1);
});
test('localized glyph-like pixels at the edge remain overflow evidence',()=>{
 const im=image();for(let y=20;y<28;y++)for(let x=183;x<188;x++)pixel(im,x,y,0);
 const result=verdict(im);assert.equal(result.done,false);assert.ok(result.edge_hits.length);assert.equal(result.keep_column,3);
});
test('uniform thick visible bands are not treated as thin gridlines',()=>{
 const im=image();for(let y=20;y<26;y++)for(let x=101;x<200;x++)pixel(im,x,y,0);
 assert.equal(verdict(im).done,false);
});
test('a single extra column does not enable the new probe-band mask',()=>{
 const im=image();for(let x=101;x<200;x++)pixel(im,x,20,0);pixel(im,10,20,0);
 const w=Array(16384).fill(100);const result=F.analyzeRenderOverflow(im,w,0,1,1);
 assert.equal(result.ignored_probe_rule_rows,0);assert.equal(result.done,false);
});
test('physical final images retain raw evidence as a separate image and direct link',()=>{
 const x=fixture(),safe=x.documents[0].workbooks[0].safe_capture;
 safe.capture_view={status:'confirmed',after:[{sheet:' Leading name',show_gridlines:false}]};
 safe.sheet_catalog[0].show_gridlines=false;
 safe.sheets[0].export_policy={crop:true,reason:'no-overflow-remove-buffer'};
 safe.sheets[0].tiles=[{id:'tile-final',sheet:' Leading name',relative_png:'Book.xlsx/tile-final.png',raw_png:'Book.xlsx/tile.png',png_width:210,png_height:80,export_kind:'native-right-crop',address:'A1:C4'}];
 const cropped=file.replace('tile.png','tile-final.png');x.files.push({path:cropped,bytes:700,width:210,height:80});
 const sh=report.buildModel(x).sessions[0].sheets[0];assert.equal(sh.images.length,2);
 const im=sh.images.find(x=>x.kind==='final');assert.equal(im.path,cropped);assert.equal(im.raw_path,file);assert.equal(im.width,210);
 assert.equal(sh.show_gridlines,false);assert.ok(sh.bounds.some(x=>x.source==='no-overflow-remove-buffer'));
});
test('failed final export still shows the raw capture as inspectable evidence',()=>{
 const x=fixture(),safe=x.documents[0].workbooks[0].safe_capture;safe.sheets[0].tiles=[];safe.sheets[0].status='failed';
 safe.final_exports=[{status:'failed',source_id:'tile'}];const sh=report.buildModel(x).sessions[0].sheets[0];assert.equal(sh.images.length,1);assert.equal(sh.images[0].kind,'probe');
});
console.log(count+' report/provenance/overflow tests passed; no native Excel calls.');
