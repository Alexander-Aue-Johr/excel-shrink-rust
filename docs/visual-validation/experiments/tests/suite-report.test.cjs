'use strict';
const assert=require('node:assert/strict'),path=require('node:path');
const S=require('../_shared/suite/suite.js');let n=0;
function test(name,f){f();n++;console.log('PASS '+name);}
function tile(o={}){return {workbook:'w.xlsx',sheet:' S',rect:{r0:0,r1:2,c0:0,c1:2},width:20,height:40,offset:{x:0,y:0},encoded_sha256:'a',raw_sha256:'raw',raw_address:'A1:D2',...o};}
function job(tiles,status='complete',ms=1000){return {phase:'measured',candidate_id:'x',candidate:{label:'X',group:'current-capture'},status,metrics:{captures:tiles,workload_ms:ms}};}
test('median ignores missing values, not genuine zero',()=>{assert.equal(S.median([null,undefined,0,20,30]),20);assert.equal(S.median([0,20]),10);assert.equal(S.median([]),null);});
test('stable serialization ignores object property order',()=>assert.equal(S.stable({b:2,a:1}),S.stable({a:1,b:2})));
test('paths reject traversal and external protocols',()=>{for(const p of ['../x','/x','a/../x','https://evil','a\\b','a?x'])assert.equal(S.path(p),null);assert.equal(S.path('cases/a/report.html'),'cases/a/report.html');});
test('identical encoded bytes establish equality',()=>{const r=S.comparison(job([tile()]),job([tile()]));assert.equal(r.status,'pixel-equivalent');assert.equal(r.rawEqual,1);});
test('different encodings without pixel hashes remain unverified',()=>assert.equal(S.comparison(job([tile()]),job([tile({encoded_sha256:'b'})])).status,'unverified'));
test('matching normalized pixels allow different encoders',()=>{const d={algorithm:'vv-bgra32-v1',sha256:'pixels',width:20,height:40};assert.equal(S.comparison(job([tile({pixel_digest:d})]),job([tile({encoded_sha256:'b',pixel_digest:d})])).status,'pixel-equivalent');});
test('different normalized pixels are reported as a mismatch',()=>{const d={algorithm:'vv-bgra32-v1',sha256:'pixels',width:20,height:40};assert.equal(S.comparison(job([tile({pixel_digest:d})]),job([tile({encoded_sha256:'b',pixel_digest:{...d,sha256:'other'}})])).status,'pixel-mismatch');});
test('missing local image never becomes verified through a metadata hash',()=>assert.equal(S.comparison(job([tile({pixel_issue:'missing'})]),job([tile()])).status,'unverified'));
test('different tile boundaries are not mislabeled pixel failure',()=>assert.equal(S.comparison(job([tile()]),job([tile({rect:{r0:0,r1:1,c0:0,c1:2}})])).status,'different-layout'));
test('duplicate tile identities are rejected',()=>assert.equal(S.comparison(job([tile(),tile()]),job([tile()])).status,'invalid'));
test('no captures remain unverified',()=>assert.equal(S.comparison(job([]),job([])).status,'unverified'));
test('failed and warm-up durations do not enter medians',()=>{const a=job([], 'complete',2000),b=job([],'failed',5),c={...job([],'complete',50000),phase:'warmup'};const g=S.summaries({jobs:[a,b,c]})[0];assert.equal(g.median_ms,2000);assert.equal(g.complete,1);assert.equal(g.total,2);});
test('phase metrics remain unknown when absent',()=>{assert.equal(S.phase(job([]),'render_ms'),null);assert.equal(S.phase({metrics:{workbooks:[{kind:'capture',timings:{render_ms:0}},{kind:'capture',timings:{render_ms:3}}]}},'render_ms'),3);});
test('sheet whitespace is part of tile identity',()=>assert.equal(S.comparison(job([tile()]),job([tile({sheet:'S'})])).status,'different-layout'));
test('changed renderer state withholds a pooled median',()=>{const a=job([], 'complete',1000),b=job([], 'complete',2000);a.metrics.workbooks=[{kind:'capture',workbook:'w',environment:{office:{version:'1'}}}];b.metrics.workbooks=[{kind:'capture',workbook:'w',environment:{office:{version:'2'}}}];const g=S.summaries({jobs:[a,b]})[0];assert.equal(g.environment_variants,2);assert.equal(g.median_ms,null);});
test('inconsistent decoded evidence is not hidden by an encoded metadata hash',()=>{const d={algorithm:'vv-bgra32-v1',sha256:'p',width:20,height:40};assert.equal(S.comparison(job([tile({pixel_digest:d})]),job([tile({pixel_digest:{...d,sha256:'q'}})])).status,'pixel-mismatch');});
console.log(`${n} suite report logic tests passed. Windows/Excel integration is not exercised.`);
