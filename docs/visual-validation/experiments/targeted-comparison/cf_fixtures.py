"""Build CF controls for input proofs, missing targets, and required fallbacks."""
from pathlib import Path


def create(directory: Path, mode: str) -> None:
    from openpyxl import Workbook
    from openpyxl.formatting.rule import CellIsRule
    from openpyxl.styles import PatternFill

    directory.mkdir(parents=True, exist_ok=False)
    for lane in ('original', 'default'):
        book = Workbook()
        sheet = book.active
        sheet.title = 'CF'
        for row in range(1, 6):
            sheet.append([1, 1])
        sheet.column_dimensions['A'].width = 18
        sheet.column_dimensions['B'].width = 18
        color = 'FF0000FF' if mode == 'styles' and lane == 'default' else 'FFFF0000'
        # Differential fills need an explicit background on this Excel renderer.
        fill = PatternFill('solid', fgColor=color, bgColor=color)
        dummy = mode.startswith('dummy-')
        formula = 'INDIRECT("Dummy_for_Comparison1!"&ADDRESS(ROW(),COLUMN()))' if dummy else '0'
        operator = 'notEqual' if dummy else 'greaterThan'
        if mode == 'constant-string':
            operator = 'equal'
            formula = '""" """'
            sheet['A1'] = '" "'
            sheet['B1'] = 'other'
            sheet['A3'] = '" "' if lane == 'original' else 'other'
        if not (mode == 'dummy-inert' and lane == 'default'):
            sheet.conditional_formatting.add('A1:B5', CellIsRule(operator=operator, formula=[formula], fill=fill))
        if mode == 'constant' and lane == 'default':
            sheet['A3'] = -1
        if mode == 'dummy-present' or (mode == 'dummy-added' and lane == 'default'):
            target = book.create_sheet('Dummy_for_Comparison1')
            target.sheet_state = 'hidden'
            for row in range(1, 6):
                target.append([1 if lane == 'default' and mode == 'dummy-present' else 2] * 2)
        book.save(directory / f'{lane}.xlsx')
