/* Offline run history. All run labels are text, all artifact paths are validated. */
(function(root){'use strict';
const array=x=>Array.isArray(x)?x:[];
function safePath(x){if(typeof x!=='string'||!x||/[\x00-\x1f\\:?#]/.test(x)||x.startsWith('/'))return null;const parts=x.split('/');return parts.some(p=>!p||p==='.'||p==='..')?null:parts.map(encodeURIComponent).join('/');}
function normalize(data){return array(data.runs).map(x=>({...x,status:String(x.status||'unknown'),experiment:String(x.experiment||'Unspecified'),id:String(x.id||''),errors:array(x.errors)})).sort((a,b)=>b.id.localeCompare(a.id));}
function filterRuns(runs,query,status,experiment){const q=query.toLowerCase();return runs.filter(r=>(!status||r.status===status)&&(!experiment||r.experiment===experiment)&&(!q||[r.id,r.experiment,JSON.stringify(r.arguments),JSON.stringify(r.errors)].join(' ').toLowerCase().includes(q)));}
function mount(data,host){const d=host.ownerDocument,runs=normalize(data),el=(tag,cls,text)=>{const e=d.createElement(tag);if(cls)e.className=cls;if(text!==undefined)e.textContent=String(text);return e;},add=(p,...cs)=>{for(const c of cs.flat())if(c)p.append(c);return p;};
 const link=(label,path,cls='button')=>{const a=el('a',cls,label),url=safePath(path);if(url)a.href=url;else a.setAttribute('aria-disabled','true');return a;};
 const badge=s=>el('span','badge '+s.replace(/[^a-z-]/g,''),s);
 const frame=el('div','layout'),nav=el('nav','sidebar'),main=el('main','main');add(host,frame);add(frame,nav,main);
 add(nav,el('div','brand','EXCEL SHRINK'),el('h2','','Visual validation'),el('p','muted','Capture evidence. Engineering decisions.'));
 const jump=el('a','nav-link','Run history');jump.href='#runs';add(nav,jump);
 const guide=el('a','nav-link','Experiment map');guide.href='#experiments';add(nav,guide);
 const hero=el('header','hero');add(main,hero,el('div','history-space'));add(hero,el('div','eyebrow','ENGINEERING WORKSPACE'),el('h1','','Run history'),el('p','history-intro','Explore results, compare capture methods and inspect the evidence behind each run.'));
 const stats=el('div','metrics');add(hero,stats);
 for(const [label,n]of [['Runs',runs.length],['Complete',runs.filter(r=>r.status==='complete').length],['Needs review',runs.filter(r=>!['complete','running'].includes(r.status)).length],['Running',runs.filter(r=>r.status==='running').length]])add(stats,add(el('div','metric'),el('strong','',n),el('span','',label)));
 if(runs.length){const last=runs[0];add(hero,add(el('div','actions'),link('Open latest report',last.report,'button primary'),last.bundle?link('Latest review bundle',last.bundle):null));}
 add(nav,el('p','nav-footnote','Read-only catalog. Use the launcher to prune history or recover interrupted runs. Input workbooks and baselines are not retention targets.'));
 const panel=el('section','history-panel');panel.id='runs';add(main,panel);
 const bar=el('div','history-toolbar'),search=el('input','search'),status=el('select'),experiment=el('select');
 search.type='search';search.placeholder='Search runs, workbooks or errors';search.setAttribute('aria-label',search.placeholder);
 function choices(select,label,items){select.setAttribute('aria-label',label);const first=el('option','',label);first.value='';add(select,first);for(const s of [...new Set(items)].sort()){const op=el('option','',s);op.value=s;add(select,op);}}
 choices(status,'All statuses',runs.map(r=>r.status));choices(experiment,'All experiments',runs.map(r=>r.experiment));
 add(bar,search,status,experiment);const count=el('p','muted'),grid=el('div','history-list');add(panel,el('h2','','Recorded runs'),bar,count,grid);
 function render(){const filtered=filterRuns(runs,search.value,status.value,experiment.value);count.textContent=`${filtered.length} of ${runs.length} runs · most recent first`;grid.replaceChildren();
  if(!filtered.length){add(grid,add(el('div','history-empty'),el('h3','',runs.length?'No matching runs':'No runs recorded yet'),el('p','muted',runs.length?'Adjust the filters to see other results.':'Run an experiment from the visual-validation launcher. Its report will appear here.')));return;}
  for(const r of filtered){const card=el('article','history-card'),heading=el('div','history-card-heading');add(card,heading);add(heading,el('h3','',r.experiment),badge(r.status));
   const when=new Date(r.started_utc);add(card,el('p','run-id',r.id),el('p','muted',Number.isFinite(when.getTime())?when.toLocaleString('en-GB',{dateStyle:'medium',timeStyle:'medium'})+' · local time':'Start time not recorded'));
   const args=r.arguments||{},chips=el('div','history-chips');for(const k of ['Strategy','Implementation','OverflowMode','TileMegapixels','TilePacking','Workbook','Suite'])if(args[k]!=null){const value=String(args[k]),display=k==='Workbook'?value.split(/[\\/]/).pop():value;const chip=el('span','history-chip',`${k.replace(/([a-z])([A-Z])/g,'$1 $2')}: ${display}`);chip.title=value;add(chips,chip);}add(card,chips);
   if(r.rollup)add(card,el('p','muted',`${r.rollup.complete_sheets??0} complete worksheets · ${r.rollup.accepted_tiles??0} retained tiles · ${(r.rollup.failed_sheets??0)+(r.rollup.inconclusive_sheets??0)} require review`));
   if(r.status!=='complete'&&r.rollup?.complete_sheets>0&&!r.rollup.failed_sheets&&!r.rollup.inconclusive_sheets&&!r.rollup.pending_sheets)add(card,el('p','muted','Recorded sheets completed; inspect the run-level error below.'));
   if(r.errors.length){const raw=r.errors[0],message=typeof raw==='string'?raw:raw.message||raw.reason||JSON.stringify(raw);add(card,el('p','history-error',message));}
   if(r.warning)add(card,el('p','history-error',r.warning));
   add(card,add(el('div','actions'),link('View report',r.report,'button primary'),r.bundle?link('Review bundle',r.bundle):el('span','muted','Bundle pending')));add(grid,card);
  }
 }
 search.addEventListener('input',render);status.addEventListener('change',render);experiment.addEventListener('change',render);render();
 const catalog=el('section','history-panel');catalog.id='experiments';add(main,catalog,el('footer','muted',`Offline report catalog · Generated ${data.generated_utc||'time not recorded'} · Historical results remain unchanged.`));add(catalog,el('div','eyebrow','EXPERIMENT MAP'),el('h2','','What each experiment answers'));
 const table=el('table','bounds-table'),body=el('tbody');add(catalog,add(el('div','table-scroll'),table));add(table,add(el('thead'),add(el('tr'),['Experiment','Question','Current scope'].map(t=>el('th','',t)))),body);
 for(const row of [
 ['Drawing bounds','Which execution model obtains shape bounds reliably and quickly?','VBA, C# add-ins, Office.js and external COM. Implementation choice remains an engineering comparison.'],
 ['Get picture · A–C','How much do bounds discovery and image capture cost?','UsedRange or content/drawing bounds. Calibrated tiling; persistent image evidence.'],
 ['Get picture · D/E','Can a before/blocked XOR pair determine overflow?','Legacy bounded blocker column / row. Retained for comparison; large probes are guarded.'],
 ['Get picture · F · A/B','Does row-selective causal verification avoid false expansions?','A: current pixel heuristic. B: one blocker per candidate row, with verified restoration. Separate original-workbook copies.'],
 ['Height plateau','Where does the native renderer saturate or clip?','Explicit diagnostic. Not the production tile budget.'],
 ['Units / markers / row heights','How do XML, Excel geometry and PNG pixels relate?','Synthetic controls. Raster scale is calibrated per run; exact XML conversion is not assumed.']])add(body,add(el('tr'),row.map(t=>el('td','',t))));
 add(catalog,el('p','callout','Production policy is separate: Rust content bounds, a reasonable minimum screenshot width, and persistent screenshots. Exact-overflow experiments do not silently replace that policy.'));
 return {runs,render};
}
const api={normalize,filterRuns,safePath,mount};root.VVHistory=api;if(typeof module!=='undefined'&&module.exports)module.exports=api;
if(typeof document!=='undefined'){const start=()=>{const host=document.getElementById('vv-history'),node=document.getElementById('vv-history-data');if(!host||!node)return;try{mount(JSON.parse(node.textContent),host);}catch(e){host.textContent='History rendering failed. Individual reports remain available under runs/. '+String(e);}};document.readyState==='loading'?document.addEventListener('DOMContentLoaded',start):start();}
}(typeof window!=='undefined'?window:globalThis));
