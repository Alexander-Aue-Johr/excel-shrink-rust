"""Resolve focused, exact workbook/worksheet selections before Excel is launched.

Only xl/workbook.xml is read for the sheet catalogue. This is not a second data,
UsedRange, drawing or geometry scan. No workbook bytes are changed.
"""
from __future__ import annotations
import copy
import hashlib
import json
import re
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

PARAMETERS = {
    'host_kind': 'dotnet', 'overflow_mode': 'selective', 'tile_megapixels': 16,
    'context_validation': 'off', 'diagnostic_level': 'detailed',
    'setup_timeout_seconds': 600, 'startup_timeout_seconds': 300,
    'capture_timeout_seconds': 1200, 'inactivity_timeout_seconds': 120,
    'native_call_timeout_seconds': 90, 'postprocess_timeout_seconds': 240,
    'repetitions': 1,
}
INTEGER_LIMITS = {
    'setup_timeout_seconds': (30, 1800), 'startup_timeout_seconds': (10, 1800),
    'capture_timeout_seconds': (30, 7200), 'inactivity_timeout_seconds': (10, 1800),
    'native_call_timeout_seconds': (10, 600), 'postprocess_timeout_seconds': (30, 1800),
    'session_timeout_seconds': (60, 86400), 'repetitions': (1, 5),
}
CHOICES = {
    'host_kind': ('rust', 'dotnet'), 'overflow_mode': ('heuristic', 'selective'),
    'tile_megapixels': (16, 32, 64), 'context_validation': ('off', 'first-seam'),
    'diagnostic_level': ('normal', 'detailed'),
}
SELECTION_KEYS = {'case_ids', 'workbook', 'sheets', 'sheet_indexes', 'targets', 'corpus_root'}
REQUEST_KEYS = SELECTION_KEYS | set(PARAMETERS) | {'schema', 'profile', 'session_timeout_seconds', 'wait_for_start', 'use_office_debug_js'}


def catalogue(repo: Path) -> dict:
    path = repo/'docs/visual-validation/experiments/capture-focus-cases.json'
    data = json.loads(path.read_text(encoding='utf-8-sig'))
    ids = [c['id'] for c in data['cases']]
    if len(ids) != len(set(ids)):
        raise ValueError('Duplicate focused case identity')
    return data


def sheet_catalogue(path: Path) -> list[dict]:
    with zipfile.ZipFile(path) as archive:
        name = 'xl/workbook.xml'
        if archive.namelist().count(name) != 1:
            raise ValueError('Expected exactly one xl/workbook.xml: '+str(path))
        info = archive.getinfo(name)
        if info.file_size > 4 * 1024 * 1024:
            raise ValueError('Workbook catalogue exceeds the 4-MiB metadata budget: '+str(path))
        data = archive.read(info)
    declaration_probe=data.upper().replace(b'\x00',b'')
    if b'<!DOCTYPE' in declaration_probe or b'<!ENTITY' in declaration_probe:
        raise ValueError('DTD/entity declarations are not accepted in workbook metadata')
    root = ET.fromstring(data)
    nodes = root.findall('./{*}sheets/{*}sheet')
    if not nodes:
        raise ValueError('Workbook has no sheet catalogue: '+str(path))
    result = []
    for index, node in enumerate(nodes, 1):
        name = node.get('name')
        if not name:
            raise ValueError('Unnamed worksheet in '+str(path))
        result.append({'index': index, 'name': name, 'state': node.get('state', 'visible'),
                       'sheet_id': node.get('sheetId')})
    if len({x['name'] for x in result}) != len(result):
        raise ValueError('Duplicate exact sheet names in '+str(path))
    return result


def _array(value, name, kind=str, limit=512):
    if not isinstance(value, list) or not value or len(value) > limit:
        raise ValueError(name+' must be a nonempty list (maximum '+str(limit)+')')
    if any(type(x) is not kind or (kind is str and not x) for x in value):
        raise ValueError('Invalid '+name)
    if len(set(value)) != len(value):
        raise ValueError('Duplicate '+name)
    return value


