"""Create embedded and standalone charts for a same-process access experiment."""
import sys
from pathlib import Path

from openpyxl import Workbook
from openpyxl.chart import BarChart, Reference


def create(path: Path) -> None:
    book = Workbook()
    data = book.active
    data.title = 'Data'
    for value in (1, 3, 2):
        data.append([value])
    for name in ('Standalone', 'Different'):
        chart = BarChart()
        chart.title = name
        chart.add_data(Reference(data, min_col=1, min_row=1, max_row=3))
        sheet = book.create_chartsheet(name)
        sheet.add_chart(chart)
    embedded = BarChart()
    embedded.title = 'Embedded'
    embedded.add_data(Reference(data, min_col=1, min_row=1, max_row=3))
    data.add_chart(embedded, 'D1')
    book.active = data
    path.parent.mkdir(parents=True, exist_ok=True)
    book.save(path)


if __name__ == '__main__':
    create(Path(sys.argv[1]))
