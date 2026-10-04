/* Offline visual-validation report. No network dependencies; works from file:// and extracted review bundles. */
(function(root){
'use strict';
const list=x=>Array.isArray(x)?x:[], obj=x=>x&&typeof x==='object'?x:{}, text=x=>x==null?'Not recorded':String(x);
const number=x=>typeof x==='number'&&Number.isFinite(x)?x:null;
function col(n){if(!Number.isInteger(n)||n<0||n>=16384)return null;let s='';for(n++;n>0;n=Math.floor((n-1)/26))s=String.fromCharCode(65+(n-1)%26)+s;return s;}
function envelope(row,column){return Number.isInteger(row)&&row>=0&&row<1048576&&col(column)?`A1:${col(column)}${row+1}`:null;}
function range(a){const m=String(a||'').match(/^\$?([A-Z]+)\$?(\d+)(?::\$?([A-Z]+)\$?(\d+))?$/i);if(!m)return null;
 const c=s=>[...s.toUpperCase()].reduce((v,x)=>v*26+x.charCodeAt(0)-64,0);const r={r0:+m[2]-1,r1:+(m[4]||m[2]),c0:c(m[1])-1,c1:c(m[3]||m[1])};
 return r.r0>=0&&r.r1>r.r0&&r.c1>r.c0&&r.r1<=1048576&&r.c1<=16384?r:null;}
function safePath(value){if(typeof value!=='string'||!value||/[\x00-\x1f\\:?#]/.test(value)||value.startsWith('/'))return null;
 const parts=value.split('/');if(parts.some(p=>!p||p==='.'||p==='..'))return null;return parts.map(encodeURIComponent).join('/');}
function labelStage(stage){return ({addin_initial_plan:'Add-in initial plan',used_range_resolved:'UsedRange resolved',
 overflow_pass_requested:'Overflow request',overflow_pass_result:'Overflow decision',overflow_expansion_stopped:'Expansion stopped',
 final_capture_bounds:'Capture bounds',capture_requested:'Capture request'})[stage]||String(stage||'Stage').replaceAll('_',' ');}
function buildModel(input){
 input=obj(input);const files=list(input.files),inventory=new Map(files.filter(f=>safePath(f.path)).map(f=>[f.path,f]));
 const sessions=[],claimed=new Set(),warnings=list(input.warnings).slice();
 const documents=list(input.documents).slice().sort((a,b)=>Number(!!a.progress)-Number(!!b.progress));
 const processed=new Set();
 const sourceRoot=path=>String(path||'').replace(/^data\//,'artifacts/').replace(/\.json$/,'').replace(/\/_progress\/.*$/,'');
 function resolveImage(relative,source){if(!relative||!safePath(relative))return null;if(inventory.has(relative))return relative;
   const hits=files.filter(f=>f.path.endsWith('/'+relative)&&safePath(f.path));
   const prefix=sourceRoot(source)+'/';const preferred=hits.filter(f=>f.path.startsWith(prefix));
   if(preferred.length===1)return preferred[0].path;
   return hits.length===1?hits[0].path:null;
 }
 for(const document of documents){
  const payloads=document.progress?[document.progress]:list(document.workbooks);
  for(const payload of payloads){
   const w=obj(payload),safe=obj(w.safe_capture||(document.progress?w:null)),name=w.workbook||w.name||safe.workbook||'Unidentified workbook';
   const key=name+'|'+sourceRoot(document.path);
   if(processed.has(key))continue;processed.add(key);
   const rust=w.precomputed_wsl_rust_visual_plan||safe.precomputed_wsl_rust_visual_plan||list(input.plans).flatMap(p=>list(obj(p.document).workbooks)).find(x=>x.workbook===name)||{};
   const sheetMap=new Map(),catalog=list(safe.sheet_catalog),catalogByName=new Map(catalog.map(x=>[x.name,x]));
   const rustByName=new Map(list(rust.sheets).map(x=>[x.sheet,x]));
   function sheet(name){name=typeof name==='string'?name:'Unassigned';if(!sheetMap.has(name)){
      const r=rustByName.get(name)||{},id=catalogByName.get(name)||{};
      sheetMap.set(name,{name,index:number(id.sheet_index)??number(r.sheet_index),
       index_basis:number(id.sheet_index)!==null?'Office worksheet position + 1':r.sheet_index_basis||
         (number(r.sheet_index)!==null?'Rust plan: visible worksheet order (legacy)':'Not recorded'),
       office_position:number(id.office_position),worksheet_id:id.worksheet_id,show_gridlines:id.show_gridlines,
       rust:r,status:'not-captured',bounds:[],passes:[],images:[],history:[],calibrations:[],tile_plans:[],sparse_checks:[],issues:[],source:document.path});
    }return sheetMap.get(name);}
   for(const x of catalog)sheet(x.name);
   for(const x of list(rust.sheets))sheet(x.sheet);
   for(const item of list(safe.sheets)){
    const sh=sheet(item.sheet);sh.record=item;sh.status=item.status||'not-recorded';sh.passes=list(item.overflow_passes);
    if(number(item.sheet_index)!==null){sh.index=item.sheet_index;sh.index_basis='Recorded worksheet index (1-based)';}
    if(item.warning)sh.issues.push(item.warning);if(item.error)sh.issues.push(`${item.error.code||'Error'}: ${item.error.message||''}`);
   }
   const historyKeys=new Set();
   function remember(entry){if(!entry||typeof entry.sheet!=='string')return;
     const token=JSON.stringify([entry.sheet,entry.stage,entry.timestamp_utc,entry.pass,entry.address,entry.bounds]);
     if(!historyKeys.has(token)){historyKeys.add(token);sheet(entry.sheet).history.push(entry);}}
   for(const entry of list(safe.bounds_history))remember(entry);
   // Events are acknowledged before the progress snapshot. A crash between them must
   // not hide the latest requested bounds. Never merge an ambiguous other host run.
   for(const event of list(input.events)){
     const e=obj(event.detail),source=event.source_file||event.file||'';
     const sameWorkbook=e.workbook===name;
     const peers=documents.filter(x=>!x.progress&&list(x.workbooks).some(y=>y.workbook===name));
     const sameSource=source?sourceRoot(String(source).replace(/\.events\.jsonl$/i,''))===sourceRoot(document.path):peers.length<=1;
     if(event.stage==='bounds_stage'&&sameWorkbook&&sameSource)remember(e);
   }
   for(const sh of sheetMap.values())sh.history.sort((a,b)=>String(a.timestamp_utc||'').localeCompare(String(b.timestamp_utc||'')));
   for(const plan of list(w.capture_plans))sheet(plan.sheet).plan=plan;
   for(const x of list(safe.calibrations))sheet(x.sheet).calibrations.push(x);
   for(const x of list(safe.tile_plans))sheet(x.sheet).tile_plans.push(x);
   for(const x of list(safe.sparse_overflow_checks))sheet(x.sheet).sparse_checks.push(x);
   const imagesByPath=new Map();
   function addImage(item,sh,kind,priority){
    if(!item||typeof item!=='object')return;
    const relative=item.relative_png||item.raw_png;
    const path=resolveImage(relative,document.path);if(!path){if(relative)sh.issues.push('Image reference unavailable or ambiguous: '+relative);return;}
    const current=imagesByPath.get(path);
    if(current&&current.priority>priority){imagesByPath.set(path,{...item,...current});return;}
    const f=inventory.get(path),png=obj(item.png),g=item.geometry||item.geometry_after||item.geometry_before;
    const merged={...(current||{}),...item,path,kind,priority,sheet:sh.name,
      width:item.png_width||png.width||current?.width||f.width||null,height:item.png_height||png.height||current?.height||f.height||null,
      geometry:g||current?.geometry,bytes:item.png_bytes||f.bytes,source:document.path};
    imagesByPath.set(path,merged);claimed.add(path);
   }
   for(const capture of list(safe.captures))addImage(capture,sheet(capture.sheet),'probe',1);
   for(const attempt of list(safe.attempts))addImage(attempt,sheet(attempt.sheet),attempt.error?'failed':'probe',attempt.error?3:0);
   for(const record of list(safe.sheets))for(const tile of list(record.tiles))
     addImage(tile,sheet(record.sheet),record.status==='complete'?'final':'retained',4);
   for(const cal of list(safe.calibrations))for(const sample of list(cal.observations))
     addImage({...sample,raster_profile:{scale_x:cal.x?.scale,scale_y:cal.y?.scale}},sheet(cal.sheet),'calibration',5);
   if(!list(safe.captures).length)for(const capture of list(w.captures))addImage(capture,sheet(capture.sheet),'diagnostic',1);
   for(const probe of list(w.probes))if(probe.relative_png)addImage(probe,sheet(probe.sheet),probe.error?'failed':'diagnostic',2);
   for(const check of list(safe.sparse_overflow_checks))for(const artifact of list(check.artifacts))
     addImage({...artifact,address:check.address,check_id:check.id,restoration:check.restoration,verdict:check.verdict},sheet(check.sheet),'verification',6);
   for(const im of imagesByPath.values())sheet(im.sheet).images.push(im);
   for(const sh of sheetMap.values()){
    if(!sh.record&&sh.images.length)sh.status='captured';
    const r=sh.rust,rec=sh.record||{},plan=sh.plan||{},initial=rec.initial_bounds||sh.history.find(h=>h.stage==='addin_initial_plan')?.bounds||{};
    sh.bounds.push({stage:'Rust cell / merge envelope',address:envelope(r.content_bottom_row,r.content_right_column),
      source:'OOXML content and merge scan',note:'Envelope anchored at A1; not a pixel-tight visible-content bound.'});
    if(Object.keys(r).length){
     sh.bounds.push({stage:'Rust drawing envelope',address:r.drawing_anchor_count?envelope(r.drawing_bottom_row,r.drawing_right_column):null,
       source:'OOXML drawing anchors',note:r.drawing_anchor_count?`${r.drawing_anchor_count} anchors; fallback ${r.drawing_fallback_required?'required':'not required'}`:'No drawing anchors recorded'});
     sh.bounds.push({stage:'Rust selected bounds',address:envelope(r.bottom_row,r.base_right_column),source:`right: ${r.right_bound_source||'?'}; bottom: ${r.bottom_bound_source||'?'}`,
       note:`Trailing hidden suffix: column ${r.trailing_hidden_column_start==null?'none':col(r.trailing_hidden_column_start)}, row ${r.trailing_hidden_row_start==null?'none':r.trailing_hidden_row_start+1}`});
    }
    sh.bounds.push({stage:'Add-in initial request',address:initial.address||rec.requested_address||plan.initial_address||null,
      source:'Add-in plan after drawing fallback / initial probe margin',note:initial.address?'Recorded before capture':rec.requested_address?'Recorded request; intermediate stages may not be available':'Not recorded before capture in this run'});
    const last=sh.history.filter(h=>h.stage==='final_capture_bounds').at(-1);
    sh.bounds.push({stage:sh.status==='complete'?'Final logical bounds':'Retained / last known bounds',address:last?.address||rec.address||plan.address||null,
      source:'Add-in overflow / capture result',note:sh.status==='complete'?'Complete according to capture result':'Not proof of complete worksheet coverage'});
    sh.extent=last?.extent_points||rec.extent_points;
    const base=range(envelope(r.bottom_row,r.base_right_column)),end=range(rec.address||plan.address);
    if(base&&end&&end.c1>base.c1){sh.expansion_columns=end.c1-base.c1;sh.expansion_ratio=(end.c1-end.c0)/(base.c1-base.c0);}
    if(sh.passes.length>1)sh.issues.push(`Repeated overflow expansion: ${sh.passes.length} passes. Inspect edge evidence and probe images.`);
    sh.images.sort((a,b)=>String(a.id||a.path).localeCompare(String(b.id||b.path),undefined,{numeric:true}));
    sh.issues=[...new Set(sh.issues)];
   }
   const sheets=[...sheetMap.values()].sort((a,b)=>(a.index??Infinity)-(b.index??Infinity)||a.name.localeCompare(b.name));
   sessions.push({key,name,status:w.status||safe.status||'not-recorded',source:document.path,sheets,
     options:safe.capture_options||{},tile_calls:list(safe.attempts).length,checks:list(safe.sparse_overflow_checks),timings:w.safe_capture_timings||null,environment:safe.environment||w.environment||{},current:safe.current||{},strategy:w.strategy||safe.strategy||document.meta?.strategy||null});
  }
 }
 const unassigned=files.filter(f=>/\.png$/i.test(f.path)&&!claimed.has(f.path)&&safePath(f.path));
 return {meta:input.meta||{},result:input.result||{},sessions,unassigned,files,warnings,
    image_count:files.filter(f=>/\.png$/i.test(f.path)).length,events:list(input.events)};
}
function mount(input,host){
 const m=buildModel(input),d=host.ownerDocument;
 const el=(tag,cls,content)=>{const n=d.createElement(tag);if(cls)n.className=cls;if(content!==undefined)n.textContent=text(content);return n;};
 const append=(p,...kids)=>{kids.flat().filter(Boolean).forEach(c=>p.append(c));return p;};
 function link(label,path,cls){const a=el('a',cls,label),url=safePath(path);if(url){a.href=url;a.target='_blank';a.rel='noopener';}else a.title='Unavailable or unsafe reference';return a;}
 function details(title,open=false){const x=el('details');x.open=open;append(x,el('summary','',title));return x;}
 function raw(label,data){const x=details(label);append(x,el('pre','json',JSON.stringify(data,null,2)));return x;}
 const format=(n,dec=0)=>number(n)!==null?n.toLocaleString('en-US',{maximumFractionDigits:dec}):'Not recorded';
 const badge=(value)=>el('span','badge '+String(value||'unknown').replace(/[^a-z-]/g,''),value||'Not recorded');
 const frame=el('div','layout'),nav=el('nav','sidebar'),main=el('main','main');append(host,frame);append(frame,nav,main);
 append(nav,el('div','brand','EXCEL SHRINK'),el('h2','','Visual validation'));
 const home=el('a','nav-link','Run overview');home.href='#overview';append(nav,home);
 const header=el('header','hero');header.id='overview';append(main,header);
 append(header,el('div','eyebrow',m.meta.experiment||'Visual validation'),el('h1','','Capture report'));
 append(header,el('p','run-id',m.meta.id),badge(m.result.status||m.meta.status));
 const actions=el('div','actions');append(header,actions,el('p','muted','Bounds provenance, capture evidence and diagnostics. Original artifacts remain unchanged.'));
 append(actions,link('Review bundle','review.zip','button primary'),link('Run data','result.json','button'),link('Timeline','timeline.jsonl','button'));
 const overviewLink=el('a','button','All runs');overviewLink.href='../../index.html';append(actions,overviewLink);
 const search=el('input','search');search.type='search';search.placeholder='Filter by workbook, worksheet or range';search.setAttribute('aria-label',search.placeholder);append(nav,search);
 const counts=el('div','metrics');append(header,counts);
 const sheets=m.sessions.flatMap(s=>s.sheets),stats=[['Worksheets',sheets.length],['Complete',sheets.filter(s=>s.status==='complete').length],['Attention',sheets.filter(s=>!['complete','captured','not-captured'].includes(s.status)).length],['Images',m.image_count]];
 for(const [label,v]of stats)append(counts,append(el('div','metric'),el('strong','',format(v)),el('span','',label)));
 const compared=m.sessions.filter(s=>s.options.overflow_mode);
 if(compared.length){
  append(main,el('h2','','Capture variants'));
  const table=el('table','bounds-table'),body=el('tbody');append(main,append(el('div','table-scroll'),table));
  append(table,append(el('thead'),append(el('tr'),['Variant / workbook','State','Tile calls','Verification calls','Blocker cells','Write / restore','Render','Analysis (inclusive)'].map(v=>el('th','',v)))),body);
  for(const s of compared){const calls=s.checks.reduce((n,c)=>n+list(c.artifacts).filter(a=>a.role!=='xor').length,0),cells=s.checks.filter(c=>c.restoration!=='not-required').reduce((n,c)=>n+(c.blocker_count||0),0),ms=k=>s.checks.reduce((n,c)=>n+(c.timings?.[k]||0),0);
    const label=(s.options.overflow_mode==='selective'?'B · Selective blockers':'A · Pixel heuristic')+` · ${s.options.tile_megapixels||16} MP · ${s.options.tile_packing||'balanced'}`;
    append(body,append(el('tr'),append(el('td'),el('strong','',label),el('div','muted',s.name)),append(el('td'),badge(s.status)),el('td','',format(s.tile_calls)),el('td','',format(calls)),el('td','',format(cells)),el('td','',`${format(ms('write_ms'),1)} / ${format(ms('restore_ms'),1)} ms`),el('td','',`${format(s.timings?.render_ms,1)} ms`),el('td','',`${format(s.timings?.analysis_ms,1)} ms`)));
  }
  append(main,el('p','muted','A/B uses separate working copies. Fixed A-then-B order does not control cache/order effects. Analysis includes selective verification time; timing columns overlap and must not be added. A successful capture is not yet a visual regression comparison.'));
 }
 if(m.warnings.length)append(header,raw('Report warnings',m.warnings));
 const errors=list(m.result.errors);if(errors.length)append(header,raw(`Run errors (${errors.length})`,errors));
 const observer=typeof IntersectionObserver!=='undefined'?new IntersectionObserver(entries=>{
   for(const item of entries){const im=item.target;if(item.isIntersecting){if(!im.hasAttribute('src')){im.src=im.dataset.src;im.dataset.active='true';}}
     else if(im.hasAttribute('src')){im.removeAttribute('src');delete im.dataset.active;}}
 },{rootMargin:'160px 0px',threshold:0}):null;
 const viewer=el('dialog','image-viewer');viewer.setAttribute('aria-label','Capture inspector');
 const viewerTitle=el('strong','viewer-title'),viewerMode=el('select'),viewerBody=el('div','viewer-body'),viewerImage=el('img','viewer-image');
 viewerMode.setAttribute('aria-label','Image scale');
 for(const [value,label]of [['1','100%'],['2','200%'],['fit','Fit width']]){const op=el('option','',label);op.value=value;append(viewerMode,op);}
 const closeViewer=el('button','button','Close'),viewerOriginal=el('a','button','Open original PNG');
 const viewerBar=el('div','viewer-bar');append(viewerBar,viewerTitle,viewerMode,viewerOriginal,closeViewer);
 append(viewerBody,viewerImage);append(viewer,viewerBar,viewerBody);append(host,viewer);
 let viewedCapture=null;
 function scaleViewer(){if(!viewedCapture)return;const fit=viewerMode.value==='fit';
   viewerImage.style.width=fit?'100%':`${(viewedCapture.width||viewerImage.naturalWidth)*Number(viewerMode.value)}px`;
   viewerImage.style.height='auto';}
 function openViewer(c){viewedCapture=c;viewerTitle.textContent=`${JSON.stringify(c.sheet)} · ${c.address||'Capture'} · ${format(c.width)} × ${format(c.height)} px`;
   viewerOriginal.href=safePath(c.path);viewerMode.value='1';viewerImage.alt=viewerTitle.textContent;viewerImage.src=safePath(c.path);
   scaleViewer();viewer.showModal();viewerBody.scrollTop=0;viewerBody.scrollLeft=0;}
 viewerMode.addEventListener('change',scaleViewer);viewerImage.addEventListener('load',scaleViewer);
 closeViewer.addEventListener('click',()=>viewer.close());viewer.addEventListener('close',()=>{viewerImage.removeAttribute('src');viewedCapture=null;});
 function picture(c){
  const card=el('article','capture');card.dataset.captureId=c.id||'';
  const head=el('div','capture-head');append(card,head);append(head,el('code','',c.address||'Range not recorded'),badge(c.kind));
  append(card,el('div','capture-meta',`${c.id||''}${c.role?' · '+c.role:''}  |  ${format(c.width)} × ${format(c.height)} px  |  ${format((c.bytes||0)/1024,1)} KiB`));
  const a=link('',c.path,'image-link'),img=el('img','capture-image');
  img.alt=`${c.sheet||''} ${c.address||''} · ${c.kind} · ${c.id||''}`;img.loading='lazy';img.decoding='async';
  if(number(c.width)>0)img.width=c.width;if(number(c.height)>0)img.height=c.height;
  const url=safePath(c.path);if(url){img.dataset.src=url;if(observer)observer.observe(img);else img.src=url;}
  img.addEventListener('load',()=>card.classList.remove('image-unavailable'));
  img.addEventListener('error',()=>{if(img.getAttribute('src'))card.classList.add('image-unavailable');});
  append(a,img);a.addEventListener('click',event=>{event.preventDefault();openViewer(c);});
  const inspect=el('button','inspect-button','Inspect image');inspect.addEventListener('click',()=>openViewer(c));
  append(card,a,append(el('div','image-actions'),inspect,link('Open original PNG',c.path,'small-link')));
  if(c.valid_rect_px)append(card,el('div','capture-meta',`Valid rectangle: x=${c.valid_rect_px.x}, y=${c.valid_rect_px.y}, ${c.valid_rect_px.width} × ${c.valid_rect_px.height} px`));
  if(c.sheet_offset_px)append(card,el('div','capture-meta',`Sheet offset: x=${c.sheet_offset_px.x}, y=${c.sheet_offset_px.y} px`));
  const det=details('Capture details');append(det,el('pre','json',JSON.stringify({address:c.address,logical_address:c.logical_address,rect:c.rect,geometry_points:c.geometry,
    raster_profile:c.raster_profile,packing:c.packing,role:c.role,restoration:c.restoration,analysis:c.analysis,error:c.error,source:c.source,render_ms:c.render_ms,relative_png:c.path,sha256:c.sha256},null,2)));append(card,det);
  return card;
 }
 const sections=[];let index=0;
 for(const session of m.sessions){
  const group=el('section','workbook');append(main,group);
  append(group,el('div','eyebrow','WORKBOOK'),el('h2','workbook-name',session.name),el('p','muted',`Strategy ${session.strategy||'?'} · ${session.options.overflow_mode==='selective'?'B · Selective blockers':session.options.overflow_mode==='heuristic'?'A · Pixel heuristic':'Legacy configuration'} · ${session.source}`));
  if(session.timings)append(group,raw('Timing and render environment',{timings_ms:session.timings,environment:session.environment}));
  append(nav,el('div','nav-workbook',`${session.options.overflow_mode==='selective'?'B · ':session.options.overflow_mode==='heuristic'?'A · ':''}${session.name}`));
  for(const sh of session.sheets){
    const id='worksheet-'+(++index),sec=el('section','worksheet');sec.id=id;const navItem=el('a','nav-link',`${sh.index==null?'?':String(sh.index).padStart(2,'0')} · ${JSON.stringify(sh.name)}`);
    navItem.href='#'+id;append(nav,navItem);append(group,sec);
    const title=el('div','sheet-title');append(sec,title);append(title,el('h3','',`Sheet ${sh.index==null?'?':String(sh.index).padStart(2,'0')} · ${JSON.stringify(sh.name)}`),badge(sh.status));
    append(sec,el('p','muted',`Index source: ${sh.index_basis}${sh.office_position!=null?` · Office position: ${sh.office_position} (zero-based)`:''}${sh.show_gridlines!=null?` · Gridlines: ${sh.show_gridlines?'visible':'hidden'}`:''}`));
    if(sh.expansion_columns)append(sec,el('p','callout',`Capture extends ${format(sh.expansion_columns)} columns beyond the Rust base (${format(sh.expansion_ratio,2)}× the base width in columns).`));
    for(const issue of sh.issues)append(sec,el('p','callout warning',issue));
    append(sec,el('h4','','Bounds provenance'));
    const table=el('table','bounds-table');append(sec,el('div','table-scroll'));sec.lastChild.append(table);
    append(table,append(el('thead'),append(el('tr'),['Stage','Range','Source / decision'].map(x=>el('th','',x)))));
    const body=el('tbody');append(table,body);
    for(const b of sh.bounds){const row=el('tr');append(row,el('td','',b.stage),append(el('td'),el('code','',b.address||'Not recorded')),append(el('td'),el('div','',b.source),el('small','muted',b.note)));append(body,row);}
    if(sh.extent)append(sec,el('p','geometry',`Boundary positions (pt): left ${format(sh.extent.left,3)} · top ${format(sh.extent.top,3)} · right ${format(sh.extent.right,3)} · bottom ${format(sh.extent.bottom,3)}`));
    if(sh.rust.bounds_evidence){const ev=sh.rust.bounds_evidence;
      const refs=[['Rightmost content',ev.rightmost_content],['Bottommost content',ev.bottommost_content],
        ['Rightmost merge',ev.rightmost_merge],['Bottommost merge',ev.bottommost_merge]];
      const witnesses=el('p','geometry');witnesses.textContent=refs.filter(x=>x[1]).map(([label,w])=>`${label}: ${w.reference} (${w.kind})`).join(' · ')||'No content or merge witness recorded.';
      append(sec,witnesses,raw('Rust scan evidence',ev));}
    else if(Object.keys(sh.rust).length)append(sec,el('p','muted','Boundary witness references were not recorded in this run. The original Rust plan is available below.'));
    if(sh.passes.length){
      const t=el('table','passes-table'),tb=el('tbody');append(sec,el('h4','','Overflow passes'),append(el('div','table-scroll'),t));
      append(t,append(el('thead'),append(el('tr'),['Pass','Requested range','Proposed last column','Tiles','Decision','Edge evidence'].map(x=>el('th','',x)))),tb);
      for(const pass of sh.passes){const decision=pass.selective_unresolved?'Verification inconclusive':pass.horizontal_split?'Context split — inconclusive':pass.done?'Closed':'Expand';
       const cell=el('td');const evidence=list(pass.edge_evidence);if(evidence.length)append(cell,raw('Inspect evidence',evidence));else cell.textContent=pass.horizontal_split?'Not evaluated':'Not recorded';
       append(tb,append(el('tr'),el('td','',pass.pass),append(el('td'),el('code','',pass.address)),el('td','',col(pass.keep_column)),el('td','',pass.tile_count),el('td','',decision),cell));}
    }
    if(sh.tile_plans.length){
      const p=sh.tile_plans.at(-1);
      append(sec,el('h4','','Tile budget'),el('p','geometry',`Packing: ${p.method||'recorded'} · tile budget: ${format((p.limits?.max_pixels||0)/1000000)} MP · edge ceiling: ${format(p.limits?.max_edge_px)} px · full width: ${format(p.full_predicted?.width)} px · allowed strip height: ${format(p.allowed_height_px)} px · planned tiles: ${format(p.tiles?.length)}`),raw('Tile planning evidence',sh.tile_plans));
    }
    if(sh.sparse_checks.length){
      append(sec,el('h4','','Selective overflow verification'));
      for(const c of sh.sparse_checks){const details=raw(`${c.id} · rows ${list(c.rows).join(', ')} · ${c.verdict||c.status} · restore: ${c.restoration}`,c);append(sec,details);}
      append(sec,el('p','muted','Before, blocked, restored and XOR images are in the Verification gallery. Unchanged pixels rule out blocker-sensitive text overflow only in the verified row region; they do not erase borders or other meaningful layout.'));
    }
    if(sh.history.length)append(sec,raw('Planning events (chronological)',sh.history));
    append(sec,raw('Recorded bounds and calibration',{rust:sh.rust,addin_initial:sh.record?.initial_bounds,addin_result:sh.plan,calibrations:sh.calibrations,
      source_file:sh.source,current:session.current}));
    const gallery=details(`Screenshots (${sh.images.length})`,index===1),tools=el('div','gallery-tools'),grid=el('div','gallery');append(sec,gallery);append(gallery,tools,grid);
    const select=el('select');select.setAttribute('aria-label','Screenshot category');const countKind=kind=>sh.images.filter(x=>kind==='all'||(kind==='final'?['final','retained'].includes(x.kind):x.kind===kind)).length;
    for(const [v,label]of [['final','Final / retained'],['probe','Overflow probes'],['verification','Verification'],['calibration','Calibration'],['failed','Failed attempts'],['diagnostic','Diagnostics'],['all','All images']]){
     const op=el('option','',`${label} (${countKind(v)})`);op.value=v;append(select,op);}
    select.value=countKind('final')?'final':'all';append(tools,el('label','','View'),select,el('span','muted','Original PNGs load near the viewport. Open an image for full resolution.'));
    let drawn=false;
    function draw(){for(const im of grid.querySelectorAll('img'))if(observer)observer.unobserve(im);grid.replaceChildren();const mode=select.value;
       const matching=sh.images.filter(x=>mode==='all'||(mode==='final'?['final','retained'].includes(x.kind):x.kind===mode));
       matching.forEach(c=>append(grid,picture(c)));if(!matching.length)append(grid,el('p','muted','No images in this category.'));drawn=true;}
    gallery.addEventListener('toggle',()=>{if(gallery.open&&!drawn)draw();if(!gallery.open){for(const im of grid.querySelectorAll('img')){if(observer)observer.unobserve(im);im.removeAttribute('src');}grid.replaceChildren();drawn=false;}});
    select.addEventListener('change',draw);navItem.addEventListener('click',()=>{sec.hidden=false;gallery.open=true;if(!drawn)draw();});if(gallery.open)draw();
    sections.push({sec,navItem,searchText:(session.name+' '+sh.name+' '+sh.index+' '+sh.bounds.map(x=>x.address).join(' ')+' '+sh.images.map(x=>x.address).join(' ')).toLowerCase()});
  }
 }
 search.addEventListener('input',()=>{const q=search.value.toLowerCase();for(const x of sections){const hide=q&&!x.searchText.includes(q);x.sec.hidden=!!hide;x.navItem.hidden=!!hide;}});
 if(m.unassigned.length){const det=details(`Unassigned images (${m.unassigned.length})`);append(main,det,el('p','muted','No worksheet association was recorded for these artifacts.'));
  let loaded=false;det.addEventListener('toggle',()=>{if(det.open&&!loaded){loaded=true;for(const f of m.unassigned)append(det,link(f.path,f.path,'file-link'));}});}
 const artifacts=details('Result files and logs');append(main,artifacts);
 for(const f of m.files.filter(f=>!f.path.endsWith('.png')&&!f.path.startsWith('report-assets/')&&!f.path.endsWith('.previous')))
  append(artifacts,link(f.path,f.path,'file-link'));
 append(main,el('footer','muted','Excel Shrink · Visual validation · Offline report. Screenshots and diagnostic logs may contain workbook data and local paths.'));
 return m;
}
const api={buildModel,mount,safePath,range,col,envelope};if(typeof module!=='undefined'&&module.exports)module.exports=api;
root.VVReport=api;
if(typeof document!=='undefined'){const start=()=>{const data=document.getElementById('vv-report-data'),host=document.getElementById('vv-report');if(!data||!host)return;
 try{mount(JSON.parse(data.textContent),host);document.getElementById('vv-loading')?.remove();}catch(e){const p=document.createElement('pre');p.textContent='Report rendering failed. Open result.json or the retained logs.\n'+String(e.stack||e);host.append(p);}};
 if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',start);else start();}
}(typeof window!=='undefined'?window:globalThis));
