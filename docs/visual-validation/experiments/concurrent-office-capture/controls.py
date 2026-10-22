"""An obvious text change inside the selected range, independent of shrink behavior."""
import posixpath
import re
import xml.etree.ElementTree as ET
from zipfile import ZipFile, ZIP_DEFLATED


def changed_workbook(source, destination, sheet_index):
    with ZipFile(source) as archive:
        book = ET.fromstring(archive.read("xl/workbook.xml"))
        sheet = list(book.find("{*}sheets"))[sheet_index - 1]
        relation = next(v for k, v in sheet.attrib.items() if k.endswith("}id"))
        relations = ET.fromstring(archive.read("xl/_rels/workbook.xml.rels"))
        target = next(item.get("Target") for item in relations if item.get("Id") == relation)
        target = posixpath.normpath(posixpath.join("xl", target)).lstrip("/")
        # Preserve the sheet's namespace declarations and mc:Ignorable prefixes.
        # Re-serializing the entire XML can provoke Excel's repair dialog.
        source_xml = archive.read(target).decode("utf-8")
        pattern = re.compile(r'<c\b(?=[^>]*\br="A1")(.*?)(?:/>|>.*?</c>)', re.S)
        match = pattern.search(source_xml)
        if not match:
            raise ValueError("Positive control requires an existing A1 cell")
        attributes = re.sub(r'\s+t="[^"]*"', "", match[1])
        replacement = '<c' + attributes + ' t="inlineStr"><is><t>EXPERIMENT CONTROL</t></is></c>'
        changed = source_xml[:match.start()] + replacement + source_xml[match.end():]
        with ZipFile(destination, "w", ZIP_DEFLATED) as output:
            for entry in archive.infolist():
                output.writestr(entry, changed.encode("utf-8")
                                if entry.filename == target else archive.read(entry.filename))
