/* Visible-axis geometry and contextual capture rectangles.
 * Measures Range.height/width on individual cells; never changes the workbook.
 * The extra cells are render context, not a larger logical content boundary.
 */
(function(root){
'use strict';
const ROWS=1048576, COLS=16384, BATCH=384, MAX_ROWS=200000;
const fail=(code,message,detail)=>root.SafeCapture.fault(code,message,detail,true);
const addr=r=>root.SafeCapture.address(r);
const sameRect=(a,b)=>['r0','r1','c0','c1'].every(k=>a[k]===b[k]);
function prefix(entries){const p=[0];for(const e of entries){if(!e)throw fail('AXIS_DATA_MISSING','Axis prefix has an unmeasured entry.');p.push(p.at(-1)+e.size);}return p;}
function pointBox(data,r){return {left:data.x[r.c0],right:data.x[r.c1],top:data.y[r.r0],bottom:data.y[r.r1],geometry_model:'individual-visible-cell-sums-v1'};}
function neighbour(entries,boundary,direction,lo,hi,scale){
 let size=0,visible=0,skipped=0,at=boundary;
 while(direction<0?at>lo:at<hi){
  const i=direction<0?--at:at++;
  if(entries[i].size>0){size+=entries[i].size;visible++;}else skipped++;
  // At least one physically visible cell and two device pixels between the
  // retained edge and the native image edge, even with extremely narrow cells.
  if(visible && size*scale>=2-1e-8)return {boundary:at,visible_cells:visible,zero_extent_cells:skipped,points:size};
 }
 if(size===0)return {boundary:direction<0?lo:hi,visible_cells:0,zero_extent_cells:skipped,points:0,physical_outer_edge:true};
 throw fail('CONTEXT_TOO_THIN','No sufficiently visible neighbour inside the requested extent.',{boundary,direction,lo,hi,points:size});
}
function guardBox(data,core,parent,limits){
 const raw={...core},guard={};
 if(limits.fixed_x && (core.c0!==parent.c0 || core.c1!==parent.c1 || parent.c0!==limits.fixed_x.c0 || parent.c1!==limits.fixed_x.c1))
  throw fail('FIXED_WIDTH_HORIZONTAL_OPERATION','Fixed-span capture keeps every tile at the identical column interval.',{core,parent});
 for(const [key,axis,side,dir] of [['r0','rows','top',-1],['r1','rows','bottom',1],['c0','columns','left',-1],['c1','columns','right',1]]){
  const low=axis==='rows'?'r0':'c0',high=axis==='rows'?'r1':'c1';
  if(core[key]===parent[key]){guard[side]={visible_cells:0,points:0,logical_outer_edge:true};continue;}
  const g=neighbour(data[axis],core[key],dir,parent[low],parent[high],axis==='rows'?limits.scale_y:limits.scale_x);
  raw[key]=g.boundary;guard[side]=g;
 }
 return {core_rect:{...core},capture_rect:raw,parent_rect:{...parent},guards:guard};
}
function cropBox(core,raw,parent,limits,header){
 const S=root.SafeCapture,sx=limits.scale_x,sy=limits.scale_y;
 if(limits.fixed_x)for(const g of [core,raw,parent])S.assertFixedSpan(g,limits);
 const x=S.xOffset(raw.left,core.left,limits),y=Math.round((core.top-raw.top)*sy);
 const width=limits.fixed_x?limits.fixed_x.width_px:Math.round((core.right-core.left)*sx)+1,height=Math.round((core.bottom-core.top)*sy)+1;
 const offset={x:S.xOffset(parent.left,core.left,limits),y:Math.round((core.top-parent.top)*sy)};
 // A crop must use actual interior pixels, never silently clamp a phase error.
 if(x<0||y<0||width<1||height<1||x+width>header.width||y+height>header.height)
  throw fail('CONTEXT_CROP_BOUNDS','Measured core does not fit its original capture.',{core,raw,parent,header,x,y,width,height});
 const rawOffset={x:S.xOffset(parent.left,raw.left,limits),y:Math.round((raw.top-parent.top)*sy)};
 if(offset.x!==rawOffset.x+x||offset.y!==rawOffset.y+y)
  throw fail('CONTEXT_RASTER_PHASE','Local crop and global raster coordinates disagree; no pixels were shifted.',{offset,rawOffset,x,y});
 return {x,y,width,height};
}
class Ledger{
 constructor(session){this.session=session;this.sheets=new Map();}
 data(sheet){
  if(!this.sheets.has(sheet)){
   const audit={sheet,model:'individual-visible-cell-sums-v1',row_count:0,column_count:0,read_batches:0,refreshed_cells:0,zero_rows:0,zero_columns:0,max_native_top_delta_points:0,max_native_left_delta_points:0,samples:[]};
   (this.session.state.axis_geometry ||= []).push(audit);
   this.sheets.set(sheet,{rows:[],columns:[],x:[0],y:[0],anchor:0,audit,mergeCache:new Map()});
  }
  return this.sheets.get(sheet);
 }
 async read(sheet,requests,refresh){
  const data=this.data(sheet);
  for(let start=0;start<requests.length;start+=BATCH){
   const batch=requests.slice(start,start+BATCH);
   const result=await this.session.office('axis_geometry',{sheet,cell_count:batch.length,refresh:!!refresh,
    rows:batch.filter(x=>x.axis==='rows').map(x=>x.index+1),columns:batch.filter(x=>x.axis==='columns').map(x=>x.index+1)},async context=>{
    const ws=context.workbook.worksheets.getItem(sheet);
    const refs=batch.map(q=>{
     const r=q.axis==='rows'?q.index:0,c=q.axis==='rows'?data.anchor:q.index;
     const cell=ws.getRangeByIndexes(r,c,1,1);
     cell.load(q.axis==='rows'?'top,height,rowHidden':'left,width,columnHidden');return {q,cell};
    });
    await context.sync();
    return refs.map(({q,cell})=>({...q,size:Number(q.axis==='rows'?cell.height:cell.width),
     hidden:q.axis==='rows'?cell.rowHidden:cell.columnHidden,native:Number(q.axis==='rows'?cell.top:cell.left)}));
   });
   data.audit.read_batches++;
   const changes=[];
   for(const item of result){
    if(!Number.isFinite(item.size)||item.size<0||!Number.isFinite(item.native)||typeof item.hidden!=='boolean')
     throw fail('AXIS_GEOMETRY_INVALID','Individual cell geometry/visibility was not finite and definite.',{sheet,...item});
    if(item.hidden)item.size=0;
    const previous=data[item.axis][item.index];
    if(refresh&&previous&&(Math.abs(previous.size-item.size)>0.01||previous.hidden!==item.hidden))
     changes.push({axis:item.axis,index_1based:item.index+1,before:previous,after:item});
   }
   if(changes.length)throw fail('AXIS_GEOMETRY_CHANGED','Previously measured rows or columns changed during capture; discard mixed geometry.',{sheet,changes:changes.slice(0,24),changed_count:changes.length});
   for(const item of result)data[item.axis][item.index]={size:item.size,hidden:item.hidden,native:item.native};
   if(refresh)data.audit.refreshed_cells+=result.length;
  }
 }
 async ensure(sheet,rects,refresh=false){
  const d=this.data(sheet),re=Math.max(0,...rects.map(r=>r.r1)),ce=Math.max(0,...rects.map(r=>r.c1));
  if(re>MAX_ROWS||ce>COLS)throw fail('AXIS_GEOMETRY_BUDGET','Requested geometric prefix exceeds the bounded ledger; no full-sheet fallback scan.',{sheet,rows:re,columns:ce,max_rows:MAX_ROWS,max_columns:COLS});
  if(!refresh && d.rows.length>=re && d.columns.length>=ce)return d;
  let requests=[];
  for(let c=d.columns.length;c<ce;c++)requests.push({axis:'columns',index:c});
  if(requests.length)await this.read(sheet,requests,false);
  const anchor=d.columns.findIndex(x=>x.size>0);d.anchor=anchor<0?0:anchor;
  requests=[];for(let r=d.rows.length;r<re;r++)requests.push({axis:'rows',index:r});
  if(requests.length)await this.read(sheet,requests,false);
  if(refresh){
   const rs=new Set(),cs=new Set();
   for(const r of rects){for(let i=r.r0;i<r.r1;i++)rs.add(i);for(let i=r.c0;i<r.c1;i++)cs.add(i);}
   await this.read(sheet,[...rs].map(index=>({axis:'rows',index})).concat([...cs].map(index=>({axis:'columns',index}))),true);
  }
  if(d.x.length!==d.columns.length+1)d.x=prefix(d.columns);
  if(d.y.length!==d.rows.length+1)d.y=prefix(d.rows);
  const a=d.audit;a.row_count=d.rows.length;a.column_count=d.columns.length;
  a.zero_rows=d.rows.filter(x=>!x.size).length;a.zero_columns=d.columns.filter(x=>!x.size).length;
  // Native positions are retained as diagnostics; they do not supply the raster coordinates.
  a.max_native_top_delta_points=0;a.max_native_left_delta_points=0;a.samples=[];
  for(const [axis,p] of [['rows',d.y],['columns',d.x]])for(let i=0;i<d[axis].length;i++){
   const e=d[axis][i],delta=e.native-p[i],key=axis==='rows'?'max_native_top_delta_points':'max_native_left_delta_points';
   a[key]=Math.max(a[key],Math.abs(delta));
   if(Math.abs(delta)>0.01&&a.samples.length<16)a.samples.push({axis,index_1based:i+1,size_points:e.size,hidden:e.hidden,native_position:e.native,additive_position:p[i],delta});
  }
  return d;
 }
 async geometries(sheet,rects,refresh=false){const d=await this.ensure(sheet,rects,refresh);return rects.map(r=>pointBox(d,r));}
 async mergedContext(sheet,core,parent,descriptor){
  const key=[core.r0,core.r1,core.c0,core.c1,parent.r0,parent.r1,parent.c0,parent.c1].join(':');
  const d=this.data(sheet);if(d.mergeCache.has(key))return d.mergeCache.get(key);
  const bands=[];
  if(core.r0>parent.r0)bands.push({...core,r1:core.r0+1});
  if(core.r1<parent.r1)bands.push({...core,r0:core.r1-1});
  if(core.c0>parent.c0)bands.push({...core,c1:core.c0+1});
  if(core.c1<parent.c1)bands.push({...core,c0:core.c1-1});
  if(!bands.length){d.mergeCache.set(key,[]);return [];}
  const merges=await this.session.office('context_merge_guard',{sheet,addresses:bands.map(addr)},async context=>{
   const ws=context.workbook.worksheets.getItem(sheet);
   const refs=bands.map(r=>{const range=ws.getRange(addr(r));if(typeof range.getMergedAreasOrNullObject!=='function')throw fail('CONTEXT_MERGE_API_UNAVAILABLE','Contextual capture requires ExcelApi 1.13 merged-area queries.',{sheet});const m=range.getMergedAreasOrNullObject();m.load('areas/items/rowIndex,areas/items/columnIndex,areas/items/rowCount,areas/items/columnCount');return m;});
   await context.sync();return refs.flatMap(m=>m.isNullObject?[]:m.areas.items.map(a=>({r0:a.rowIndex,r1:a.rowIndex+a.rowCount,c0:a.columnIndex,c1:a.columnIndex+a.columnCount})));
  });
  const unique=[...new Map(merges.map(m=>[addr(m),m])).values()];d.mergeCache.set(key,unique);return unique;
 }
 async descriptor(sheet,core,parent,limits,checkMerges=false){
  const d=await this.ensure(sheet,[parent]),desc=guardBox(d,core,parent,limits);
  let merges=[];
  if(checkMerges){
   merges=await this.mergedContext(sheet,core,parent,desc);
   for(const m of merges){
    // A merge intersecting the retained core may need more than one neighbour.
    for(const k of ['r0','c0'])desc.capture_rect[k]=Math.max(parent[k],Math.min(desc.capture_rect[k],m[k]));
    for(const k of ['r1','c1'])desc.capture_rect[k]=Math.min(parent[k],Math.max(desc.capture_rect[k],m[k]));
   }
  }
  const geometry=pointBox(d,core),captureGeometry=pointBox(d,desc.capture_rect);
  return {...desc,rect:{...core},geometry,capture_geometry:captureGeometry,merge_context:merges,
   predicted:root.SafeCapture.pixels(captureGeometry,limits),invisible:geometry.right===geometry.left||geometry.bottom===geometry.top};
 }
}
async function plan(session,sheet,rect,parent=rect){
 const ledger=session.axisGeometry;await ledger.ensure(sheet,[parent]);
 const initial=(await ledger.geometries(sheet,[rect]))[0];
 if(initial.right===initial.left||initial.bottom===initial.top){
  await session.event('invisible_tile',{sheet,address:addr(rect),rect,geometry:initial});
  return [{rect,capture_rect:rect,parent_rect:parent,geometry:initial,capture_geometry:initial,invisible:true}];
 }
 await session.ensureCalibration(sheet,rect);
 const S=root.SafeCapture,limits=session.limitsFor(sheet);
 const output=[];
 async function add(core,depth=0){
  if(depth>limits.max_depth||output.length>=limits.max_tiles)throw fail('CONTEXT_PLAN_BUDGET','Contextual tile plan exceeded its bounds.',{sheet,core,depth});
  let desc=await ledger.descriptor(sheet,core,parent,limits,false);
  if(desc.invisible){output.push(desc);return;}
  if(S.fits(desc.capture_geometry,limits)){
   desc=await ledger.descriptor(sheet,core,parent,limits,true);
   if(S.fits(desc.capture_geometry,limits)){output.push(desc);return;}
  }
  // Prefer full-width strips. Horizontal splitting remains explicitly unqualified
  // for incoming text overflow; its geometric guards alone are not that proof.
  const children=S.split(core,desc.capture_geometry,limits);
  for(const child of children)await add(child,depth+1);
 }
 if(session.packing==='balanced'){await add(rect);}
 else{
  const whole=await ledger.descriptor(sheet,rect,parent,limits,false);
  if(whole.invisible){output.push(whole);}
  else if(S.fits(whole.capture_geometry,limits)){await add(rect);}
  else if(whole.predicted.width>limits.max_edge_px||rect.r1-rect.r0<2){await add(rect);}
  else{
   let start=rect.r0;
   while(start<rect.r1){
    if(output.length>=limits.max_tiles)throw fail('CONTEXT_PLAN_BUDGET','Too many context tiles.',{sheet,rect});
    // Binary search runs against cached, measured prefix sums: no Excel call per midpoint.
    let low=start,high=rect.r1+1;
    while(high-low>1){const mid=Math.floor((low+high)/2);const d=await ledger.descriptor(sheet,{...rect,r0:start,r1:mid},parent,limits,false);
     if(d.invisible||S.fits(d.capture_geometry,limits))low=mid;else high=mid;}
    if(low===start)low=start+1;
    await add({...rect,r0:start,r1:low});start=low;
   }
  }
 }
 output.sort((a,b)=>a.rect.r0-b.rect.r0||a.rect.c0-b.rect.c0);
 const full=(await ledger.geometries(sheet,[rect]))[0],fullPixels=S.pixels(full,limits);
 const record={sheet,address:addr(rect),method:'Visible-neighbour context / '+session.packing,packing:session.packing,
  geometry_model:'individual-visible-cell-sums-v1',context_policy:'visible-cell-halo-v1',tile_count:output.length,
  full_predicted:fullPixels,limits,allowed_height_px:Math.min(limits.max_edge_px,Math.floor(limits.max_pixels/fullPixels.width)),max_pixels:limits.max_pixels,max_edge_px:limits.max_edge_px,
  allowed_height_at_full_width_px:Math.min(limits.max_edge_px,Math.floor(limits.max_pixels/fullPixels.width)),
  horizontal_split:output.some(x=>x.rect.c0!==rect.c0||x.rect.c1!==rect.c1),
  tiles:output.map(x=>({rect:x.rect,capture_rect:x.capture_rect,guards:x.guards,predicted:x.predicted,merge_context:x.merge_context}))};
 session.state.tile_plans.push(record);await session.event('tile_plan',record);return output;
}
const api={Ledger,prefix,pointBox,neighbour,guardBox,cropBox,plan,sameRect,MAX_ROWS,BATCH};root.CaptureGeometry=api;
if(typeof module!=='undefined'&&module.exports)module.exports=api;
}(typeof window!=='undefined'?window:globalThis));
