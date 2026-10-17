/* A bounded real-Excel oracle for the contextual crop rule. Qualification only.
 * Four small PNGs: whole range, two guarded pieces, whole range again.
 * Compare exact decoded RGBA, never PNG encoding, scale or shifted alignment.
 */
(function(root){
'use strict';
const S=()=>root.SafeCapture, G=()=>root.CaptureGeometry;
function difference(a,b){
 if(a.width!==b.width||a.height!==b.height)return {equal:false,reason:'dimensions',a:{width:a.width,height:a.height},b:{width:b.width,height:b.height}};
 let count=0;const examples=[];
 for(let i=0;i<a.data.length;i+=4){if(a.data[i]===b.data[i]&&a.data[i+1]===b.data[i+1]&&a.data[i+2]===b.data[i+2]&&a.data[i+3]===b.data[i+3])continue;
  count++;if(examples.length<12)examples.push({x:(i/4)%a.width,y:Math.floor(i/4/a.width),a:Array.from(a.data.slice(i,i+4)),b:Array.from(b.data.slice(i,i+4))});}
 return {equal:count===0,different_pixels:count,compared_pixels:a.width*a.height,examples};
}
function compose(first,last,offset,height){
 const width=first.width;
 if(width!==last.width||offset<0||offset>first.height||offset+last.height!==height)
  throw S().fault('CONTEXT_REFERENCE_MISMATCH','Local strips have a gap, size mismatch or missing coverage.',{},true);
 const data=new Uint8ClampedArray(width*height*4);data.set(first.data);
 let conflicts=0;const examples=[];
 for(let y=0;y<last.height;y++)for(let x=0;x<width;x++){
  const a=((offset+y)*width+x)*4,b=(y*width+x)*4;
  if(offset+y<first.height){
   if([0,1,2,3].some(c=>data[a+c]!==last.data[b+c])){conflicts++;if(examples.length<12)examples.push({x,y:offset+y});}
  }else for(let c=0;c<4;c++)data[a+c]=last.data[b+c];
 }
 return {width,height,data,overlap_mismatches:conflicts,overlap_examples:examples};
}
async function decode(base64,box,limit){
 const h=S().png(base64);if(!S().actualFits(h,limit))throw S().fault('CONTEXT_REFERENCE_BUDGET','Reference PNG exceeded its independent four-MP budget.',h,true);
 const image=new root.Image(),canvas=root.document.createElement('canvas');
 try{
  await new Promise((ok,no)=>{image.onload=ok;image.onerror=()=>no(new Error('Reference image decode failed'));image.src='data:image/png;base64,'+base64;});
  if(image.naturalWidth!==h.width||image.naturalHeight!==h.height)throw new Error('Reference PNG decoder dimensions disagree');
  canvas.width=h.width;canvas.height=h.height;const ctx=canvas.getContext('2d',{willReadFrequently:true});
  if(!ctx)throw new Error('Reference Canvas unavailable');ctx.drawImage(image,0,0);
  return {width:box.width,height:box.height,data:ctx.getImageData(box.x,box.y,box.width,box.height).data};
 }finally{image.src='';canvas.width=0;canvas.height=0;}
}
async function verify(session,sheet,tiles,logicalId){
 const records=(session.state.context_validations ||= []);
 if(records.some(r=>r.sheet===sheet))return;
 const pair=tiles.slice(1).map((b,i)=>[tiles[i],b]).find(([a,b])=>!a.invisible&&!b.invisible&&a.rect.r1===b.rect.r0&&a.rect.c0===b.rect.c0&&a.rect.c1===b.rect.c1);
 if(!pair)return; // Monolithic sheets have no internal seam; not a fake passing test.
 const [first,second]=pair,rootRect=first.parent_rect,cut=first.rect.r1;
 const record={id:'context'+String(records.length+1).padStart(5,'0'),sheet,logical_id:logicalId,status:'started',
  method:'whole-versus-contextual-tiles-and-repeat-v1',cut_row_1based:cut+1,artifacts:[]};records.push(record);
 const start=performance.now(),limits={...session.limitsFor(sheet),max_pixels:4000000};
 async function take(role,descriptor,reference){
  const r=descriptor.capture_rect,g=(await session.geometries(sheet,[r],true))[0];
  if(!S().fits(g,limits))throw S().fault('CONTEXT_REFERENCE_BUDGET','Local whole-range check does not fit four MP; no unsafe reference was queued.',{sheet,address:S().address(r)},true);
  const id=record.id+'-'+role;let base64=null;
  try{
   base64=await session.office('context_reference_render',{sheet,address:S().address(r),role,id},async context=>{
    const result=context.workbook.worksheets.getItem(sheet).getRange(S().address(r)).getImage();await context.sync();return result.value;
   });
   const h=S().png(base64),receipt=await session.http('/image-batch',{workbook:session.workbook,batch_index:0,images:[{id,sheet,address:S().address(r),base64}]});
   const saved=receipt.captures?.[0];if(!saved)throw S().fault('PERSIST_NO_RECEIPT','Reference image receipt missing.',{id},true);
   const after=(await session.geometries(sheet,[r],true))[0],expected=S().pixels(after,limits);
   if(h.width!==expected.width||h.height!==expected.height)throw S().fault('CONTEXT_PNG_MISMATCH','Local reference differs from measured axes.',{h,expected},true);
   const core=(await session.geometries(sheet,[descriptor.rect]))[0],parent=(await session.geometries(sheet,[reference]))[0];
   const crop=G().cropBox(core,after,parent,limits,h);
   record.artifacts.push({...saved,role,crop_rect_px:crop,core_address:S().address(descriptor.rect)});
   return await decode(base64,crop,limits);
  }finally{base64=null;}
 }
 try{
  const d=session.axisGeometry.data(sheet);
  // Two genuinely visible rows on each side, not two indices that may be hidden.
  function reach(from,dir){let n=0,i=from;while(dir<0?i>rootRect.r0:i<rootRect.r1){const row=dir<0?--i:i++;if(d.rows[row].size>0&&++n>=2)break;}return i;}
  const reference={...rootRect,r0:reach(cut,-1),r1:reach(cut,1)};
  const whole=await session.axisGeometry.descriptor(sheet,reference,reference,limits,true);
  const upper=await session.axisGeometry.descriptor(sheet,{...reference,r1:cut},reference,limits,true);
  const lower=await session.axisGeometry.descriptor(sheet,{...reference,r0:cut},reference,limits,true);
  record.reference_address=S().address(reference);record.upper_address=S().address(upper.capture_rect);record.lower_address=S().address(lower.capture_rect);
  await session.event('context_reference_started',{sheet,...record});
  const a=await take('whole-before',whole,reference),b=await take('upper',upper,reference),c=await take('lower',lower,reference),z=await take('whole-after',whole,reference);
  record.repeat_comparison=difference(a,z);
  const offset=Math.round((lower.geometry.top-whole.geometry.top)*limits.scale_y);
  const joined=compose(b,c,offset,a.height);record.overlap_mismatches=joined.overlap_mismatches;record.overlap_examples=joined.overlap_examples;
  record.full_comparison=difference(a,joined);
  if(!record.repeat_comparison.equal)throw S().fault('CONTEXT_REFERENCE_UNSTABLE','Repeated local whole-range pictures differ.',record.repeat_comparison,true);
  if(record.overlap_mismatches||!record.full_comparison.equal)throw S().fault('CONTEXT_REFERENCE_MISMATCH','Guarded pieces did not reproduce the whole reference exactly.',{overlap_mismatches:record.overlap_mismatches,comparison:record.full_comparison},true);
  record.status='complete';
 }catch(e){record.status='failed';record.error=S().errorInfo(e);throw e;}
 finally{record.total_ms=performance.now()-start;session.timings.context_validation_ms=(session.timings.context_validation_ms||0)+record.total_ms;
  await session.event('context_reference_finished',record);await session.checkpoint('context_reference_finished',{sheet,status:record.status});}
}
const api={difference,compose,verify};root.ContextValidation=api;if(typeof module!=='undefined'&&module.exports)module.exports=api;
}(typeof window!=='undefined'?window:globalThis));
