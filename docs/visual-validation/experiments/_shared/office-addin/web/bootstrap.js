(function () {
  'use strict';

  function send(stage, detail) {
    try {
      var xhr = new XMLHttpRequest();
      xhr.open(
        'GET',
        '/client-beacon?stage=' + encodeURIComponent(stage) +
          '&detail=' + encodeURIComponent(JSON.stringify(detail || null)) +
          '&t=' + String(Date.now()),
        true
      );
      xhr.send();
    } catch (_) {
    }
  }

  window.__visualValidationBootstrapEvent = send;

  send('bootstrap_loaded', {
    office_defined: typeof window.Office !== 'undefined',
    excel_defined: typeof window.Excel !== 'undefined',
    ready_state: document.readyState
  });

  window.addEventListener('error', function (event) {
    send('window_error', {
      message: event.message || null,
      filename: event.filename || null,
      line: event.lineno || null
    });
  }, true);

  window.addEventListener('unhandledrejection', function (event) {
    send('unhandled_rejection', {
      reason: event.reason ? String(event.reason) : null
    });
  });
}());
