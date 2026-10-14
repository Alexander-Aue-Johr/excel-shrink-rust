/* Offline suite overview. Source workbooks and result JSON remain authoritative. */
(function(root){'use strict';
const arr=x=>Array.isArray(x)?x:[], finite=x=>typeof x==='number'&&Number.isFinite(x);
function median(xs){const a=xs.filter(finite).slice().sort((a,b)=>a-b);return a.length?(a[(a.length-1)>>1]+a[a.length>>1])/2:null;}
function stable(x){if(Array.isArray(x))return '['+x.map(stable).join(',')+']';if(x&&typeof x==='object')return '{'+Object.keys(x).sort().map(k=>JSON.stringify(k)+':'+stable(x[k])).join(',')+'}';return JSON.stringify(x??null);}
function path(s){if(typeof s!=='string'||!s||/[\x00-\x1f\\:?#]/.test(s)||s.startsWith('/'))return null;const p=s.split('/');return p.some(v=>!v||v==='.'||v==='..')?null:p.map(encodeURIComponent).join('/');}
function key(t){return stable([t.workbook,t.sheet,t.rect,t.width,t.height,t.offset]);}
function comparison(a,b){
 const aa=arr(a?.metrics?.captures),bb=arr(b?.metrics?.captures),am=new Map(),bm=new Map();let duplicates=false;
 for(const t of aa){const k=key(t);if(am.has(k))duplicates=true;am.set(k,t);}for(const t of bb){const k=key(t);if(bm.has(k))duplicates=true;bm.set(k,t);}
 if(duplicates)return {status:'invalid',note:'Duplicate tile identity in one result.',matched:0,total:Math.max(aa.length,bb.length)};
 if(!aa.length||!bb.length)return {status:'unverified',note:'No comparable final-capture records.',matched:0,total:Math.max(aa.length,bb.length)};
 const common=[...am.keys()].filter(k=>bm.has(k));let equal=0,different=0,unknown=0,rawEqual=0,rawCompared=0;
 for(const k of common){const x=am.get(k),y=bm.get(k),dx=x.pixel_digest,dy=y.pixel_digest;
  if(x.raw_sha256&&y.raw_sha256&&x.raw_address===y.raw_address){rawCompared++;if(x.raw_sha256===y.raw_sha256)rawEqual++;}
  if(x.pixel_issue||y.pixel_issue){unknown++;continue;}
  if(dx?.sha256&&dy?.sha256&&['vv-bgra32-v1','vv-bgra32-gdiplus-v1'].includes(dx.algorithm)&&dx.algorithm===dy.algorithm&&dx.width===dy.width&&dx.height===dy.height){if(dx.sha256===dy.sha256)equal++;else different++;}
  else if(x.encoded_sha256&&x.encoded_sha256===y.encoded_sha256){equal++;}
  else unknown++;
 }
 const sameLayout=common.length===aa.length&&common.length===bb.length;
 const status=!sameLayout?'different-layout':different?'pixel-mismatch':unknown?'unverified':'pixel-equivalent';
 return {status,matched:equal,total:Math.max(aa.length,bb.length),common:common.length,different,unknown,rawEqual,rawCompared,
  note:!sameLayout?'Bounds, raster size, tile boundaries or offsets differ. No composite image equivalence is inferred.':unknown?'Different PNG bytes are not a pixel mismatch. A decoded digest or identical file hash is required.':different?'Decoded final pixels differ for matching tiles. Inspect the case reports.':'All matching final tiles have identical encoded bytes or identical normalized BGRA32 digests. This is pairwise equivalence, not ground-truth completeness.'};
}
function environmentKey(job){const ws=arr(job.metrics?.workbooks).filter(w=>w.kind==='capture');if(!ws.length)return null;return stable(ws.map(w=>({workbook:w.workbook,office:w.environment?.office,screen:w.environment?.screen,gridlines:w.capture_view?.after,policy:w.capture_policy,scales:[...new Set(arr(job.metrics?.captures).filter(t=>t.workbook===w.workbook).map(t=>stable([t.raster?.scale_x,t.raster?.scale_y,t.raster?.border_px])))].sort()})));}
function summaries(state){
 const groups=new Map();for(const j of arr(state.jobs)){if(j.phase!=='measured')continue;if(!groups.has(j.candidate_id))groups.set(j.candidate_id,[]);groups.get(j.candidate_id).push(j);}
 return [...groups.entries()].map(([id,jobs])=>{const good=jobs.filter(j=>j.status==='complete'&&finite(j.metrics?.workload_ms));const times=good.map(j=>j.metrics.workload_ms),envs=[...new Set(good.map(environmentKey).filter(x=>x!==null))];return {environment_variants:envs.length,id,label:jobs[0].candidate?.label||id,group:jobs[0].candidate?.group,jobs,complete:good.length,total:jobs.length,median_ms:envs.length>1?null:median(times),min_ms:times.length?Math.min(...times):null,max_ms:times.length?Math.max(...times):null};});
}
function phase(job,name){const vals=arr(job.metrics?.workbooks).filter(w=>w.kind==='capture').map(w=>w.timings?.[name]).filter(finite);return vals.length?vals.reduce((a,b)=>a+b,0):null;}
function mount(data,host){const d=host.ownerDocument,el=(t,c,txt)=>{const x=d.createElement(t);if(c)x.className=c;if(txt!==undefined)x.textContent=String(txt);return x;},add=(p,...cs)=>{for(const c of cs.flat())if(c)p.append(c);return p;};
 const link=(text,p,c='button')=>{const x=el('a',c,text),s=path(p);if(s)x.href=s;else x.setAttribute('aria-disabled','true');return x;};
 const badge=s=>el('span','badge '+String(s||'unknown').replace(/[^a-z-]/g,''),s||'unknown'),sec=(id,title)=>{const x=el('section','suite-panel');x.id=id;add(x,el('h2','',title));add(main,x);return x;};
 const seconds=x=>finite(x)?(x/1000).toFixed(2)+' s':'—';
 const layout=el('div','layout'),nav=el('nav','sidebar'),main=el('main','main');add(host,add(layout,nav,main));
 add(nav,el('div','brand','EXCEL SHRINK'),el('h2','','Comparison suite'),el('p','muted','One plan. Isolated cases. Comparable evidence.'));
 for(const [id,label]of [['overview','Overview'],['candidates','Candidates'],['equivalence','Output equivalence'],['queue','Execution log'],['catalog','Experiment coverage'],['inputs','Input snapshot']]){const a=el('a','nav-link',label);a.href='#'+id;add(nav,a);}
 const history=el('a','nav-link','Run history');history.href='../../index.html';add(nav,history,el('p','nav-footnote','No experiments are removed automatically. Diagnostics and native baselines answer different questions.'));
 const hero=el('header','hero');hero.id='overview';add(main,hero);add(hero,el('div','eyebrow','ENGINEERING COMPARISON'),el('h1','','Visual validation suite'),el('p','suite-intro',data.description||'A configured sequence of independent experiments.'),badge(data.status),el('p','run-id',data.run_id));
 const jobs=arr(data.jobs),measured=jobs.filter(j=>j.phase==='measured'),warmups=jobs.filter(j=>j.phase==='warmup'),stats=el('div','metrics');add(hero,stats);
 for(const [label,n]of [['Cases',jobs.length],['Measured complete',measured.filter(j=>j.status==='complete').length],['Warm-ups',warmups.length],['Needs review',jobs.filter(j=>!['queued','running','complete'].includes(j.status)).length]])add(stats,add(el('div','metric'),el('strong','',n),el('span','',label)));
 add(hero,add(el('div','actions'),link('Review bundle · no images','review-metadata.zip','button primary'),link('Results CSV','case-results.csv'),link('Full result data','suite-state.json')));
 if(data.status==='running')add(hero,el('p','callout','The suite is running. Reload this local page after a case finishes to see its latest results.'));
 for(const e of arr(data.errors))add(hero,el('p','suite-error',typeof e==='string'?e:JSON.stringify(e)));
 const unchecked=measured.filter(j=>j.metrics?.pixel_verification_requested&&arr(j.metrics?.captures).some(c=>!c.pixel_digest||c.pixel_issue));
 if(unchecked.length)add(hero,el('p','callout',`${unchecked.length} measured cases have incomplete offline pixel verification. Capture completion and pixel equivalence are separate outcomes. Repair existing reports; no new Excel capture is required when the PNGs remain locally available.`));
 const failedReports=jobs.filter(j=>j.metrics?.report_publication?.status==='failed');
 if(failedReports.length)add(hero,el('p','suite-error',`${failedReports.length} case reports or bundles failed during post-processing. See their case-report-error.txt files.`));
 add(hero,el('p','muted','Cases run sequentially on frozen workbook copies. Warm-ups, workload time and offline pixel-verification time are separate. A shorter run does not establish equal visual coverage.'));
 function table(parent,headers,rows){const t=el('table','bounds-table'),body=el('tbody');add(t,add(el('thead'),add(el('tr'),headers.map(h=>el('th','',h)))),body);for(const row of rows)add(body,add(el('tr'),row.map(x=>add(el('td'),x?.nodeType?x:el('span','',x??'—')))));add(parent,add(el('div','table-scroll'),t));return t;}
 const candidates=sec('candidates','Candidate summary'),summary=summaries(data);
 add(candidates,el('p','muted','Successful measured cases only; failures remain in the completion denominator. Workload includes host setup/build, workbook launch and capture, but excludes offline final-pixel hashing and report generation. Compare matched cases, not unrelated diagnostics.'));
 table(candidates,['Configuration','Group','Complete','Median workload','Observed range','Evidence'],summary.map(g=>[g.label,g.group,`${g.complete}/${g.total}`,seconds(g.median_ms),g.complete?seconds(g.min_ms)+' – '+seconds(g.max_ms):'—',g.environment_variants>1?'Renderer state changed between repetitions; median withheld.':g.total<3?'Screening only; too few repeats for a stable choice.':'Repeated observations; check equivalence and input scope before retiring anything.']));
 const eq=sec('equivalence','Final-output equivalence');add(eq,el('p','muted','Same-round comparisons of current candidates. Different bounds or tiling are reported separately; the suite does not resize or stitch images to manufacture equality. Exact matching bytes are sufficient; otherwise normalized decoded pixel hashes are required.'));
 const pairs=[];for(const round of [...new Set(measured.map(j=>j.round))]){const r=measured.filter(j=>j.round===round&&j.candidate?.group==='current-capture'&&j.status==='complete');const ref=r.find(j=>j.candidate_id==='f-dotnet-selective')||r[0];if(!ref)continue;for(const j of r){if(j===ref)continue;pairs.push({ref,j,result:comparison(ref,j)});}}
 table(eq,['Round','Reference → candidate','Outcome','Matching final tiles','Identical raw bytes','Details'],pairs.map(p=>[p.j.round,p.ref.candidate_id+' → '+p.j.candidate_id,p.result.status,`${p.result.matched}/${p.result.total}`,p.result.rawCompared?`${p.result.rawEqual}/${p.result.rawCompared}`:'—',p.result.note]));
 if(!pairs.length)add(eq,el('p','muted','Pairwise capture evidence becomes available after at least two current candidates complete in the same measured round.'));
 add(eq,el('p','callout','Matching outputs show equivalence for the recorded input and renderer state, not correctness against an independent full-workbook oracle. A single hospital workbook is not sufficient grounds to delete a baseline.'));
 const queue=sec('queue','Execution log'),search=el('input','search');search.type='search';search.placeholder='Filter cases by name, status or phase';search.setAttribute('aria-label',search.placeholder);add(queue,search);const qbody=el('div');add(queue,qbody);
 function renderQueue(){qbody.replaceChildren();const query=search.value.toLowerCase();const selected=jobs.filter(j=>[j.id,j.status,j.phase,j.candidate?.label].join(' ').toLowerCase().includes(query));
  for(const j of selected){const card=el('article','suite-case');add(qbody,card);add(card,add(el('div','suite-case-head'),el('h3','',`${j.sequence}. ${j.candidate?.label||j.candidate_id}`),badge(j.status)));
   add(card,el('p','muted',`${j.phase==='warmup'?'Warm-up · excluded from statistics':'Measured'} · round ${j.round} · position ${j.position}`));
   if(j.error)add(card,el('p','suite-error',j.error));
   if(j.metrics){const wbs=arr(j.metrics.workbooks).filter(w=>w.kind==='capture');add(card,el('p','',`Workload ${seconds(j.metrics.workload_ms)} · offline pixel hashing ${seconds(j.metrics.pixel_verification_ms)}${wbs.length?' · '+wbs.reduce((n,w)=>n+(w.capture_count||0),0)+' final tiles':''}`));
    if(wbs.length){const timing=el('details'),ts=el('summary','','Phase timers and capture state');add(card,add(timing,ts));table(timing,['Geometry','Image render','Image persistence','Analysis','Telemetry','Final PNG export'],[[seconds(phase(j,'geometry_ms')),seconds(phase(j,'render_ms')),seconds(phase(j,'persist_ms')),seconds(phase(j,'analysis_ms')),seconds(phase(j,'telemetry_ms')),seconds(phase(j,'finalize_ms'))]]);
     add(timing,el('p','muted','Phase timers overlap. In particular, analysis includes selective interventions; these columns must not be added into a total.'));
     for(const w of wbs){add(timing,el('p','',`${w.workbook}: ${w.complete_sheets}/${w.sheet_count} sheets complete · ${w.verification_groups||0} verification groups · ${w.confirmed_blockers||0} blocker cells · native image batch ${w.actual_image_request_batch_size??'not recorded'}`));add(timing,el('pre','',JSON.stringify({office:w.environment?.office,screen:w.environment?.screen,capture_options:w.capture_options,gridlines:w.capture_view?.status},null,2)));}
    }
    const issues=arr(j.metrics.captures).filter(c=>c.pixel_issue);if(issues.length)add(card,el('p','suite-error',`${issues.length} final PNGs could not be independently hashed. See case-metrics.json.`));
   }
   if(j.report)add(card,add(el('div','actions'),link('Open case report',j.report,'button primary'),link('Case data',`cases/${j.id}/case-metrics.json`),link('Worker log',`cases/${j.id}/worker.stdout.log`)));
  }
 }
 search.addEventListener('input',renderQueue);renderQueue();
 const cat=sec('catalog','Experiment coverage');table(cat,['Experiment','Role','This profile','Question'],arr(data.catalog).map(c=>[c.label,c.scope,jobs.some(j=>j.candidate_id===c.id)?'Included':'Not selected',c.purpose]));
 add(cat,el('p','callout','Retirement means removing a candidate from routine execution only after repeated comparable results over a representative corpus. Keep source, reproduction cases and incident documentation until that evidence exists. The native shape base set remains one coupled invocation.'));
 const inputs=sec('inputs','Frozen input snapshot');table(inputs,['Workbook','Bytes','SHA-256'],arr(data.inputs).map(f=>[f.name,f.bytes,f.sha256]));add(inputs,link('Source fingerprints','inputs/source-fingerprints.json'),el('p','muted',data.ordering||''),el('p','muted','Transport is part of the tested host configuration: the Rust host uses local HTTP; .NET uses HTTPS. This is not a pure language-speed benchmark.'));
 return {summary,pairs};
}
const api={median,stable,path,comparison,summaries,environmentKey,phase,mount};root.VVSuite=api;if(typeof module!=='undefined'&&module.exports)module.exports=api;
if(typeof document!=='undefined'){const start=()=>{const h=document.getElementById('vv-suite'),n=document.getElementById('vv-suite-data');if(!h||!n)return;try{mount(JSON.parse(n.textContent),h);}catch(e){h.textContent='Suite view unavailable. Open suite-state.json for the persisted measurements. '+String(e);}};document.readyState==='loading'?document.addEventListener('DOMContentLoaded',start):start();}
})(typeof window!=='undefined'?window:globalThis);
