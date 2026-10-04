
'use strict';
const assert=require('node:assert/strict'),path=require('node:path');
const H=require('../_shared/report/history.js'),R=require('../_shared/report/inspector.js');
let n=0;function test(name,fn){fn();n++;console.log('PASS '+name);}
test('history has stable descending order without changing recorded statuses',()=>{const x=H.normalize({runs:[{id:'20260101',status:'failed'},{id:'20261001',status:'complete'}]});assert.equal(x[0].id,'20261001');assert.equal(x[1].status,'failed');});
test('history filters by status, experiment, workbook and diagnostic text',()=>{const x=H.normalize({runs:[{id:'a',status:'failed',experiment:'range-image',arguments:{Workbook:'hospital.xlsx'},errors:[{message:'NATIVE_TIMEOUT'}]},{id:'b',status:'complete',experiment:'calibration'}]});assert.equal(H.filterRuns(x,'hospital','failed','range-image').length,1);assert.equal(H.filterRuns(x,'timeout','','').length,1);assert.equal(H.filterRuns(x,'','complete','range-image').length,0);});
test('history paths reject protocol and traversal injection',()=>{for(const x of ['javascript:alert(1)','../secret','x\\y','/root','a?b','x/#y'])assert.equal(H.safePath(x),null);assert.equal(H.safePath('runs/a/report.html'),'runs/a/report.html');});
test('A/B arms stay distinct; failed B progress is not replaced with successful A',()=>{
 const make=(mode)=>({workbook:'Book.xlsx',strategy:'F',capture_options:{overflow_mode:mode},sheets:[{sheet:'S',status:mode==='heuristic'?'complete':'inconclusive'}],attempts:[],sparse_overflow_checks:[]});
 const input={files:[],documents:[{path:'data/range-image/f/dotnet-heuristic.json',workbooks:[{workbook:'Book.xlsx',safe_capture:make('heuristic')}]},{path:'artifacts/range-image/f/dotnet-selective/_progress/Book.xlsx.progress.json',progress:make('selective')}]};
 const m=R.buildModel(input);assert.equal(m.sessions.length,2);assert.equal(m.sessions[1].sheets[0].status,'inconclusive');assert.equal(m.sessions[1].options.overflow_mode,'selective');
});
test('verification pictures do not become final captures',()=>{
 const x={files:[{path:'artifacts/range-image/f/dotnet/Book.xlsx/sparse-before.png',width:51,height:21,bytes:50}],documents:[{path:'data/range-image/f/dotnet.json',workbooks:[{workbook:'Book.xlsx',safe_capture:{sheets:[{sheet:'S',status:'complete'}],sparse_overflow_checks:[{sheet:'S',id:'sparse',restoration:'verified',address:'A1:E1',artifacts:[{id:'sparse-before',relative_png:'Book.xlsx/sparse-before.png',role:'before'}]}]}}]}]};
 const s=R.buildModel(x).sessions[0].sheets[0];assert.equal(s.images[0].kind,'verification');assert.equal(s.images[0].role,'before');assert.equal(s.sparse_checks.length,1);
});
console.log(`${n} history/variant projection tests passed.`);
