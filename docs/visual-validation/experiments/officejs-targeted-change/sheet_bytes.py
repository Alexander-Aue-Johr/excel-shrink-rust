"""Stream the controlled numeric worksheet as row and cell byte records.

This scanner is deliberately limited to the fixture's unprefixed worksheet
tags and numeric A-column cells. It does not build an XML tree or cell map.
"""

from pathlib import Path
import xml.etree.ElementTree as ET
import zipfile


SHEET_PART = 'xl/worksheets/sheet2.xml'
ROW_OPEN = b'<row '
ROW_CLOSE = b'</row>'
CELL_OPEN = b'<c '
CELL_CLOSE = b'</c>'
SHEET_OPEN = b'<sheetData'
SHEET_CLOSE = b'</sheetData>'
MAIN = '{http://schemas.openxmlformats.org/spreadsheetml/2006/main}'


def segments(stream):
    """Yield complete sheetData rows and intervening bytes from a ZIP member."""
    buffer = b''
    state = 'before_sheet_data'
    while chunk := stream.read(64 * 1024):
        buffer += chunk
        while True:
            if state == 'before_sheet_data':
                start = buffer.find(SHEET_OPEN)
                if start < 0:
                    safe = len(buffer) - len(SHEET_OPEN) + 1
                    if safe > 0:
                        yield 'other', buffer[:safe]
                        buffer = buffer[safe:]
                    break
                end = buffer.find(b'>', start)
                if end < 0:
                    if start:
                        yield 'other', buffer[:start]
                        buffer = buffer[start:]
                    break
                yield 'other', buffer[:end + 1]
                buffer = buffer[end + 1:]
                state = 'within_sheet_data'
                continue
            if state == 'after_sheet_data':
                yield 'other', buffer
                buffer = b''
                break
            start = buffer.find(ROW_OPEN)
            if start < 0:
                sheet_end = buffer.find(SHEET_CLOSE)
                if sheet_end >= 0:
                    end = sheet_end + len(SHEET_CLOSE)
                    yield 'other', buffer[:end]
                    buffer = buffer[end:]
                    state = 'after_sheet_data'
                    continue
                safe = len(buffer) - max(len(ROW_OPEN), len(SHEET_CLOSE)) + 1
                if safe > 0:
                    yield 'other', buffer[:safe]
                    buffer = buffer[safe:]
                break
            if start:
                yield 'other', buffer[:start]
                buffer = buffer[start:]
            end = buffer.find(ROW_CLOSE)
            if end < 0:
                break
            end += len(ROW_CLOSE)
            yield 'row', buffer[:end]
            buffer = buffer[end:]
    if state != 'after_sheet_data':
        raise ValueError('Worksheet sheetData was incomplete')
    if buffer:
        yield 'other', buffer


def attribute(tag, name):
    """Read one double-quoted attribute from a controlled opening tag."""
    marker = b' ' + name + b'="'
    start = tag.find(marker)
    if start < 0:
        return None
    start += len(marker)
    end = tag.find(b'"', start)
    if end < 0:
        raise ValueError(f'Unclosed {name.decode()} attribute')
    return tag[start:end]


def cells(row):
    """Yield address, bytes, and offsets of each numeric cell in one row."""
    cursor = 0
    while (start := row.find(CELL_OPEN, cursor)) >= 0:
        opening_end = row.find(b'>', start)
        if opening_end < 0:
            raise ValueError('Unclosed cell tag')
        opening = row[start:opening_end + 1]
        if opening.endswith(b'/>'):
            end = opening_end + 1
        else:
            closing = row.find(CELL_CLOSE, opening_end)
            if closing < 0:
                raise ValueError('Unclosed cell body')
            end = closing + len(CELL_CLOSE)
        address = attribute(opening, b'r')
        if address is None or not address.startswith(b'A') or not address[1:].isdigit():
            raise ValueError('Fixture contains an unexpected cell address')
        yield address.decode('ascii'), row[start:end], start, end
        cursor = end


