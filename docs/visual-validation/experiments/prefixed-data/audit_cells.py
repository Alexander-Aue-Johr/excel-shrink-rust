"""Compare expanded cell payloads while allowing Excel's explicit numeric default to disappear."""
import argparse
import json
import zipfile
from collections import deque
from pathlib import Path
from xml.parsers import expat


def cells(stream):
    queue = deque()
    current = None
    parser = expat.ParserCreate(namespace_separator='|')

    def start(name, attrs):
        nonlocal current
        local = name.rsplit('|', 1)[-1]
        if local == 'c':
            address = attrs['r']
            attrs = dict(attrs)
            attrs.pop('r')
            if attrs.get('t') == 'n':
                attrs.pop('t')
            current = [address, [], False]
        if current is not None:
            current[1].append(('start', name, sorted(attrs.items())))
            current[2] |= local in ('v', 'f', 'is', 't')

    def end(name):
        nonlocal current
        if current is not None:
            current[1].append(('end', name))
            if name.rsplit('|', 1)[-1] == 'c':
                queue.append(current)
                current = None

    def text(value):
        if current is not None:
            events = current[1]
            if events and events[-1][0] == 'text':
                events[-1] = ('text', events[-1][1] + value)
            else:
                events.append(('text', value))

    parser.StartElementHandler = start
    parser.EndElementHandler = end
    parser.CharacterDataHandler = text
    while chunk := stream.read(65536):
        parser.Parse(chunk, False)
        while queue:
            yield queue.popleft()
    parser.Parse(b'', True)
    yield from queue


def coordinate(cell):
    address = cell[0]
    split = next(i for i, char in enumerate(address) if char.isdigit())
    column = 0
    for letter in address[:split]:
        column = column * 26 + ord(letter) - ord('A') + 1
    return int(address[split:]), column


def audit(before, after):
    counts = {'unchanged_cells': 0, 'removed_empty_unstyled_cells': 0, 'other_changes': 0, 'examples': []}
    with zipfile.ZipFile(before) as left, zipfile.ZipFile(after) as right:
        parts = [name for name in left.namelist() if name.startswith('xl/worksheets/') and name.endswith('.xml')]
        for part in parts:
            with left.open(part) as a, right.open(part) as b:
                first, second = iter(cells(a)), iter(cells(b))
                x, y = next(first, None), next(second, None)
                while x is not None or y is not None:
                    if x is not None and y is not None and coordinate(x) == coordinate(y):
                        if x == y:
                            counts['unchanged_cells'] += 1
                        else:
                            counts['other_changes'] += 1
                            if len(counts['examples']) < 5:
                                counts['examples'].append({'before': x, 'after': y})
                        x, y = next(first, None), next(second, None)
                    elif x is not None and (y is None or coordinate(x) < coordinate(y)):
                        # Only an empty cell with no formatting or other attributes is harmless here.
                        empty = not x[2] and len(x[1]) == 2 and not x[1][0][2]
                        counts['removed_empty_unstyled_cells' if empty else 'other_changes'] += 1
                        if not empty and len(counts['examples']) < 5:
                            counts['examples'].append({'before': x, 'after': None})
                        x = next(first, None)
                    else:
                        counts['other_changes'] += 1
                        if len(counts['examples']) < 5:
                            counts['examples'].append({'before': None, 'after': y})
                        y = next(second, None)
    return counts


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    with args.output.open('x', encoding='utf-8') as output:
        json.dump(audit(args.before, args.after), output, indent=2)
