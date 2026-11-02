"""Build CF controls for input proofs, missing targets, and required fallbacks."""
from pathlib import Path


def create(directory: Path, mode: str) -> None:
    from openpyxl import Workbook
    from openpyxl.formatting.rule import CellIsRule, ColorScaleRule, DataBarRule, Rule
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
            reference = {
                'ref-column': '$A1', 'ref-row': 'A$1', 'ref-fixed': '$A$1',
            }.get(mode, 'A1')
            sheet['A1'] = None if lane == 'original' else 1
            for row in range(2, 6):
                for column in range(2, 5):
                    sheet.cell(row, column, 1)
            area = 'B2:D5'
            if mode == 'ref-disjoint':
                area = 'B2:C3 F5:G6'
                sheet['G6'] = 1
            formula = f'LEN(TRIM({reference}))=0'
            if mode == 'ref-search':
                sheet['A1'] = 2 if lane == 'original' else 1
                formula = 'NOT(ISERROR(SEARCH("1",A1)))'
            elif mode == 'ref-multiple':
                sheet['A1'] = 1
                sheet['F1'] = 0 if lane == 'original' else 1
                formula = 'AND(A1=1,$F$1=1)'
            elif mode == 'ref-cell-is':
                sheet['A1'] = 1
                sheet['F1'] = 2 if lane == 'original' else 0
            elif mode == 'ref-range':
                formula = 'SUM(A1:A2)>1'
            elif mode == 'ref-range-fixed':
                formula = 'COUNTIF($A$1:$A$2,1)>1'
            elif mode == 'ref-range-growing':
                formula = 'SUM($A$1:A1)>0'
            elif mode == 'ref-mixed':
                formula = 'SUM($A$1:$A$2)>1'
            elif mode == 'ref-mixed-group':
                sheet['A1'] = 1
                sheet['B3'] = 1 if lane == 'original' else 10
            elif mode.startswith('ref-cross-'):
                sheet['A1'] = 1
                name = "O'Brien data" if mode == 'ref-cross-quoted' else 'Other'
                source = book.create_sheet(name)
                source['A1'] = 0 if lane == 'original' else 1
                source['A2'] = 1
                qualifier = "'O''Brien data'" if mode == 'ref-cross-quoted' else 'Other'
                if mode == 'ref-cross-range':
                    formula = f'SUM({qualifier}!$A$1:$A$2)>1'
                elif mode == 'ref-cross-quoted':
                    formula = f'{qualifier}!$A$1=1'
                else:
                    formula = f'{qualifier}!A1=1'
            if mode == 'ref-cell-is':
                rule = CellIsRule(operator='greaterThan', formula=['$F$1'], fill=fill)
            else:
                rule = Rule(type='expression', formula=[formula], dxf=DifferentialStyle(fill=fill))
            sheet.conditional_formatting.add(area, rule)
            if mode in ('ref-mixed', 'ref-mixed-group'):
                sheet.conditional_formatting.add(area, ColorScaleRule(start_type='min', start_color='FFFF0000', end_type='max', end_color='FF00FF00'))
        elif mode in ('row-parity', 'row-parity-shift'):
            for column, parity in [('A', 0), ('B', 1)]:
                if mode == 'row-parity-shift' and lane == 'default':
                    parity = 1 - parity
                sheet.conditional_formatting.add(
                    f'{column}1:{column}5', Rule(type='expression', formula=[f'MOD(ROW(),2)={parity}'], dxf=DifferentialStyle(fill=fill)),
                )
                if mode == 'row-parity' and lane == 'default':
                    sheet[f'{column}3'] = 9
        elif mode in ('top-bottom', 'top-bottom-unchanged'):
            for column, bottom, percent in [('A', False, False), ('B', True, False), ('C', False, True), ('D', True, True)]:
                for row in range(1, 6):
                    sheet[f'{column}{row}'] = row
                sheet.conditional_formatting.add(
                    f'{column}1:{column}5', Rule(type='top10', rank=20 if percent else 1, bottom=bottom, percent=percent, dxf=DifferentialStyle(fill=fill)),
                )
                if mode == 'top-bottom' and lane == 'default':
                    sheet[f'{column}3'] = -10 if bottom else 10
        elif mode == 'duplicate':
            sheet.conditional_formatting.add('A1:B5', Rule(type='duplicateValues', dxf=DifferentialStyle(fill=fill)))
            sheet['A1'] = 2
            sheet['A3'] = 2 if lane == 'original' else 3
        elif mode.startswith('cell-is-ref-'):
            operators = ('equal', 'notEqual', 'lessThan', 'lessThanOrEqual', 'greaterThan', 'greaterThanOrEqual')
            for row, (operator, value) in enumerate(zip(operators, (0, -1, -1, 0, 1, 1)), 1):
                sheet.cell(row, 1, value)
                sheet.cell(row, 2, 'text')
                if not (mode == 'cell-is-ref-inert' and lane == 'default'):
                    operand = '0' if mode == 'cell-is-ref-visible' and lane == 'default' else '#REF!'
                    sheet.conditional_formatting.add(
                        f'A{row}:B{row}', CellIsRule(operator=operator, formula=[operand], stopIfTrue=True, fill=fill),
                    )
            blue = PatternFill('solid', fgColor='FF0000FF', bgColor='FF0000FF')
            sheet.conditional_formatting.add('A1:B6', Rule(type='expression', formula=['1'], dxf=DifferentialStyle(fill=blue)))
        elif mode.startswith('error-ref-'):
            formula = '#REF!=1' if mode == 'error-ref-compare-inert' else '#REF!'
            if mode == 'error-ref-visible' and lane == 'default':
                formula = '1'
            if not (mode.endswith('-inert') and lane == 'default'):
                sheet.conditional_formatting.add(
                    'A1:B5', Rule(type='expression', formula=[formula], stopIfTrue=True, dxf=DifferentialStyle(fill=fill)),
                )
            if mode.endswith('-inert'):
                # A failing stopIfTrue rule must not suppress a later, valid rule.
                blue = PatternFill('solid', fgColor='FF0000FF', bgColor='FF0000FF')
                sheet.conditional_formatting.add(
                    'A1:B5', Rule(type='expression', formula=['1'], dxf=DifferentialStyle(fill=blue)),
                )
        elif mode.startswith('name-'):
            if mode == 'name-defined':
                from openpyxl.workbook.defined_name import DefinedName
                book.defined_names.add(DefinedName('isblank', attr_text='TRUE' if lane == 'original' else 'FALSE'))
            if not (mode == 'name-inert' and lane == 'default'):
                sheet.conditional_formatting.add(
                    'A1:B5', Rule(type='expression', formula=['isblank'], dxf=DifferentialStyle(fill=fill)),
                )
        elif mode in ('data-bar', 'data-bar-unchanged'):
            for row in range(1, 6):
                sheet.cell(row, 1, row)
            sheet.conditional_formatting.add(
                'A1:B5', DataBarRule(start_type='min', end_type='max', color='FF638EC6'),
            )
            if mode == 'data-bar' and lane == 'default':
                sheet['A3'] = 10
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
