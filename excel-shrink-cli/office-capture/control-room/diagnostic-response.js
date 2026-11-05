/** Distinguish an older server's missing route from missing or invalid evidence. */
export async function readDiagnosticResponse(response, feature) {
  const text = await response.text();
  let value;
  if (text.trim()) {
    try { value = JSON.parse(text); }
    catch {
      if (response.ok) throw new Error(`${feature}: invalid server JSON response (HTTP ${response.status}).`);
    }
  }
  if (!response.ok) {
    if (typeof value?.error === 'string') throw new Error(value.error);
    if (response.status === 404 && !text.trim()) {
      const error = new Error(`${feature} is not supported by this server (HTTP 404). Open the current run's Observatory link; this page may be connected to an older server.`);
      error.code = 'unsupported-diagnostic-endpoint';
      throw error;
    }
    throw new Error(`${feature}: server request failed (HTTP ${response.status}).`);
  }
  if (value == null) throw new Error(`${feature}: empty server response (HTTP ${response.status}).`);
  return value;
}