def validate_request(request: dict) -> dict:
    if not isinstance(request, dict):
        raise ValueError('A focused request must be a JSON object')
    if set(request) - REQUEST_KEYS:
        raise ValueError('Unknown focused request fields: '+str(sorted(set(request)-REQUEST_KEYS)))
    if request.get('schema', 'vv-focus-request-v1') != 'vv-focus-request-v1':
        raise ValueError('Unsupported focused request schema')
    if 'profile' in request:
        if not isinstance(request['profile'], str) or not request['profile']:
            raise ValueError('profile must be a named question profile')
        if any(k in request for k in ('case_ids','workbook','targets','sheets','sheet_indexes')):
            raise ValueError('Choose a question profile OR an explicit selection, not both')
    for key, (lo, hi) in INTEGER_LIMITS.items():
        if key in request and (type(request[key]) is not int or not lo <= request[key] <= hi):
            raise ValueError(f'{key} must be an integer in {lo}..{hi}')
    for key, values in CHOICES.items():
        if key in request and (request[key] not in values or isinstance(request[key], bool)):
            raise ValueError(key+' must be one of '+str(values))
    for key in ('wait_for_start', 'use_office_debug_js'):
        if key in request and type(request[key]) is not bool:
            raise ValueError(key+' must be boolean')
    if 'case_ids' in request:
        _array(request['case_ids'], 'case_ids', limit=32)
    if 'workbook' in request and (not isinstance(request['workbook'], str) or not request['workbook']):
        raise ValueError('workbook must be an exact path or unambiguous filename')
    for key in ('sheets', 'sheet_indexes'):
        if key in request and request[key] is not None:
            _array(request[key], key, int if key == 'sheet_indexes' else str)
    if request.get('sheets') and request.get('sheet_indexes'):
        raise ValueError('Choose exact sheet names OR 1-based indexes, not both')
    if request.get('case_ids') and (request.get('workbook') or request.get('targets')):
        raise ValueError('Choose named cases OR an explicit workbook/targets selection')
    if request.get('workbook') and request.get('targets'):
        raise ValueError('Choose workbook OR targets')
    if 'targets' in request:
        if request.get('sheets') is not None or request.get('sheet_indexes') is not None:
            raise ValueError('Put worksheet selectors inside each target, not at the request root')
        if not isinstance(request['targets'], list) or not 1 <= len(request['targets']) <= 32:
            raise ValueError('targets must contain 1..32 selections')
        for t in request['targets']:
            if not isinstance(t, dict) or set(t)-{'id','workbook','sheets','sheet_indexes','question'}:
                raise ValueError('Invalid target fields')
            validate_request({k:v for k,v in t.items() if k not in {'id','question'}})
            if not t.get('workbook'):
                raise ValueError('Each target requires a workbook')
    return request


def _relative_workbook(value: str, files: list[dict], corpus: Path) -> dict:
    normalized = value.replace('\\', '/')
    # Names are selectors, never shell code. No wildcard or implicit "all" expansion.
    if normalized in ('*', 'all') or any(x in normalized for x in ('*','?')):
        raise ValueError('Focused workbooks must be explicit, not all/*')
    exact = [f for f in files if f['relative_path'] == normalized or str(Path(f['source']).resolve()).replace('\\','/') == normalized]
    if not exact and '/' not in normalized:
        exact = [f for f in files if f['name'] == normalized]
    if len(exact) != 1:
        raise ValueError('Workbook selector must identify exactly one file: '+value+'; matched '+str(len(exact)))
    return exact[0]


def _selected_sheets(catalog: list[dict], target: dict) -> list[str] | None:
    requested = target.get('sheets')
    indexes = target.get('sheet_indexes')
    if requested is not None and indexes is not None:
        raise ValueError('Choose sheet names or indexes for a target, not both')
    if indexes is not None:
        _array(indexes, 'sheet_indexes', int)
        if any(i < 1 or i > len(catalog) for i in indexes):
            raise ValueError('Sheet index is outside the actual workbook catalogue')
        requested = [catalog[i-1]['name'] for i in indexes]
    if requested is None:
        return None
    _array(requested, 'sheets')
    by_name = {s['name']:s for s in catalog}
    missing = [s for s in requested if s not in by_name]
    if missing:
        raise ValueError('Unknown exact sheet name(s): '+json.dumps(missing, ensure_ascii=False)+
                         '. Use -ListSheets. Leading/trailing spaces are significant.')
    hidden = [s for s in requested if by_name[s]['state'] != 'visible']
    if hidden:
        raise ValueError('Hidden worksheets will not be unhidden: '+json.dumps(hidden, ensure_ascii=False))
    # Capture follows workbook order; exact names, including whitespace, remain identities.
    return [s['name'] for s in catalog if s['name'] in requested]


