'use strict';
// Mock Office object model only; no assertions below represent measured Excel behaviour.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');
const web=path.resolve(__dirname,'../../_shared/office-addin/web');
const D=require(path.join(web,'render-diagnostics.js'));
const R=require(path.join(web,'row-height-diagnostics.js'));
const manifest=JSON.parse(fs.readFileSync(path.resolve(__dirname,'../row-height-fixtures/manifest.json'),'utf8'));
let tests=0;
async function test(name,fn){await fn();tests++;console.log('PASS',name);}
function png(w,h){const b=Buffer.alloc(57);Buffer.from('\x89PNG\r\n\x1a\n','latin1').copy(b);b.writeUInt32BE(13,8);b.write('IHDR',12);b.writeUInt32BE(w,16);b.writeUInt32BE(h,20);b[24]=8;b[25]=6;b.write('IDAT',37);Buffer.from('0000000049454e44ae426082','hex').copy(b,45);return b.toString('base64');}
const realTimer=global.setTimeout;
global.setTimeout=(fn,ms)=>realTimer(fn,ms<=250?0:ms);
global.ExperimentUi=Object.fromEntries(['setPhase','log','setWorkbook','updateProgress','fail'].map(x=>[x,()=>{}]));
global.window=global;
function mock(fixture){
  const events=[],posts=[],mutations=[];const sizes=new Map(),forced=new Map();
  const profiles=new Map([...(fixture.matrices||[]),...(fixture.geometry||[]),...(fixture.runtime_controls||[]),...(fixture.rounding?[fixture.rounding]:[])].map(p=>[p.sheet,p]));
  function rawHeight(sheet,row){const p=profiles.get(sheet)||{};if(forced.has(sheet))return forced.get(sheet);if(p.heights)return p.heights[row-1]||15;if(p.row_custom_height===1)return p.row_ht;
    const c=p.cases?.find(c=>c.row===row);const style=c?.cells?.[0]?.[2]??c?.row_style??0;
    const size=sizes.get(sheet)??[fixture.normal_font_size_pt,8,11,24,48,8,24][style];
    return Math.max(p.default_row_height_pt||15,size*1.1);
  }
  function font(sheet,row,column){const p=profiles.get(sheet)||{};const c=p.cases?.find(c=>c.row===row);const cell=c?.cells?.find(v=>v[0]===column);const style=cell?.[2]??c?.row_style??0;
    return {name:'Calibri',load(){},get size(){return sizes.get(sheet)??[fixture.normal_font_size_pt,8,11,24,48,8,24][style];},set size(v){mutations.push([sheet,'font',v]);sizes.set(sheet,v);}};
  }
  function range(sheet,address){const m=address.match(/^([A-Z]+)(\d+)(?::([A-Z]+)(\d+))?$/);assert.ok(m,address);const first=+m[2],last=+(m[4]||m[2]);const rows=Array.from({length:last-first+1},(_,i)=>i+first);const column=m[1];
    const height=()=>rows.reduce((s,r)=>s+rawHeight(sheet,r),0);
    const top=()=>Array.from({length:first-1},(_,i)=>i+1).reduce((s,r)=>s+rawHeight(sheet,r),0);
    return {address,width:60,left:0,rowCount:rows.length,columnCount:1,rowHidden:false,
      get height(){return height();},get top(){return top();},
      load(){events.push([sheet,'geometry',address]);},
      format:{load(){},get rowHeight(){return rawHeight(sheet,first);},font:font(sheet,first,column),autofitRows(){mutations.push([sheet,'autofit']);forced.set(sheet,(sizes.get(sheet)||11)*1.1);}},
      getImage(){events.push([sheet,'image',address]);return {value:png(81,rows.reduce((s,r)=>s+Math.floor(rawHeight(sheet,r)*4/3+1e-9),1))};}
    };
  }
  global.Excel={run:async fn=>fn({trace(){},sync:async()=>{},workbook:{name:fixture.workbook,load(){},worksheets:{getItem:sheet=>({activate(){events.push([sheet,'activate']);},getRange:a=>range(sheet,a)})}}})};
  global.fetch=async(url,init)=>{
    if(url==='/config')return{ok:true,json:async()=>({experiment:'range-image-calibration',diagnostic_run_id:'test',diagnostic_options:{fixtures:[fixture],suite:'row-heights'}})};
    const b=JSON.parse(init.body);assert.equal(init.headers['X-VV-Run-Id'],'test');
    // Retain only one final result; progress payloads are intentionally large.
    if(url==='/result')posts.push([url,b]);
    let data={ok:true};if(url==='/diagnostic-claim')data={acquired:true};
    if(url==='/image-batch')data={captures:b.images.map(i=>({id:i.id,relative_png:`${fixture.workbook}/${i.id}.png`}))};
    return{ok:true,json:async()=>data};
  };
  global.document={createElement:()=>({}),head:{appendChild(script){assert.match(script.src,/^\/row-height-diagnostics\.js\?run=test$/);script.onload();}}};
  return{events,posts,mutations};
}
(async()=>{
  await test('partition correction is inferred, not silently forced to one pixel',()=>{
    const r=R.summarizePartitions({success:true,png_height:161},[{success:true,png_height:81},{success:true,png_height:81}]);
    assert.equal(r.inferred_extra_pixel_per_capture,1);assert.equal(r.residuals_by_assumed_extra_pixel[1].residual_px,0);
    assert.equal(R.summarizePartitions({success:false},[]).status,'inconclusive');
  });
  await test('row-level XML flags preserve absent versus zero versus one',()=>{
    assert.equal(R.rowSource({sheet:'s',row_custom_height:0},1).xml_custom_height,0);
    assert.equal(R.rowSource({sheet:'s',row_custom_height:1},1).xml_custom_height,1);
    assert.equal(R.rowSource({sheet:'s'},1).xml_custom_height,null);
  });
  for(const fixture of manifest.fixtures){
    await test('real suite orchestration with mocked Office: '+fixture.workbook,async()=>{
      const m=mock(fixture);
      const script=path.join(web,'modes/range-image-calibration.js');
      await vm.runInThisContext(fs.readFileSync(script,'utf8'),{filename:script});
      const result=m.posts.find(([u])=>u==='/result')?.[1];
      assert.ok(result);assert.equal(result.status,'complete',JSON.stringify(result.analysis?.fatal_error));
      assert.equal(result.analysis.row_height.matrix.length,(fixture.matrices||[]).length);
      assert.equal(result.analysis.row_height.geometry.length,(fixture.geometry||[]).length);
      assert.ok(result.analysis.row_height.snapshots.every(s=>s.rows.length>0));
      for(const profile of fixture.geometry||[]){
        const e=m.events.filter(e=>e[0]===profile.sheet);
        assert.equal(e[0][1],profile.measurement_order==='image-first'?'image':'geometry');
        assert.ok(e.findIndex(e=>e[1]==='activate')>e.findIndex(e=>e[1]==='image'));
      }
      assert.ok(m.mutations.every(([sheet])=>sheet==='RuntimeAuto'||sheet==='RuntimeFixed'));
      if(fixture.rounding)assert.equal(result.analysis.row_height.rounding.observations.length,fixture.rounding.heights.length);
      // There must be actual raw-row snapshots in JSON, not only aggregate/regression summaries.
      assert.ok(result.analysis.row_height.snapshots.every(s=>s.aggregate_before_row_reads&&s.aggregate_after_row_reads));
    });
  }
  await test('unknown future image dimensions are not clamped by the new probe branch',async()=>{
    const f=manifest.fixtures[2];const m=mock(f);const s=new D.Session({experiment:'range-image-calibration',diagnostic_run_id:'test',diagnostic_options:{}},f.workbook);
    const p=await s.probe({sheet:'ImgFixed126',address:'A1:A1000',role:'image-first-test'},{geometryOrder:'image-first'});
    assert.equal(p.geometry_timing,'after-getImage');assert.equal(m.events[0][1],'image');assert.ok(p.png_height>10000);
  });
  await test('timeout halts follow-up calls rather than measuring a changed state',async()=>{
    let called=0;global.Excel={run:()=>{called++;return new Promise(()=>{});}};
    global.setTimeout=(fn,ms)=>realTimer(fn,ms>=10000?0:ms);
    const s={halted:false,options:{probe_timeout_ms:10000}};
    const log=console.error;console.error=()=>{};
    try{await assert.rejects(R.office(s,'timeout-test',async()=>{}),/timeout/);assert.equal(s.halted,true);await assert.rejects(R.office(s,'next',async()=>{}),/No further Office/);assert.equal(called,1);}
    finally{console.error=log;global.setTimeout=(fn,ms)=>realTimer(fn,ms<=250?0:ms);}
  });
  console.log(`${tests} row-height tests passed (mocks, not Excel measurements).`);
})().catch(e=>{console.error(e);process.exitCode=1;}).finally(()=>{global.setTimeout=realTimer;});
