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
    get height(){return sum(ys,r.r1)-sum(ys,r.r0);},get width(){return sum(xs,r.c1)-sum(xs,r.c0);},get rowHidden(){return ys(r.r0)===0;},get columnHidden(){return o.zeroNotHidden?false:xs(r.c0)===0;},
    getMergedAreasOrNullObject(){const merges=(o.merges||[]).filter(m=>m.r0<r.r1&&m.r1>r.r0&&m.c0<r.c1&&m.c1>r.c0);return{load(){},isNullObject:!merges.length,areas:{items:merges.map(m=>({rowIndex:m.r0,rowCount:m.r1-m.r0,columnIndex:m.c0,columnCount:m.c1-m.c0}))}};},
    getImage(){const result={};queue.push(()=>{rendered.push(r);if(o.renderError&&o.renderError(r))throw new Error('Renderer rejected');
      const width=Math.round((sum(xs,r.c1)-sum(xs,r.c0))*sx)+1+(o.adjustWidth?o.adjustWidth(r,rendered.length):0),height=Math.round((sum(ys,r.r1)-sum(ys,r.r0))*sy)+1;
      result.value=png(width,height,{top:Math.round(sum(ys,r.r0)*sy),left:Math.round(sum(xs,r.c0)*sx),r0:r.r0});});return result;}};}
  return fn({workbook:{worksheets:{getItem(sheet){return{name:sheet,load(){},getRange:a=>range(sheet,S.parse(a)),getRangeByIndexes:(r,c,n,m)=>range(sheet,{r0:r,r1:r+n,c0:c,c1:c+m})};}}},trace(){},sync:async()=>{while(queue.length)queue.shift()();}});
 }};
 return{posts,queries,rendered,frames};
}

module.exports={mock,S,G,P,V,web};
