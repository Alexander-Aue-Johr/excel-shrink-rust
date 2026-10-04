/* Experimental sparse whitespace intervention. All captures operate on the runner's
 * disposable workbook. No mutation of a nonempty, merged, spilled or table cell.
 * A no-difference result is evidence only for verified candidate rows in this probe. */
(function(root){
'use strict';
const S=()=>root.SafeCapture;
const fail=(code,message,detail,fatal=false)=>S().fault(code,message,detail,fatal);
const bands=ys=>{const out=[];for(const y of ys){const p=out.at(-1);if(p&&y<=p[1]+1)p[1]=y;else out.push([y,y]);}return out;};
function scanCandidates(image,edges,base,end,background){
 const x0=Math.max(1,Math.ceil(edges[base+1])+1),x1=Math.min(image.width-1,Math.floor(edges[end+1])-1);
 const ys=[];let scanned=0;
 // This intentionally sees the signal BEFORE the line heuristic. Otherwise B could
 // never test the known line incident when A's corrected heuristic already filters it.
 for(let y=1;y<image.height-1;y++)for(let x=x0;x<x1;x++){
   scanned++;const o=(y*image.width+x)*4,d=image.data;
   if(d[o+3]>24&&(Math.abs(d[o]-background[0])>18||Math.abs(d[o+1]-background[1])>18||Math.abs(d[o+2]-background[2])>18)) {ys.push(y);break;}
 }
 return {pixel_bands:bands(ys),pixel_rows:ys.length,scanned_pixels:scanned,probe_x0:x0,probe_x1:x1};
}
function pixelDiff(before,after,x0,x1,allowedRows){
 if(before.width!==after.width||before.height!==after.height)throw fail('SPARSE_IMAGE_SIZE_CHANGED','Intervention image dimensions differ.',{},false);
 const a=before.data,b=after.data,out=new Uint8ClampedArray(a.length);let changed=0,outside=0,right=-1;
 const perRow=new Map(allowedRows.map(r=>[r.row,{row:r.row,changed_pixels:0,rightmost_changed_x:-1}]));
 for(let y=0;y<before.height;y++){
  const row=allowedRows.find(r=>y>=r.y0&&y<r.y1);
  for(let x=0;x<before.width;x++){
   const i=(y*before.width+x)*4;
   if(a[i]===b[i]&&a[i+1]===b[i+1]&&a[i+2]===b[i+2]&&a[i+3]===b[i+3])continue;
   out[i]=a[i]^b[i];out[i+1]=a[i+1]^b[i+1];out[i+2]=a[i+2]^b[i+2];out[i+3]=255;
   if(!row||x<x0||x>=x1){outside++;continue;}
   changed++;right=Math.max(right,x);const p=perRow.get(row.row);p.changed_pixels++;p.rightmost_changed_x=Math.max(p.rightmost_changed_x,x);
  }
 }
 return {changed_pixels:changed,outside_changed_pixels:outside,rightmost_changed_x:right,rows:[...perRow.values()],rgba:out};
}
function exactEqual(a,b){if(a.width!==b.width||a.height!==b.height)return false;for(let i=0;i<a.data.length;i++)if(a.data[i]!==b.data[i])return false;return true;}
function release(im){if(!im)return;im.data=null;if(im.canvas){im.canvas.width=0;im.canvas.height=0;}}
async function decode(base64){
 const header=S().png(base64);
 // Narrow row groups only. This secondary budget is independent of the large tile budget.
 if(header.width*header.height>4000000)throw fail('SPARSE_DECODE_BUDGET','Sparse evidence image exceeds four million pixels.',header);
 const image=new root.Image();await new Promise((ok,no)=>{image.onload=ok;image.onerror=()=>no(fail('SPARSE_DECODE_FAILED','Cannot decode sparse evidence PNG.'));image.src='data:image/png;base64,'+base64;});
 const canvas=root.document.createElement('canvas');canvas.width=header.width;canvas.height=header.height;
 const ctx=canvas.getContext('2d',{willReadFrequently:true});if(!ctx)throw fail('SPARSE_CANVAS_FAILED','Canvas 2D is unavailable.');
 ctx.drawImage(image,0,0);const data=ctx.getImageData(0,0,header.width,header.height).data;image.src='';
 return {width:header.width,height:header.height,data,canvas};
}
function encodeDiff(diff,width,height){
 const canvas=root.document.createElement('canvas');canvas.width=width;canvas.height=height;
 try{const ctx=canvas.getContext('2d');if(!ctx)throw fail('SPARSE_CANVAS_FAILED','Canvas 2D is unavailable.');
  const data=ctx.createImageData(width,height);data.data.set(diff);ctx.putImageData(data,0,0);
  const url=canvas.toDataURL('image/png');if(!url.startsWith('data:image/png;base64,'))throw fail('SPARSE_ENCODE_FAILED','XOR PNG encoding did not produce PNG data.');
  const value=url.slice(url.indexOf(',')+1);S().png(value);return value;
 }finally{canvas.width=0;canvas.height=0;}
}
async function pixelRowsToSheetRows(session,sheet,tile,pixelBands,maxRows=128){
 if(pixelBands.length>128)throw fail('SPARSE_CANDIDATE_BUDGET','Too many independent candidate bands.',{bands:pixelBands.length});
 const sy=tile.raster_profile.scale_y,r=tile.rect,cache=new Map([[r.r0,tile.geometry.top],[r.r1,tile.geometry.bottom]]);
 // Batch simultaneous binary searches for all band boundaries; no full-sheet row scan.
 const requests=pixelBands.flatMap(([a,b])=>[{target:tile.geometry.top+(a-1)/sy,lo:r.r0,hi:r.r1},{target:tile.geometry.top+(b+1)/sy,lo:r.r0,hi:r.r1}]);
 for(let iter=0;requests.some(q=>q.hi-q.lo>1);iter++){
  if(iter>22)throw fail('SPARSE_ROW_SEARCH','Could not map candidate pixels to rows.');
  const missing=[...new Set(requests.filter(q=>q.hi-q.lo>1).map(q=>Math.floor((q.lo+q.hi)/2)))].filter(i=>!cache.has(i));
  if(missing.length){const gs=await session.geometries(sheet,missing.map(i=>({r0:i,r1:i+1,c0:r.c0,c1:r.c0+1})));missing.forEach((n,i)=>cache.set(n,gs[i].top));}
  for(const q of requests){if(q.hi-q.lo<=1)continue;const mid=Math.floor((q.lo+q.hi)/2);if(cache.get(mid)<=q.target)q.lo=mid;else q.hi=mid;}
 }
 const found=new Set();for(let i=0;i<requests.length;i+=2)for(let row=requests[i].lo;row<=requests[i+1].lo&&row<r.r1;row++){
  found.add(row);if(found.size>maxRows)throw fail('SPARSE_CANDIDATE_BUDGET','Candidate rows exceed the sparse mutation budget.',{rows:found.size,max_rows:maxRows});
 }
 return [...found].sort((a,b)=>a-b);
}
function groupedRows(rows,maxSpan=16){const out=[];for(const row of rows){const p=out.at(-1);if(p&&row===p.at(-1)+1&&p.length<maxSpan)p.push(row);else out.push([row]);}return out;}
function guardsOk(c){return c.value_type==='Empty'&&(c.value===''||c.value===null)&&
 (c.formula===''||c.formula===null)&&c.has_spill===false&&c.merged===false&&c.table_count===0&&c.pivot_count===0;}
async function readGuards(session,sheet,rows,column){
 return session.office('sparse_guard',{sheet,rows:rows.map(x=>x+1),blocker_column:column+1},async context=>{
  const ws=context.workbook.worksheets.getItem(sheet);ws.protection.load('protected');
  const refs=rows.map(row=>{
   const cell=ws.getRangeByIndexes(row,column,1,1);cell.load('values,formulas,valueTypes,hasSpill,top,height,left,width');
   const merged=cell.getMergedAreasOrNullObject();merged.load('address');
   // Also reject a table immediately left: entering a value beside a table may expand it.
   const neighborhood=ws.getRangeByIndexes(row,Math.max(0,column-1),1,column>0?2:1);
   const tables=neighborhood.getTables(false);tables.load('items/id');
   const pivots=neighborhood.getPivotTables(false);pivots.load('items/name');
   return {row,cell,merged,tables,pivots};
  });await context.sync();
  if(ws.protection.protected)throw fail('SPARSE_PROTECTED_SHEET','Sparse intervention is disabled on protected worksheets.');
  return refs.map(({row,cell,merged,tables,pivots})=>({row,value:cell.values?.[0]?.[0],formula:cell.formulas?.[0]?.[0],
    value_type:cell.valueTypes?.[0]?.[0],has_spill:cell.hasSpill,merged:!merged.isNullObject,table_count:tables.items.length,pivot_count:pivots.items.length,
    top:cell.top,height:cell.height,left:cell.left,width:cell.width}));
 });
}
async function save(session,record,role,base64,rect){
 const id=`${record.id}-${role}`,header=S().png(base64),a=S().address(rect),t=performance.now();
 const receipt=await session.http('/image-batch',{workbook:session.workbook,batch_index:0,images:[{id,sheet:record.sheet,address:a,base64}]});
 const saved=receipt.captures?.[0];if(!saved)throw fail('PERSIST_NO_RECEIPT','Sparse evidence PNG has no receipt.',{id},true);
 session.timings.persist_ms+=performance.now()-t;
 const artifact={...saved,id,role,rect,png_width:header.width,png_height:header.height};record.artifacts.push(artifact);return artifact;
}
async function take(session,record,role,rect,expected){
 const a=S().address(rect),gs=(await session.geometries(record.sheet,[rect]))[0];
 if(['top','bottom','left','right'].some(k=>Math.abs(gs[k]-expected[k])>0.01))throw fail('SPARSE_GEOMETRY_CHANGED','Geometry changed before sparse evidence capture.',{before:expected,after:gs});
 const limits={...session.limitsFor(record.sheet),max_pixels:4000000};if(!S().fits(gs,limits))throw fail('SPARSE_CAPTURE_BUDGET','Sparse row image exceeds its four-million-pixel budget.',{a,gs});
 let base64=await session.office('sparse_render',{sheet:record.sheet,address:a,role,check_id:record.id},async context=>{
  const result=context.workbook.worksheets.getItem(record.sheet).getRange(a).getImage();await context.sync();return result.value;
 });
 try{const h=S().png(base64);await save(session,record,role,base64,rect);
  const after=(await session.geometries(record.sheet,[rect]))[0],p=S().pixels(after,limits);
  if(['top','bottom','left','right'].some(k=>Math.abs(after[k]-expected[k])>0.01)||h.width!==p.width||h.height!==p.height||!S().actualFits(h,limits))
   throw fail('SPARSE_GEOMETRY_CHANGED','Sparse evidence geometry/raster scale was not stable.',{after,expected,header:h});
  return await decode(base64);
 }finally{base64=null;}
}
async function mutate(session,record,rows,column,value){
 const phase=value===' '?'sparse_write':'sparse_restore';const t=performance.now();
 try{return await session.office(phase,{sheet:record.sheet,check_id:record.id,cell_count:rows.length,addresses:rows.map(r=>S().address({r0:r,r1:r+1,c0:column,c1:column+1}))},async context=>{
  const ws=context.workbook.worksheets.getItem(record.sheet);
  // The cells were verified empty. Values-only writes leave formatting untouched.
  // Contiguous candidate rows are batched in one request; unrelated rows are not touched.
  for(const group of groupedRows(rows,128))ws.getRangeByIndexes(group[0],column,group.length,1).values=group.map(()=>[value]);
  await context.sync();
 });}finally{record.timings[value===' '?'write_ms':'restore_ms']+=performance.now()-t;}
}
async function verifyGroup(session,sheet,tile,rows,column,edges,originalImage=null){
 const record={id:`sparse${String(session.state.sparse_overflow_checks.length+1).padStart(6,'0')}`,sheet,
   source_tile:tile.id,rows:rows.map(x=>x+1),blocker_column:column+1,blocker_count:rows.length,
   status:'started',restoration:'not-required',artifacts:[],timings:{write_ms:0,restore_ms:0},
   note:'No change means no blocker-sensitive pixels in the tested probe rows, not proof about arbitrary worksheet content.'};
 session.state.sparse_overflow_checks.push(record);
 const rect={...tile.rect,r0:rows[0],r1:rows.at(-1)+1};record.address=S().address(rect);
 const started=performance.now();let before,blocked,restored,mutationQueued=false,error=null;
 await session.checkpoint('sparse_check_started',{sheet,check_id:record.id,rows:record.rows});
 try{
  const guards=await readGuards(session,sheet,rows,column);record.guards=guards;
  if(!guards.every(guardsOk))throw fail('SPARSE_UNSAFE_TARGET','A blocker target is not an ordinary empty cell.',{rows:guards.filter(x=>!guardsOk(x)).map(x=>x.row+1)});
  // Do not classify a narrow row crop when its source could be a vertical merge
  // or rotated text outside these rows. A skipped check retains the current bounds.
  await session.office('sparse_source_guard',{sheet,address:record.address},async context=>{
   const source=context.workbook.worksheets.getItem(sheet).getRangeByIndexes(rect.r0,0,rect.r1-rect.r0,column);
   source.format.load('textOrientation');
   const merged=source.getMergedAreasOrNullObject();merged.load('areas/items/rowIndex,areas/items/rowCount');
   await context.sync();
   if(source.format.textOrientation!==0 || (!merged.isNullObject && merged.areas.items.some(x=>x.rowCount!==1)))
    throw fail('SPARSE_SOURCE_CONTEXT','Rotated or vertically merged source context cannot be verified by a row-local blocker.',{sheet,address:record.address});
  });
  const geometry=(await session.geometries(sheet,[rect]))[0];record.geometry=geometry;
  before=await take(session,record,'before',rect,geometry);
  if(originalImage){
   const dy=Math.round((geometry.top-tile.geometry.top)*tile.raster_profile.scale_y);
   const x0=Math.max(1,Math.floor(edges[column])+1), x1=before.width-1;
   let compared=0,mismatch=0;
   for(let y=1;y<before.height-1;y++)for(let x=x0;x<x1;x++){
    const sourceY=dy+y;if(sourceY<1||sourceY>=originalImage.height-1)continue;
    const a=(y*before.width+x)*4,b=(sourceY*originalImage.width+x)*4;compared++;
    if(before.data[a]!==originalImage.data[b]||before.data[a+1]!==originalImage.data[b+1]||before.data[a+2]!==originalImage.data[b+2]||before.data[a+3]!==originalImage.data[b+3])mismatch++;
   }
   record.source_crop_check={compared_pixels:compared,mismatched_pixels:mismatch};
   if(!compared||mismatch)throw fail('SPARSE_CROP_MISMATCH','Row-local before image does not reproduce the original probe pixels.',record.source_crop_check);
  }
  // Flush the intended write and restoration recipe BEFORE queueing native mutation.
  record.restoration='pending';await session.checkpoint('sparse_write_pending',{sheet,check_id:record.id,rows:record.rows,blocker_column:column+1});
  mutationQueued=true;
  try{
   await mutate(session,record,rows,column,' ');
   const written=await session.office('sparse_verify_write',{sheet,check_id:record.id},async context=>{
    const cells=rows.map(row=>{const c=context.workbook.worksheets.getItem(sheet).getRangeByIndexes(row,column,1,1);c.load('values,formulas');return c;});await context.sync();
    return cells.every(c=>c.values[0][0]===' '&&c.formulas[0][0]===' ');
   });
   if(!written)throw fail('SPARSE_WRITE_NOT_CONFIRMED','The whitespace write was not confirmed.');
   blocked=await take(session,record,'blocked',rect,geometry);
  }finally{
   if(session.dead){record.restoration='unknown-native-timeout';throw fail('SPARSE_RESTORE_UNCONFIRMED','Native call timed out; no further calls queued. Discard this working copy.',{check_id:record.id},true);}
   await mutate(session,record,rows,column,'');
   const restoredGuards=await readGuards(session,sheet,rows,column);
   if(!restoredGuards.every(guardsOk))throw fail('SPARSE_RESTORE_FAILED','Blocker contents were not restored.',{check_id:record.id},true);
   record.restoration='contents-restored';mutationQueued=false;
  }
  restored=await take(session,record,'restored',rect,geometry);
  record.roundtrip_identical=exactEqual(before,restored);
  if(!record.roundtrip_identical)throw fail('SPARSE_UNSTABLE_ROUNDTRIP','Before and restored images differ. Do not infer absence of overflow.',{check_id:record.id});
  const sy=tile.raster_profile.scale_y;
  const allowed=guards.map(g=>({row:g.row+1,y0:Math.max(1,Math.floor((g.top-geometry.top)*sy)),y1:Math.min(before.height-1,Math.ceil((g.top+g.height-geometry.top)*sy)+1)}));
  const x0=Math.max(1,Math.floor(edges[column])),x1=before.width-1;
  const diff=pixelDiff(before,blocked,x0,x1,allowed);
  record.difference={changed_pixels:diff.changed_pixels,outside_changed_pixels:diff.outside_changed_pixels,
    rightmost_changed_x:diff.rightmost_changed_x,rows:diff.rows};
  await save(session,record,'xor',encodeDiff(diff.rgba,before.width,before.height),rect);
  if(diff.outside_changed_pixels)throw fail('SPARSE_NONLOCAL_CHANGE','The write affected pixels outside the verified probe area.',{outside_changed_pixels:diff.outside_changed_pixels});
  record.restoration='verified';record.status='complete';
  record.verdict=diff.changed_pixels?'blocker-sensitive-signal':'false-positive-for-text-overflow';
  return record;
 }catch(e){error=e;record.status='inconclusive';record.error=S().errorInfo(e);
  if(mutationQueued&&record.restoration!=='contents-restored'&&record.restoration!=='verified'){
    e.fatal=true;session.dead=true;record.status='failed';record.error=S().errorInfo(e);
  }
  if(e.fatal)throw e;
  return record;
 }finally{
  release(before);release(blocked);release(restored);record.timings.total_ms=performance.now()-started;
  await session.event('sparse_check_finished',{sheet,...record});await session.checkpoint('sparse_check_finished',{sheet,check_id:record.id,status:record.status});
 }
}
async function verify(session,item,tile,image,decision,edges,end){
 const start=performance.now(),scan=scanCandidates(image,edges,item.base_right_column,end,decision.background_rgba);
 if(!scan.pixel_rows)return {keep_column:item.base_right_column,done:true,scanned_pixels:scan.scanned_pixels,mode:'selective',checks:[]};
 try{
  if(root.Office?.context?.requirements&&!root.Office.context.requirements.isSetSupported('ExcelApi','1.13'))
    throw fail('SPARSE_API_UNAVAILABLE','Selective verification needs ExcelApi 1.13 for safe merged/spill guards.');
  const rows=await pixelRowsToSheetRows(session,item.sheet,tile,scan.pixel_bands);
  let column=item.base_right_column+1;while(column<=end&&edges[column+1]===edges[column])column++;
  if(column>end)throw fail('SPARSE_NO_VISIBLE_BLOCKER','No visible empty blocker column exists in the probe.');
  const groups=groupedRows(rows),checks=[];let keep=item.base_right_column;
  for(const group of groups){const check=await verifyGroup(session,item.sheet,tile,group,column,edges,image);checks.push(check.id);
    if(check.status!=='complete')return {unresolved:true,done:false,keep_column:end,checks,reason:check.error?.code,mode:'selective'};
    const x=check.difference.rightmost_changed_x;
    if(x>=0){let c=column;while(c<end&&edges[c+1]<=x)c++;keep=Math.max(keep,c);}
  }
  let last=end;while(last>item.base_right_column&&edges[last+1]===edges[last])last--;
  return {mode:'selective',done:keep<last,keep_column:keep,checks,scanned_pixels:scan.scanned_pixels,
    candidate_pixel_rows:scan.pixel_rows,candidate_sheet_rows:rows.map(x=>x+1),timing_ms:performance.now()-start,
    note:'Unchanged line/fill pixels are excluded only within verified probe rows; source images are retained.'};
 }catch(e){if(e.fatal)throw e;
  await session.event('sparse_check_unresolved',{sheet:item.sheet,address:tile.address,error:S().errorInfo(e)});
  return {mode:'selective',unresolved:true,done:false,keep_column:end,reason:e.code,error:S().errorInfo(e)};
 }
}
const api={scanCandidates,bands,pixelDiff,exactEqual,pixelRowsToSheetRows,groupedRows,guardsOk,verify,verifyGroup};
root.SelectiveOverflow=api;if(typeof module!=='undefined'&&module.exports)module.exports=api;
}(typeof window!=='undefined'?window:globalThis));
