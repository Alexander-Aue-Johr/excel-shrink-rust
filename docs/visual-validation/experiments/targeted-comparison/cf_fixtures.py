"""Build CF controls for input proofs, missing targets, and required fallbacks."""
from pathlib import Path


def create(directory: Path, mode: str) -> None:
    from openpyxl import Workbook
    from openpyxl.formatting.rule import CellIsRule, ColorScaleRule, Rule
    from openpyxl.styles.differential import DifferentialStyle
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
        if mode.startswith('ref-'):
            reference = {'ref-relative': 'A1', 'ref-column': '$A1', 'ref-row': 'A$1', 'ref-fixed': '$A$1', 'ref-disjoint': 'A1'}[mode]
            sheet['A1'] = None if lane == 'original' else 1
            for row in range(2, 6):
                for column in range(2, 5):
                    sheet.cell(row, column, 1)
            area = 'B2:D5'
            if mode == 'ref-disjoint':
                area = 'B2:C3 F5:G6'
                sheet['G6'] = 1
            rule = Rule(type='expression', formula=[f'LEN(TRIM({reference}))=0'], dxf=DifferentialStyle(fill=fill))
            sheet.conditional_formatting.add(area, rule)
        elif mode == 'duplicate':
            sheet.conditional_formatting.add('A1:B5', Rule(type='duplicateValues', dxf=DifferentialStyle(fill=fill)))
            sheet['A1'] = 2
            sheet['A3'] = 2 if lane == 'original' else 3
        elif mode == 'color-scale':
            for row in range(1, 6):
                sheet.cell(row, 1, row)
            sheet.conditional_formatting.add('A1:B5', ColorScaleRule(start_type='min', start_color='FFFF0000', end_type='max', end_color='FF00FF00'))
            if lane == 'default':
                sheet['A3'] = 10
        elif not (mode == 'dummy-inert' and lane == 'default'):
            sheet.conditional_formatting.add('A1:B5', CellIsRule(operator=operator, formula=[formula], fill=fill))
        if mode == 'constant' and lane == 'default':
            sheet['A3'] = -1
        if mode == 'dummy-present' or (mode == 'dummy-added' and lane == 'default'):
            target = book.create_sheet('Dummy_for_Comparison1')
            target.sheet_state = 'hidden'
            for row in range(1, 6):
                target.append([1 if lane == 'default' and mode == 'dummy-present' else 2] * 2)
        book.save(directory / f'{lane}.xlsx')
