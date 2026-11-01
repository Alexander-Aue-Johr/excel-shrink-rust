"""Compare decoded probe images and geometry, then build a local review report."""

import argparse
from html import escape
import json
from pathlib import Path

from PIL import Image

VARIANTS = ('single', 'all', 'below', 'one-sentinel', 'no-sentinel', 'fixed', 'hidden', 'blue')


def compare_probes(before, after, directory):
    """Compare pixels independently of PNG encoding and compare measured geometry."""
    original_captures = {capture['address']: capture for capture in before['captures']}
    variant_captures = {capture['address']: capture for capture in after['captures']}
    if original_captures.keys() != variant_captures.keys():
        raise ValueError('Probe capture ranges differ; this is not a valid comparison.')
    captures = []
    for address, original in original_captures.items():
        variant = variant_captures[address]
        paths = [Path(capture['image_path']) for capture in (original, variant)]
        with Image.open(paths[0]) as left, Image.open(paths[1]) as right:
            equal = left.size == right.size and left.convert('RGBA').tobytes() == right.convert('RGBA').tobytes()
        captures.append({
            'range': address,
            'pixels_equal': equal,
            'original_image': paths[0].relative_to(directory).as_posix(),
            'variant_image': paths[1].relative_to(directory).as_posix(),
        })
    original_rows = {row['address']: row for row in before['rows']}
    variant_rows = {row['address']: row for row in after['rows']}
    if original_rows.keys() != variant_rows.keys():
        raise ValueError('Probe row addresses differ.')
    geometry_fields = ('top', 'height', 'row_height')
    changed_rows = [
        address for address, row in original_rows.items()
        if any(row[field] != variant_rows[address][field] for field in geometry_fields)
    ]
    return {
        'original': before['name'],
        'variant': after['name'],
        'captured_ranges': captures,
        'rows_checked': len(original_rows),
        'changed_row_geometry': changed_rows,
        'whole_tail_before': before['whole_tail'],
        'whole_tail_after': after['whole_tail'],
    }


def collect_comparisons(directory):
    probes = {}
    for folder in ('visual', 'height-only-visual', 'electricity-visual', 'electricity-control-visual'):
        path = directory / folder / 'results.json'
        for probe in json.loads(path.read_text(encoding='utf-8')):
            probes[probe['name']] = probe
    pairs = [('real-control', 'real-default'), ('real-control', 'real-maximum')]
    pairs.extend((f'{mode}-control', f'{mode}-default') for mode in VARIANTS)
    pairs.extend([
        ('below-control', 'one-sentinel-control'),
        ('below-control', 'no-sentinel-control'),
        ('height-only-control', 'height-only-default'),
        ('electricity-9-default', 'electricity-9-sentinel'),
        ('electricity-10-default', 'electricity-10-sentinel'),
    ])
    for index in (9, 10):
        pairs.extend([
            (f'electricity-{index}-control', f'electricity-{index}-default'),
            (f'electricity-{index}-control', f'electricity-{index}-sentinel'),
        ])
    return [compare_probes(probes[left], probes[right], directory) for left, right in pairs]


def render_comparison(comparison):
    """Keep images at their original resolution, with browser scaling for review."""
    title = escape(f"{comparison['original']} → {comparison['variant']}")
    changed = comparison['changed_row_geometry']
    all_equal = all(capture['pixels_equal'] for capture in comparison['captured_ranges'])
    verdict = 'Measured ranges match' if all_equal and not changed else 'Visible or geometry difference'
    pictures = []
    for capture in comparison['captured_ranges']:
        label = 'identical pixels' if capture['pixels_equal'] else 'different pixels or dimensions'
        pictures.append(f"""
        <details>
          <summary>{escape(capture['range'])}: {label}</summary>
          <div class="images">
            <figure><figcaption>Before</figcaption>
              <a href="{capture['original_image']}"><img src="{capture['original_image']}" loading="lazy" alt="Before"></a>
            </figure>
            <figure><figcaption>After</figcaption>
              <a href="{capture['variant_image']}"><img src="{capture['variant_image']}" loading="lazy" alt="After"></a>
            </figure>
          </div>
        </details>""")
    return f"""
    <section>
      <h2>{title}</h2>
      <p><strong>{verdict}.</strong> Geometry checked at {comparison['rows_checked']} rows;
        {len(changed)} changed. All measurements are in points.</p>
      <pre>Before: {escape(json.dumps(comparison['whole_tail_before']))}
After:  {escape(json.dumps(comparison['whole_tail_after']))}</pre>
      {''.join(pictures)}
    </section>"""


def write_report(directory, comparisons):
    sizes = json.loads((directory / 'all-cases.json').read_text(encoding='utf-8'))
    rows = '\n'.join(
        f"<tr><td>{escape(case['workbook'])}</td><td>{case['controls']['bytes']:,}</td>"
        f"<td>{case['default']['bytes']:,}</td><td>{case['maximum']['bytes']:,}</td></tr>"
        for case in sizes
    )
    page = f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>EUSES empty font-cell investigation</title>
  <style>
    body {{ max-width: 1200px; margin: 32px auto; padding: 0 24px; font: 16px/1.5 system-ui; }}
    table {{ width: 100%; border-collapse: collapse; }}
    td, th {{ text-align: left; padding: 8px; border-bottom: 1px solid #ccc; }}
    section {{ margin: 32px 0; padding-top: 12px; border-top: 2px solid #bbb; }}
    details {{ padding: 8px 0; }}
    summary {{ cursor: pointer; }}
    .images {{ display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }}
    figure {{ margin: 8px 0; min-width: 0; }}
    img {{ max-width: 100%; height: auto; border: 1px solid #ccc; }}
    pre {{ overflow-x: auto; }}
  </style>
</head>
<body>
  <h1>EUSES empty font-cell investigation</h1>
  <p>The real 23d7c3ac case matches in the measured regions. Controlled font-height
    and fixed-height changes expose unsafe tail removal in the current Default.
    The Electricity sentinel experiment preserves every measured row position
    and ten sampled images while removing 60,646 empty cells.</p>
  <p>These are selected cases and bounded visual checks, not a full visual
    certification of all twelve workbooks. The production shrinker was not changed.</p>
  <h2>Fresh build: twelve supplied EUSES cases (bytes)</h2>
  <table>
    <thead><tr><th>Workbook</th><th>Excel-resaved control</th><th>Default</th><th>Maximum</th></tr></thead>
    <tbody>{rows}</tbody>
  </table>
  {''.join(render_comparison(comparison) for comparison in comparisons)}
</body>
</html>
"""
    (directory / 'report.html').write_text(page, encoding='utf-8')
    (directory / 'measured-comparisons.json').write_text(
        json.dumps(comparisons, indent=2) + '\n', encoding='utf-8'
    )


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    arguments = parser.parse_args()
    output = arguments.directory.resolve()
    write_report(output, collect_comparisons(output))
