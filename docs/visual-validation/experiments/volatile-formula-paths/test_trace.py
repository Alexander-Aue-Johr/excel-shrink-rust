"""Behavioral checks for the intentionally limited dependency prototype."""
import unittest
from trace import Cell, find_path, references, shifted_formula, volatile_span


class FormulaPaths(unittest.TestCase):
    def test_highlight_covers_only_the_nested_volatile_call(self):
        formula = 'IF(A1>RANDBETWEEN(1,SUM(2,3)),"RAND()",0)'
        start, end = volatile_span(formula)
        self.assertEqual(formula[start:end], 'RANDBETWEEN(1,SUM(2,3))')
        self.assertIsNone(volatile_span('IF(A1=1,"RAND()",0)'))

    def test_direct_absolute_and_quoted_sheet_references(self):
        self.assertEqual(references("$A$1+'Other Sheet'!B2+'O''Brien'!$C4", 'Here'),
                         [('Here', 'A1'), ('Other Sheet', 'B2'), ("O'Brien", 'C4')])
        self.assertEqual(references('IF(A1="B2",3,4)', 'Here'), [('Here', 'A1')])

    def test_shared_translation_preserves_absolute_coordinates_and_strings(self):
        self.assertEqual(shifted_formula('A1+$B2+C$3+$D$4+IF(A1="A1",1,0)', 'E5', 'F7'),
                         'B3+$B4+D$3+$D$4+IF(B3="A1",1,0)')

    def test_cycles_and_irrelevant_branches_do_not_hide_a_supported_path(self):
        cells = {('S', 'A1'): Cell('B1+C1', '1'), ('S', 'B1'): Cell('A1', '2'),
                 ('S', 'C1'): Cell("'Other'!D1", '3'), ('Other', 'D1'): Cell('RAND()', '.5')}
        result = find_path(cells, ('S', 'A1'))
        self.assertEqual([node['cell'] for node in result['path']], ['A1', 'C1', 'D1'])
        self.assertEqual(result['path'][-1]['volatile_span'], [0, 6])

    def test_dynamic_and_range_references_remain_explicitly_unresolved(self):
        for formula in ('INDIRECT(A1)', 'OFFSET(A1,1,0)', '[other.xlsx]S!A1'):
            with self.subTest(formula=formula):
                result = find_path({('S', 'A1'): Cell(formula, '1')}, ('S', 'A1'))
                self.assertEqual(result['status'], 'not_resolved')

    def test_range_visits_formula_cells_without_expanding_the_full_sheet(self):
        cells = {('S', 'A1'): Cell('SUM(B1:B1048576)', '1'),
                 ('S', 'B999999'): Cell('RAND()', '.5')}
        path = find_path(cells, ('S', 'A1'))['path']
        self.assertEqual([node['cell'] for node in path], ['A1', 'B999999'])

    def test_only_unambiguous_literal_indirect_is_followed(self):
        cells = {('S', 'A1'): Cell('INDIRECT("B2",TRUE)', '1'), ('S', 'B2'): Cell('TODAY()', '40000')}
        self.assertEqual(find_path(cells, ('S', 'A1'))['status'], 'path_found')
        self.assertEqual(references('IF(A1="INDIRECT(""B2"")",1,2)', 'S'), [('S', 'A1')])
        self.assertEqual(find_path({('S', 'A1'): Cell('INDIRECT("B"&2)', '1')}, ('S', 'A1'))['status'], 'not_resolved')


if __name__ == '__main__':
    unittest.main()