def value(cell):
    """Return the numeric value and style index from one cell record."""
    opening_end = cell.find(b'>')
    style = attribute(cell[:opening_end + 1], b's')
    start = cell.find(b'<v>', opening_end)
    if start < 0:
        return None, style
    start += 3
    end = cell.find(b'</v>', start)
    if end < 0:
        raise ValueError('Unclosed numeric value')
    return cell[start:end], style


def fill_colors(archive):
    """Resolve style indexes to colors; Excel renumbers them after SaveAs."""
    root = ET.fromstring(archive.read('xl/styles.xml'))
    fills = root.find(f'{MAIN}fills')
    xfs = root.find(f'{MAIN}cellXfs')
    if fills is None or xfs is None:
        raise ValueError('Fixture style table is incomplete')
    colors = []
    for fill in fills:
        foreground = fill.find(f'{MAIN}patternFill/{MAIN}fgColor')
        colors.append(foreground.get('rgb') if foreground is not None else None)
    return [colors[int(xf.get('fillId', '0'))] for xf in xfs]


def records(path):
    """Yield sorted row/value/color records using bounded row-sized buffers."""
    with zipfile.ZipFile(path) as archive:
        colors = fill_colors(archive)
        with archive.open(SHEET_PART) as sheet:
            for kind, raw in segments(sheet):
                if kind == 'row':
                    for address, cell, _, _ in cells(raw):
                        numeric, style = value(cell)
                        style_index = int(style) if style is not None else 0
                        yield (int(address[1:]),
                               numeric.decode('ascii') if numeric is not None else None,
                               colors[style_index])


def changed_cells(original, variant):
    """Merge two ordered streams without retaining either worksheet in memory."""
    left = iter(records(original))
    right = iter(records(variant))
    a = next(left, None)
    b = next(right, None)
    while a is not None or b is not None:
        if b is None or a is not None and a[0] < b[0]:
            yield {'address': f'A{a[0]}', 'before': a[1], 'after': None, 'kind': 'removed'}
            a = next(left, None)
        elif a is None or b[0] < a[0]:
            yield {'address': f'A{b[0]}', 'before': None, 'after': b[1], 'kind': 'added'}
            b = next(right, None)
        else:
            if a[1:] != b[1:]:
                yield {'address': f'A{a[0]}', 'before': a[1], 'after': b[1],
                       'kind': 'value_or_style_changed'}
            a = next(left, None)
            b = next(right, None)


def mutate_row(row, changes, seen):
    """Delete one cell or replace its numeric bytes while preserving other XML."""
    pieces = []
    cursor = 0
    for address, cell, start, end in cells(row):
        if address not in changes:
            continue
        pieces.append(row[cursor:start])
        replacement = changes[address]
        if replacement is not None:
            value_start = cell.find(b'<v>') + 3
            value_end = cell.find(b'</v>', value_start)
            if value_start < 3 or value_end < 0:
                raise ValueError(f'No numeric value at {address}')
            pieces.append(cell[:value_start] + replacement + cell[value_end:])
        cursor = end
        seen.add(address)
    pieces.append(row[cursor:])
    return b''.join(pieces)


def make_variant(original, target, changes):
    """Copy the XLSX, streaming only the target sheet through the row mutator."""
    seen = set()
    with zipfile.ZipFile(original) as source, zipfile.ZipFile(target, 'w') as output:
        for info in source.infolist():
            if info.filename != SHEET_PART:
                output.writestr(info, source.read(info))
                continue
            with source.open(info) as sheet, output.open(info, 'w', force_zip64=True) as changed:
                for kind, raw in segments(sheet):
                    changed.write(mutate_row(raw, changes, seen) if kind == 'row' else raw)
    if seen != set(changes):
        Path(target).unlink(missing_ok=True)
        raise ValueError(f'Expected changes {set(changes)}, applied {seen}')
