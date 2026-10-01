(function () {
  'use strict';

  function encodeDetail(detail) {
    try {
      return encodeURIComponent(JSON.stringify(detail || null));
    } catch (_) {
      return '';
    }
  }

  function reportBootstrapEvent(stage, detail) {
    try {
      var xhr = new XMLHttpRequest();
      var url =
        '/client-beacon?stage=' +
        encodeURIComponent(stage) +
        '&detail=' +
        encodeDetail(detail) +
        '&t=' +
        String(Date.now());

      xhr.open('GET', url, true);
      xhr.send();
    } catch (_) {
    }
  }

  window.__drawingBoundsReportBootstrapEvent = reportBootstrapEvent;

  reportBootstrapEvent('bootstrap_loaded', {
    office_defined: typeof window.Office !== 'undefined',
    excel_defined: typeof window.Excel !== 'undefined',
    document_ready_state: document.readyState,
    user_agent: navigator.userAgent
  });

  window.addEventListener('error', function (event) {
    var target = event.target;

    if (target && target.src) {
      reportBootstrapEvent('resource_load_error', {
        tag: target.tagName || null,
        src: target.src
      });
      return;
    }

    reportBootstrapEvent('window_error', {
      message: event.message || null,
      filename: event.filename || null,
      lineno: event.lineno || null,
      colno: event.colno || null
    });
  }, true);

  window.addEventListener('unhandledrejection', function (event) {
    reportBootstrapEvent('unhandled_rejection', {
      reason: event.reason ? String(event.reason) : null
    });
  });

  window.setTimeout(function () {
    reportBootstrapEvent('office_js_watchdog_15s', {
      office_defined: typeof window.Office !== 'undefined',
      excel_defined: typeof window.Excel !== 'undefined',
      document_ready_state: document.readyState
    });
  }, 15000);
}());
