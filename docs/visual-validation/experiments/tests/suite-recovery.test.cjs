const test=require('node:test');
const assert=require('node:assert/strict');
const path=require('node:path');
const fs=require('node:fs');
const suite=require('../_shared/suite/suite.js');
const bundle=require('../_shared/report/bundle-status.js');
function trial(algorithm,sha='abc') {
 const digest={algorithm,sha256:sha,width:2,height:1};
 return {metrics:{captures:[{workbook:'sample.xlsx',sheet:'Sheet 1',rect:{r0:0,r1:1,c0:0,c1:1},
    width:2,height:1,offset:{x:0,y:0},pixel_digest:digest,encoded_sha256:'not-the-same'}]}};
}
test('GDI+ canonical digests can be compared',()=>{
 const a=trial('vv-bgra32-gdiplus-v1'),b=trial('vv-bgra32-gdiplus-v1');
 b.metrics.captures[0].encoded_sha256='another-png-encoding';
 assert.equal(suite.comparison(a,b).status,'pixel-equivalent');
});
test('Legacy digest comparison is retained',()=>{
 const a=trial('vv-bgra32-v1'),b=trial('vv-bgra32-v1');
 assert.equal(suite.comparison(a,b).status,'pixel-equivalent');
});
test('Different digest backends cannot be compared as equivalent',()=>{
 const a=trial('vv-bgra32-v1'),b=trial('vv-bgra32-gdiplus-v1');
 b.metrics.captures[0].encoded_sha256='different-encoding';
 assert.equal(suite.comparison(a,b).status,'unverified');
});
test('Matching algorithms with differing pixels are a mismatch',()=>{
 assert.equal(suite.comparison(trial('vv-bgra32-gdiplus-v1','a'),trial('vv-bgra32-gdiplus-v1','b')).status,'pixel-mismatch');
});
function fakeDocument() {
 const anchor={attrs:{href:'review-metadata.zip'},setAttribute(k,v){this.attrs[k]=v;},removeAttribute(k){delete this.attrs[k];}};
 const notes=[]; const hero={appendChild(n){notes.push(n);}};
 return {anchor,notes,querySelectorAll(){return[anchor]},getElementById(){return null},querySelector(){return hero},
    createElement(){return{}},body:hero};
}
test('A failed or in-progress archive has no clickable file URL',()=>{
 for(const status of ['failed','building','unavailable']){const d=fakeDocument();bundle.apply(d,{status});assert.equal(d.anchor.attrs.href,undefined);assert.equal(d.notes.length,1);}
});
test('A verified published archive is linked',()=>{
 const d=fakeDocument();bundle.apply(d,{status:'ready'});assert.equal(d.anchor.attrs.href,'review-metadata.zip');assert.equal(d.notes.length,0);
});
test('Metadata copies never link to a nested copy of their own archive',()=>{
 const d=fakeDocument();bundle.apply(d,{status:'ready',metadata_copy:true});assert.equal(d.anchor.attrs.href,undefined);
});
test('Report repair entry point has no Excel launch invocation',()=>{
 const s=fs.readFileSync(path.join(__dirname,'../repair-suite.ps1'),'utf8');
 assert.doesNotMatch(s,/\bNew-Object\s+-ComObject\b|\bStart-Process\b|range-image\\run\.ps1|Excel\.run/);
 assert.match(s,/Get-VVCaseMetrics/);
 assert.match(s,/workload_ms/);
});