def resolve(repo: Path, cfg: dict, request: dict, inventory=None) -> dict:
    import campaign as C
    request = copy.deepcopy(validate_request(request))
    out = copy.deepcopy(cfg)
    root = request.get('corpus_root', cfg['corpus_root'])
    if not isinstance(root, str) or not root:
        raise ValueError('Invalid corpus root')
    p = Path(root)
    if not p.is_absolute() and not re.match(r'^[A-Za-z]:|^\\\\',root):
        p = repo/p
    elif C.os.name != 'nt' and re.match(r'^[A-Za-z]:|^\\\\',root):
        p = C.local_path(root)
    p = p.resolve()
    out['corpus_root'] = str(p)
    all_cfg = dict(out, workbooks=['*'])
    files = inventory if inventory is not None else C.discover(repo, all_cfg)
    cats = catalogue(repo)
    known = {x['id']:x for x in cats['cases']}
    question = None
    if request.get('profile'):
        from focus_profiles import load_profile
        question = load_profile(repo, request['profile'])
        varied = set().union(*(v.keys() for v in question['variants']))
        if varied.intersection(request):
            raise ValueError('This profile varies '+', '.join(sorted(varied))+'. Do not override the varied field; all other parameters remain available.')
        request = dict(question['defaults'], **request)
        request.setdefault('session_timeout_seconds', question['session_seconds'])
        targets = copy.deepcopy(question['targets'])
    elif request.get('workbook'):
        targets = [{k:request[k] for k in ('workbook','sheets','sheet_indexes') if k in request}]
    elif request.get('targets'):
        targets = copy.deepcopy(request['targets'])
    else:
        ids = request.get('case_ids') or [cats['default_case']]
        missing = [i for i in ids if i not in known]
        if missing:
            raise ValueError('Unknown case ID(s): '+', '.join(missing)+'. Use -ListCases.')
        targets = [copy.deepcopy(known[i]) for i in ids]
        if request.get('sheets') or request.get('sheet_indexes'):
            if len(targets) != 1:
                raise ValueError('A sheet override needs exactly one selected case')
            targets[0].pop('sheets', None); targets[0].pop('sheet_indexes', None)
            for k in ('sheets','sheet_indexes'):
                if k in request: targets[0][k] = request[k]
    settings = dict(PARAMETERS)
    settings.update({k:v for k,v in request.items() if k in PARAMETERS})
    # The named long-sheet case gets a larger capture allowance unless explicitly overridden.
    if 'capture_timeout_seconds' not in request and any(t.get('id') == 'kernhaushalt-long' for t in targets):
        settings['capture_timeout_seconds'] = 1800
    if settings['inactivity_timeout_seconds'] < settings['native_call_timeout_seconds'] + 10:
        raise ValueError('Inactivity timeout must exceed native-call timeout by at least 10 seconds')
    if settings['capture_timeout_seconds'] < settings['native_call_timeout_seconds']:
        raise ValueError('Capture timeout must not be shorter than a native-call timeout')
    workload = settings['setup_timeout_seconds'] + settings['startup_timeout_seconds'] + settings['capture_timeout_seconds'] + 30
    session = request.get('session_timeout_seconds', max(3600, min(86400, len(targets)*settings['repetitions']*(workload+settings['postprocess_timeout_seconds']+30))))
    if session < workload+settings['postprocess_timeout_seconds']+30:
        raise ValueError('Session budget is shorter than one complete configured case. Increase -SessionTimeoutSeconds; no hidden six-minute cap will be used.')
    identities = set(); resolved = []; catalogues = {}; workbooks = []
    for target in targets:
        selector = target['workbook']
        if C.os.name != 'nt' and re.match(r'^[A-Za-z]:|^\\\\', selector):
            selector = str(C.local_path(selector))
        f = _relative_workbook(selector, files, p)
        if f['relative_path'] not in catalogues:
            catalogues[f['relative_path']] = sheet_catalogue(Path(f['source']))
        if 'after_sheet' in target:
            catalog = catalogues[f['relative_path']]
            anchors = [x for x in catalog if x['name'] == target['after_sheet']]
            if len(anchors) != 1: raise ValueError('Missing exact coverage anchor: '+target['after_sheet'])
            selected = [x['name'] for x in catalog if x['index'] > anchors[0]['index'] and x['state'] == 'visible']
            if not selected: raise ValueError('No visible worksheets after coverage anchor; nothing will be silently tested instead.')
            target = dict(target, sheets=selected)
        sheets = _selected_sheets(catalogues[f['relative_path']], target)
        ident = target.get('id') or 'custom-'+hashlib.sha256(json.dumps([f['relative_path'],sheets],ensure_ascii=False).encode()).hexdigest()[:10]
        if not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,63}',ident) or ident in identities:
            raise ValueError('Focused target IDs must be unique lowercase slugs')
        identities.add(ident)
        item = {'id':ident, 'workbook':f['relative_path'], 'sheets':sheets,
                'question':target.get('question','Selected-sheet reproduction; no strategy recommendation.'),
                'sheet_catalog':catalogues[f['relative_path']]}
        if target.get('expected_rows_note'):
            item['expected_rows_note'] = target['expected_rows_note']
        resolved.append(item)
        if f['relative_path'] not in workbooks: workbooks.append(f['relative_path'])
    out['workbooks'] = workbooks
    out['focused_targets'] = resolved
    if question:
        out['focus_comparison'] = dict(question)
        out['focus_comparison']['targets'] = [{k:v for k,v in t.items() if k!='sheet_catalog'} for t in resolved]
    else:
        out.pop('focus_comparison', None)
    out['focus_parameters'] = dict(settings, session_timeout_seconds=session,
        workload_timeout_seconds=workload, wait_for_start=request.get('wait_for_start',False),
        use_office_debug_js=request.get('use_office_debug_js',False))
    out['focus_request'] = dict(request, schema='vv-focus-request-v1')
    out['focus_protocol'] = 'exact-scope-phase-timeouts-v1'
    out['focus_selection_hash'] = C.obj_digest([resolved, out['focus_parameters'], str(p)])
    out['limits'].update(session_seconds=session, case_min_seconds=30,
        case_default_seconds=workload, case_max_seconds=workload,
        postprocess_seconds=settings['postprocess_timeout_seconds'])
    out['diagnostics'] = []
    for k in out['references']:
        out['references'][k] = [] if k == 'bounds_strategies' else False
    out['capture']['excel_launch_mode']='excel-x'
    out['capture']['batch_size']=1
    out['capture']['tile_packing']='budget'
    out['auto_resume']=False  # explicit -ResumeRun; repeating a failed repro is never silently skipped
    C.validate(out)
    return out


def plan(cfg):
    p = cfg.get('focus_parameters', {})
    return {'profile':'focused-capture', 'protocol':cfg.get('focus_protocol'),
            'targets':[{k:v for k,v in t.items() if k!='sheet_catalog'} for t in cfg['focused_targets']], 'parameters':p,
            'scope':'Selected worksheets only; no production winner or implicit corpus sweep. See the explicit profile recovery policy.',
            'case_count':len(cfg['focused_targets'])*p.get('repetitions',1)*len(cfg.get('focus_comparison',{}).get('variants',[{}])),
            'question_profile':cfg.get('focus_comparison'),
            'extra_attempt_policy':'At most one same-scope fresh 16-MP attempt per unsuccessful large-budget primary; actual cost retained.' if cfg.get('focus_comparison',{}).get('recovery_to_16') else 'No automatic recovery sweep.',
            'session_budget_seconds':cfg['limits']['session_seconds']}
