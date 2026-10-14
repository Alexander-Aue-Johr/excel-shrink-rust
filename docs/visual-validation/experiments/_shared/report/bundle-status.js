/* File://-safe publication state; no requests to missing bundle URLs. */
(function(root) {
'use strict';
function apply(doc,state) {
    state=state||{status:'unavailable'};
    var ready=state.status==='ready'&&!state.metadata_copy;
    var links=doc.querySelectorAll('a[href="review-metadata.zip"],a[data-vv-bundle]');
    for(var i=0;i<links.length;i++) {
        var a=links[i];a.setAttribute('data-vv-bundle','true');
        if(ready){a.setAttribute('href','review-metadata.zip');a.removeAttribute('aria-disabled');a.removeAttribute('title');}
        else{a.removeAttribute('href');a.setAttribute('aria-disabled','true');
            a.setAttribute('title',state.metadata_copy?'This is a metadata copy; the archive is not nested inside itself.':(state.message||'Bundle not published yet.'));}
    }
    if(!ready&&!doc.getElementById('vv-publication-note')) {
        var note=doc.createElement('p');note.id='vv-publication-note';note.className='callout';
        note.textContent=state.metadata_copy?'Metadata copy: original screenshots are not included.':
            'Bundle publication: '+state.status+'. '+(state.message||'Run the report-repair task; Excel does not need to run again.');
        var target=doc.querySelector('.hero')||doc.body;
        target.appendChild(note);
    }
}
root.VVBundleStatus={apply:apply};
if(typeof module!=='undefined'&&module.exports)module.exports={apply:apply};
if(typeof document!=='undefined') {
    var start=function(){apply(document,root.VV_BUNDLE_STATE);};
    if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',start);else start();
}
})(typeof window!=='undefined'?window:globalThis);
