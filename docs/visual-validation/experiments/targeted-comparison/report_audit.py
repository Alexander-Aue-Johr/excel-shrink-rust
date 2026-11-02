"""Load the tiny smoke-test audits; production readers should stream large sidecars."""
import gzip
import json


def load_report(directory):
    report = json.loads((directory / 'report.json').read_text())
    for transition in report['transitions']:
        if 'changes' in transition:
            continue
        with gzip.open(transition['changes_file'], 'rt', encoding='utf-8') as stream:
            header = json.loads(next(stream))
            changes = []
            for line in stream:
                values = json.loads(line)
                change = dict(zip(header['columns'], values))
                for field in ['context_before', 'context_after']:
                    change[field] = header['contexts'][change[field]]
                changes.append(change)
            assert len(changes) == transition['change_count']
            transition['changes'] = changes
    return report
