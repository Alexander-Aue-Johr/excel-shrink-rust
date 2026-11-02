//! Recognize a small pure formula grammar and collect dependencies, without evaluation.
use super::references::{parse, Reference};

const MAX_FORMULA_NESTING: usize = 64;

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
    references: Vec<Reference>,
    depth: usize,
}

/// Reject the whole formula if any token or function can hide an unknown dependency.
pub(super) fn dependencies(formula: &str) -> Option<Vec<Reference>> {
    let formula = formula.strip_prefix('=').unwrap_or(formula);
    let mut parser = Parser {
        bytes: formula.as_bytes(),
        position: 0,
        references: Vec::new(),
        depth: 0,
    };
    parser.expression()?;
    parser.whitespace();
    (parser.position == parser.bytes.len()).then_some(parser.references)
}

impl Parser<'_> {
    fn whitespace(&mut self) {
        while self
            .bytes
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }

    fn take(&mut self, byte: u8) -> bool {
        self.whitespace();
        if self.bytes.get(self.position) != Some(&byte) {
            return false;
        }
        self.position += 1;
        true
    }

    fn expression(&mut self) -> Option<()> {
        self.depth += 1;
        if self.depth > MAX_FORMULA_NESTING {
            return None;
        }
        self.atom()?;
        loop {
            self.whitespace();
            if !self
                .bytes
                .get(self.position)
                .is_some_and(|b| b"+-*/^&=<>".contains(b))
            {
                break;
            }
            let operator = self.bytes[self.position];
            self.position += 1;
            if matches!(operator, b'<' | b'>') && self.bytes.get(self.position) == Some(&b'=') {
                self.position += 1;
            } else if operator == b'<' && self.bytes.get(self.position) == Some(&b'>') {
                self.position += 1;
            }
            self.atom()?;
        }
        self.depth -= 1;
        Some(())
    }

    fn atom(&mut self) -> Option<()> {
        self.whitespace();
        // Unary signs are scanned iteratively to avoid recursion on malformed input.
        while self
            .bytes
            .get(self.position)
            .is_some_and(|b| matches!(b, b'+' | b'-'))
        {
            self.position += 1;
            self.whitespace();
        }
        match *self.bytes.get(self.position)? {
            b'\'' => self.quoted_sheet()?,
            b'"' => self.string()?,
            b'(' => {
                self.position += 1;
                self.expression()?;
                if !self.take(b')') {
                    return None;
                }
            }
            b'0'..=b'9' | b'.' => self.number()?,
            b'A'..=b'Z' | b'a'..=b'z' | b'$' => self.identifier()?,
            _ => return None,
        }
        while self.take(b'%') {}
        Some(())
    }

    fn string(&mut self) -> Option<()> {
        self.position += 1;
        while let Some(&byte) = self.bytes.get(self.position) {
            self.position += 1;
            if byte == b'"' {
                if self.bytes.get(self.position) != Some(&b'"') {
                    return Some(());
                }
                self.position += 1;
            }
        }
        None
    }

    fn number(&mut self) -> Option<()> {
        let start = self.position;
        while self
            .bytes
            .get(self.position)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
        {
            self.position += 1;
        }
        if self
            .bytes
            .get(self.position)
            .is_some_and(|b| matches!(b, b'e' | b'E'))
        {
            self.position += 1;
            if self
                .bytes
                .get(self.position)
                .is_some_and(|b| matches!(b, b'+' | b'-'))
            {
                self.position += 1;
            }
            while self
                .bytes
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
        }
        let value: f64 = std::str::from_utf8(&self.bytes[start..self.position])
            .ok()?
            .parse()
            .ok()?;
        value.is_finite().then_some(())
    }

    fn identifier(&mut self) -> Option<()> {
        let start = self.position;
        while self
            .bytes
            .get(self.position)
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'$' | b'_' | b'.'))
        {
            self.position += 1;
        }
        let name = std::str::from_utf8(&self.bytes[start..self.position]).ok()?;
        if self.take(b'!') {
            let cell = self.address_token()?.to_owned();
            return self.cell_reference(&cell, Some(name));
        }
        if !self.take(b'(') {
            if !["TRUE", "FALSE"].contains(&name.to_ascii_uppercase().as_str()) {
                self.cell_reference(name, None)?;
            }
            return Some(());
        }
        // Only functions whose inputs are explicit arguments, not addresses or ambient state.
        if ![
            "LEN",
            "TRIM",
            "ISERROR",
            "ISERR",
            "ISBLANK",
            "ISTEXT",
            "ISNUMBER",
            "SEARCH",
            "FIND",
            "LEFT",
            "RIGHT",
            "MID",
            "AND",
            "OR",
            "NOT",
            "IF",
            "IFERROR",
            "MOD",
            "ABS",
            "ROUND",
            "INT",
            "LOWER",
            "UPPER",
            "EXACT",
            "CODE",
            "SUM",
            "MIN",
            "MAX",
            "AVERAGE",
            "COUNT",
            "COUNTA",
            "COUNTBLANK",
            "COUNTIF",
            "COUNTIFS",
        ]
        .contains(&name.to_ascii_uppercase().as_str())
        {
            return None;
        }
        self.expression()?;
        while self.take(b',') {
            self.expression()?;
        }
        self.take(b')').then_some(())
    }

    fn address_token(&mut self) -> Option<&str> {
        self.whitespace();
        let start = self.position;
        while self
            .bytes
            .get(self.position)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'$')
        {
            self.position += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.position])
            .ok()
            .filter(|token| !token.is_empty())
    }

    fn cell_reference(&mut self, start: &str, sheet: Option<&str>) -> Option<()> {
        let reference = if self.take(b':') {
            let end = self.address_token()?;
            parse(&format!("{start}:{end}"))?
        } else {
            parse(start)?
        };
        self.references.push(if let Some(name) = sheet {
            reference.qualified(name)?
        } else {
            reference
        });
        Some(())
    }

    /// Excel doubles apostrophes inside quoted sheet names.
    fn quoted_sheet(&mut self) -> Option<()> {
        self.position += 1;
        let mut name = Vec::new();
        loop {
            let byte = *self.bytes.get(self.position)?;
            self.position += 1;
            if byte == b'\'' {
                if self.bytes.get(self.position) != Some(&b'\'') {
                    break;
                }
                self.position += 1;
            }
            name.push(byte);
        }
        if !self.take(b'!') {
            return None;
        }
        let cell = self.address_token()?.to_owned();
        self.cell_reference(&cell, Some(std::str::from_utf8(&name).ok()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simple_formulas_collect_only_real_cell_tokens() {
        for (formula, count) in [
            ("NOT(ISERROR(SEARCH(\"ERROR\",F3)))", 1),
            ("IF(A26>89.49%,\"A999\",IF($B$2<1E-3,\"a\"\"b\",\"c\"))", 2),
            ("AND(A1<>0,B$2+$C3>=4)", 3),
            ("COUNTIF($A$1:A10,A1)>1", 2),
            ("Other!A1=1", 1),
            ("SUM('O''Brien data'!$A$1:B3)>0", 1),
        ] {
            assert_eq!(dependencies(formula).unwrap().len(), count, "{formula}");
        }
        for formula in [
            "INDIRECT(\"A1\")",
            "OFFSET(A1,1,0)",
            "'[Other.xlsx]Data'!A1",
            "First:Last!A1",
            "SUM(A:A)",
            "SUMIF(A1:A10,1,B1)",
            "Table1[Value]",
            "NamedRange=A1",
            "NOW()+A1",
            "ROW()=A1",
            "A1 B2",
            "A1+",
            "\"unclosed",
            "A1#",
        ] {
            assert!(dependencies(formula).is_none(), "{formula}");
        }
        let nested = format!(
            "{}A1{}",
            "(".repeat(MAX_FORMULA_NESTING + 1),
            ")".repeat(MAX_FORMULA_NESTING + 1)
        );
        assert!(dependencies(&nested).is_none());
    }
}
