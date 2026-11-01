"""Recover an exact CF subtree without searching the original corpus again."""
import argparse
import json
from pathlib import Path
import zipfile


def extract(root: Path, source_id: int, part: str, block_index: int, output: Path) -> None:
    with (root / 'sheets.jsonl').open(encoding='utf-8') as records:
        sheet = None
        for line in records:
            item = json.loads(line)
            if item['source_id'] == source_id and item['part'] == part:
                sheet = item
                break
    if sheet is None:
        raise ValueError(f'No worksheet record for source {source_id}, part {part}')
    block = sheet['blocks'][block_index]
    if block['raw_xml'] is not None:
        data = block['raw_xml'].encode('utf-8')
    elif block['raw_file'] is not None:
        data = (root / block['raw_file']).read_bytes()
    else:
        # ZIP seeks may decompress the prefix, but no structural XML search is needed.
        with zipfile.ZipFile(root / f'sources/{source_id:05}.xlsx') as archive:
            with archive.open(part) as source:
                source.seek(block['start'])
                data = source.read(block['end'] - block['start'])
    assert len(data) == block['end'] - block['start'], 'Incomplete source evidence'
    with output.open('xb') as target:
        target.write(data)
    print(f'{len(data)} exact XML bytes written to {output}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('inventory', type=Path)
    parser.add_argument('--source-id', type=int, required=True)
    parser.add_argument('--part', required=True)
    parser.add_argument('--block', type=int, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    extract(args.inventory, args.source_id, args.part, args.block, args.output)
