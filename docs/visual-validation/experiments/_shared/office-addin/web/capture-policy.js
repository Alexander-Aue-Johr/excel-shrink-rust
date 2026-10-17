/* Capture presentation/export policy. No content-bound discovery or image resampling. */
(function(root){
'use strict';
const S=()=>root.SafeCapture;
const VERSION='gridless-context-final-png-v2';
function outputPolicy({complete,checked,base,end,probe,detected=false}){
 if(!Number.isInteger(base)||!Number.isInteger(end)||base<0||end<base)throw S().fault('INVALID_EXPORT_POLICY','Invalid base/probe bounds.',{base,end},true);
 if(!complete)return {crop:false,end,reason:'unresolved-retain-buffer'};
 if(!probe)return {crop:false,end,reason:'no-overflow-probe'};
 if(detected)return {crop:false,end,reason:'overflow-detected-retain-buffer'};
 if(!checked)return {crop:false,end,reason:'unchecked-retain-buffer'};
 return {crop:true,end:base,reason:'no-overflow-remove-buffer'};
}
async function hideGridlines(session,catalog){
 const previous=catalog.map(s=>({sheet:s.name,sheet_index:s.sheet_index,show_gridlines:s.show_gridlines}));
 const record={schema:VERSION,setting:'Worksheet.showGridlines',requested:false,status:'pending',before:previous,after:[]};
 session.state.capture_view=record;
 await session.checkpoint('capture_view_pending',{setting:record.setting,requested:false});
 try{
  record.after=await session.office('capture_view',{setting:record.setting,sheets:previous.map(x=>x.sheet)},async context=>{
   const refs=previous.map(x=>context.workbook.worksheets.getItem(x.sheet));
   for(const ws of refs)ws.showGridlines=false;
   await context.sync();
   for(const ws of refs)ws.load('name,showGridlines');
   await context.sync();
   return refs.map(ws=>({sheet:ws.name,show_gridlines:ws.showGridlines}));
  });
  if(record.after.some(x=>x.show_gridlines!==false))throw S().fault('GRIDLINES_NOT_DISABLED','The gridline setting was not confirmed; no screenshots will be attempted.',{before:previous,after:record.after},true);
  record.status='confirmed';
  for(const s of catalog){s.original_show_gridlines=s.show_gridlines;s.show_gridlines=false;}
  await session.event('capture_view_confirmed',record);
  await session.checkpoint('capture_view_confirmed',{show_gridlines:false});
 }catch(e){record.status='failed';record.error=S().errorInfo(e);e.fatal=true;throw e;}
 // The runner closes/discards its disposable workbook. Do not re-enable the grid
 // before screenshots finish, change styled cell borders, or alter diagnostic fixtures.
}
async function probeBuffer(session,item,requestedPixels){
 if(!Number.isInteger(requestedPixels)||requestedPixels<64||requestedPixels>8192)
  throw S().fault('INVALID_PROBE_BUFFER','Probe buffer must be 64..8192 pixels.',{requestedPixels},true);
 await session.ensureCalibration(item.sheet,S().parse(item.address));
 const limits=session.limitsFor(item.sheet),scale=limits.scale_x,base=item.base_right_column;
 if(limits.fixed_x)throw S().fault('FIXED_WIDTH_HORIZONTAL_OPERATION',
  'A right-hand overflow probe needs independently calibrated horizontal geometry. The fixed-span mode does not extrapolate.',
  {sheet:item.sheet,address:item.address,fixed_x:limits.fixed_x},true);
 // Use Rust visibility hints to skip hidden columns, then verify actual boundary positions.
 const candidates=[];
 for(let col=base+1;col<16384 && candidates.length<64;col++)if(Number(item.column_widths[col])>0)candidates.push(col);
 if(!candidates.length)return {end_column:base,extra_visible_columns:0,requested_pixels:requestedPixels,actual_pixels:0,limited:true};
 const rects=[{r0:0,r1:1,c0:base,c1:base+1},...candidates.map(c=>({r0:0,r1:1,c0:c,c1:c+1}))];
 const gs=await session.geometries(item.sheet,rects),baseRight=gs[0].right;
 let count=candidates.findIndex((_,i)=>(gs[i+1].right-baseRight)*scale>=requestedPixels)+1;
 if(count===0)count=candidates.length;
 return {end_column:candidates[count-1],extra_visible_columns:count,requested_pixels:requestedPixels,
  actual_pixels:Math.round((gs[count].right-baseRight)*scale),scale_x:scale,
  limited:(gs[count].right-baseRight)*scale<requestedPixels,maximum_extra_visible_columns:64,
  method:'measured-column-edges-and-calibrated-scale'};
}
async function exportTile(session,tile,width,logicalAddress){
 const core=tile.core_box_px||{x:0,y:0,width:tile.png_width,height:tile.png_height};
 if(!Number.isInteger(width)||width<1||width>core.width)throw S().fault('INVALID_EXPORT_WIDTH','Invalid final PNG width.',{tile:tile.id,width,core},true);
 if(tile.raster_profile?.fixed_x && (width!==tile.raster_profile.fixed_x.width_px || core.x!==0))
  throw S().fault('FIXED_WIDTH_HORIZONTAL_OPERATION','A final export must preserve the observed fixed width.',{tile:tile.id,width,core},true);
 const crop={x:core.x,y:core.y,width,height:core.height};
 if(crop.x===0&&crop.y===0&&width===tile.png_width&&crop.height===tile.png_height)
  return {...tile,export_kind:'retained-original',capture_address:tile.address,address:logicalAddress};
 const t=performance.now();
 const record={source_id:tile.id,sheet:tile.sheet,capture_address:tile.address,logical_address:logicalAddress,
  source_width:tile.png_width,source_height:tile.png_height,output_width:width,output_height:crop.height,crop_rect_px:crop,status:'pending'};
 (session.state.final_exports ||= []).push(record);
 await session.checkpoint('final_png_pending',record);
 try{
  const receipt=await session.http('/finalize-image',{workbook:session.workbook,source_id:tile.id,source_sha256:tile.sha256,
   sheet:tile.sheet,address:logicalAddress,...crop,source_width:tile.png_width,source_height:tile.png_height});
  if(!receipt.capture || receipt.capture.png_width!==width || receipt.capture.png_height!==crop.height)
   throw S().fault('FINAL_PNG_RECEIPT_MISMATCH','Native crop returned unexpected dimensions.',{source_id:tile.id,crop,received:receipt.capture||null},true);
  record.status='complete';record.receipt=receipt.capture;record.host_crop_ms=receipt.crop_ms;
  return {...tile,...receipt.capture,source_id:tile.id,raw_png:tile.relative_png,raw_sha256:tile.sha256,
   raw_png_width:tile.png_width,raw_png_height:tile.png_height,capture_address:tile.address,
   export_kind:(crop.x||crop.y||crop.height!==tile.png_height)?'native-context-crop':'native-right-crop',crop_rect_px:crop};
 }catch(e){record.status='failed';record.error=S().errorInfo(e);throw e;}
 finally{record.total_ms=performance.now()-t;session.timings.finalize_ms=(session.timings.finalize_ms||0)+record.total_ms;
  await session.checkpoint('final_png_finished',{sheet:tile.sheet,source_id:tile.id,status:record.status});}
}

const api={VERSION,outputPolicy,hideGridlines,probeBuffer,exportTile};root.CapturePolicy=api;
if(typeof module!=='undefined'&&module.exports)module.exports=api;
}(typeof window!=='undefined'?window:globalThis));
