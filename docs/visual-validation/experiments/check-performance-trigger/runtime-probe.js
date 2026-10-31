/** Enumerate the actual Office.js surface without invoking unknown methods. */
async function probeCheckPerformance(command, office) {
  function describe(object) {
    const members = new Map();
    for (let depth = 0; object && depth < 6; depth++) {
      for (const [name, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(object))) {
        if (!members.has(name)) {
          members.set(name, {name, kind: typeof descriptor.value,
            getter: typeof descriptor.get === 'function', depth});
        }
      }
      object = Object.getPrototypeOf(object);
    }
    return [...members.values()].sort((a, b) => a.name.localeCompare(b.name));
  }

  return office(async context => {
    // Deliberately expose real proxies for the debugger, not a mocked API.
    globalThis.cpProbe = {
      Office, Excel, context, workbook: context.workbook,
      application: context.workbook.application,
      worksheet: context.workbook.worksheets.getFirst(),
      range: context.workbook.worksheets.getFirst().getRange('A1'),
      ribbon: Office.ribbon, actions: Office.actions,
      document: Office.context.document
    };
    if (command.breakpoint) {
      debugger; // The isolated CDP client records this pause and resumes it.
    }
    const objects = {};
    for (const [name, object] of Object.entries(globalThis.cpProbe)) {
      const members = describe(object);
      objects[name] = {members, candidates: members.filter(member =>
        /performance|optimi|cleanup|command|ribbon/i.test(member.name))};
    }
    return {operation: command.operation, objects,
      office_version: Office.context.diagnostics?.version,
      host: Office.context.diagnostics?.host,
      unknown_methods_invoked: false};
  });
}
