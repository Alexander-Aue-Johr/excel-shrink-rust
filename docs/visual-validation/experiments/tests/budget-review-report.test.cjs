'use strict';
const assert=require('node:assert/strict'),path=require('node:path');
const R=require(path.join(__dirname,'../_shared/report/inspector.js'));
const safe={workbook:'test.xlsx',sheets:[{sheet:'S',status:'inconclusive',tiles:[]}],attempts:[],captures:[],
 sparse_overflow_scans:[{id:'scan1',sheet:'S',source_tile:'t1',status:'inconclusive',reason:'SPARSE_CANDIDATE_BUDGET',candidate_density:1}],
 sparse_overflow_checks:[]};
const m=R.buildModel({files:[],documents:[{path:'data/run.json',workbooks:[{workbook:'test.xlsx',safe_capture:safe}]}]});
assert.equal(m.sessions[0].sheets[0].sparse_scans[0].reason,'SPARSE_CANDIDATE_BUDGET');
assert.equal(m.sessions[0].sheets[0].sparse_scans[0].candidate_density,1);
assert.equal(m.sessions[0].sheets[0].sparse_checks.length,0);
console.log('PASS report retains unresolved scan without an intervention');
const legacy=R.buildModel({files:[],documents:[{path:'data/old.json',workbooks:[{workbook:'old.xlsx',safe_capture:{sheets:[{sheet:'Old',status:'complete'}]}}]}]});
assert.deepEqual(legacy.sessions[0].sheets[0].sparse_scans,[]);
console.log('PASS old reports have no invented candidate evidence');
