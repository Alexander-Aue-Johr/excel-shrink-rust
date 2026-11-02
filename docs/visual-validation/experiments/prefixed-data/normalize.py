"""Experimental namespace-only copies; never edit corpus originals or count this as shrink savings."""
import argparse
import hashlib
import shutil
import struct
import zipfile
from pathlib import Path
from xml.parsers import expat
from xml.sax.saxutils import escape, quoteattr

MAIN = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'


def expanded(name):
    parts = name.split('|')
    return (parts[0], parts[1], parts[2] if len(parts) == 3 else '') if len(parts) > 1 else ('', name, '')


class XmlCopy:
    """Stream expanded XML names through a semantic digest and optional prefix-normalizing writer."""
    def __init__(self, output=None):
        self.output = output
        self.buffer = bytearray()
        self.digest = hashlib.sha256()
        self.pending = []
        self.defaults = ['']
        self.names = []
        self.text_open = False
        self.rows = 0
        self.cells = 0
        self.max_row = 0
        self.parser = expat.ParserCreate(namespace_separator='|')
        self.parser.namespace_prefixes = True
        self.parser.StartNamespaceDeclHandler = lambda prefix, uri: self.pending.append((prefix or '', uri or ''))
        self.parser.StartElementHandler = self.start
        self.parser.EndElementHandler = self.end
        self.parser.CharacterDataHandler = self.text
        self.parser.CommentHandler = lambda text: self.write(f'<!--{text}-->')
        self.parser.ProcessingInstructionHandler = lambda target, data: self.write(f'<?{target} {data}?>')
        self.parser.StartDoctypeDeclHandler = self.reject_doctype

    @staticmethod
    def reject_doctype(*_):
        raise ValueError('DTD normalization is intentionally unsupported')

    def write(self, text):
        if self.output is None:
            return
        self.buffer.extend(text.encode('utf-8'))
        if len(self.buffer) >= 1024 * 1024:
            self.output.write(self.buffer)
            self.buffer.clear()

    def field(self, text):
        data = text.encode('utf-8')
        self.digest.update(struct.pack('<Q', len(data)))
        self.digest.update(data)

    def start(self, name, attributes):
        uri, local, prefix = expanded(name)
        self.digest.update(b'\x01')
        self.field(uri + '|' + local)
        for key, value in sorted((expanded(key)[:2], value) for key, value in attributes.items()):
            self.field('|'.join(key))
            self.field(value)
        self.text_open = False
        current_default = self.defaults[-1]
        declarations = []
        for ns_prefix, ns_uri in self.pending:
            if ns_uri == MAIN:
                continue
            declarations.append(('xmlns' + (':' + ns_prefix if ns_prefix else ''), ns_uri))
            if not ns_prefix:
                current_default = ns_uri
        self.pending.clear()
        output_name = local if uri == MAIN or not prefix else prefix + ':' + local
        if uri == MAIN or not prefix:
            if current_default != uri:
                declarations.append(('xmlns', uri))
                current_default = uri
        self.defaults.append(current_default)
        self.names.append(output_name)
        output_attributes = list(declarations)
        for key, value in attributes.items():
            attr_uri, attr_local, attr_prefix = expanded(key)
            if attr_uri == MAIN:
                raise ValueError('Namespace-qualified SpreadsheetML attributes need a separate experiment')
            output_attributes.append(((attr_prefix + ':' if attr_prefix else '') + attr_local, value))
        self.write('<' + output_name + ''.join(' ' + key + '=' + quoteattr(value) for key, value in output_attributes) + '>')
        if uri == MAIN and local == 'row':
            self.rows += 1
            self.max_row = max(self.max_row, int(attributes.get('r', '0')))
        if uri == MAIN and local == 'c':
            self.cells += 1

    def end(self, name):
        self.digest.update(b'\x03')
        uri, local, _ = expanded(name)
        self.field(uri + '|' + local)
        self.text_open = False
        self.write('</' + self.names.pop() + '>')
        self.defaults.pop()

    def text(self, text):
        if not text:
            return
        if not self.text_open:
            self.digest.update(b'\x02')
            self.text_open = True
        self.digest.update(text.encode('utf-8'))
        self.write(escape(text))

    def read(self, source):
        while data := source.read(1024 * 1024):
            self.parser.Parse(data, False)
        self.parser.Parse(b'', True)
        if self.buffer:
            self.output.write(self.buffer)
            self.buffer.clear()
        return {'semantic_sha256': self.digest.hexdigest(), 'rows': self.rows, 'cells': self.cells, 'max_row': self.max_row}


def normalize(source: Path, target: Path):
    if target.exists():
        raise FileExistsError(target)
    target.parent.mkdir(parents=True, exist_ok=True)
    evidence = {}
    with zipfile.ZipFile(source) as original, zipfile.ZipFile(target, 'x', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as output:
        for entry in original.infolist():
            with original.open(entry) as incoming, output.open(entry.filename, 'w', force_zip64=True) as outgoing:
                if entry.filename.startswith('xl/') and entry.filename.endswith('.xml'):
                    evidence[entry.filename] = XmlCopy(outgoing).read(incoming)
                else:
                    shutil.copyfileobj(incoming, outgoing, 1024 * 1024)
    with zipfile.ZipFile(target) as result:
        for name, expected in evidence.items():
            with result.open(name) as entry:
                actual = XmlCopy().read(entry)
            if actual != expected:
                raise ValueError(f'Namespace normalization changed XML semantics: {name}')
    return evidence


if __name__ == '__main__':
    import json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('target', type=Path)
    args = parser.parse_args()
    result = normalize(args.source, args.target)
    print(json.dumps(result, indent=2))
