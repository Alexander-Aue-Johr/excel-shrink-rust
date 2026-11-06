/** Read one declared option without interpreting positional paths or shell text. */
export function optionValue(argumentsList, option) {
  const flag = `--${option.name}`;
  const index = argumentsList.findIndex(value => value === flag || value.startsWith(`${flag}=`));
  if (index < 0) return option.kind === 'boolean' ? false : '';
  if (option.kind === 'boolean') return true;
  if (argumentsList[index].startsWith(`${flag}=`)) return argumentsList[index].slice(flag.length + 1);
  if (option.kind === 'list') {
    const values = [];
    for (let cursor = index + 1; cursor < argumentsList.length && !argumentsList[cursor].startsWith('--'); cursor++) {
      values.push(argumentsList[cursor]);
    }
    return values.join('\n');
  }
  return argumentsList[index + 1] ?? '';
}

/** Replace one option while preserving every other explicit argument and its order. */
export function setOption(argumentsList, option, value) {
  const flag = `--${option.name}`;
  const result = [];
  for (let index = 0; index < argumentsList.length; index++) {
    const argument = argumentsList[index];
    if (argument.startsWith(`${flag}=`)) continue;
    if (argument !== flag) {
      result.push(argument);
      continue;
    }
    if (option.kind === 'list') {
      while (index + 1 < argumentsList.length && !argumentsList[index + 1].startsWith('--')) index++;
    } else if (option.kind !== 'boolean' && index + 1 < argumentsList.length) {
      index++;
    }
  }
  if (option.kind === 'boolean') {
    if (value) result.push(flag);
  } else if (String(value).trim()) {
    const values = option.kind === 'list'
      ? String(value).split(/\r?\n/).map(line => line.trim()).filter(Boolean)
      : [String(value)];
    result.push(flag, ...values);
  }
  return result;
}
