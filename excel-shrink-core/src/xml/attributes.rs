//! Borrowed, forward-only attribute scanning. Input is the portion after a tag's
//! name and before its closing `>` or `/>`. Values are not entity-decoded.

#[derive(Debug, Clone, Copy)]
pub(crate) struct Attribute<'a> {
    pub name: &'a [u8],
    pub value: &'a [u8],
    pub assignment: &'a [u8],
    pub value_start: usize,
    pub value_end: usize,
}

pub(crate) struct Attributes<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Attributes<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        Self { input, position: 0 }
    }

    fn skip_space(&mut self) {
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }

    fn read_attribute(&mut self) -> Option<Attribute<'a>> {
        let start = self.position;
        while self.input.get(self.position).is_some_and(|b| {
            !b.is_ascii_whitespace() && !matches!(b, b'=' | b'\'' | b'"' | b'<' | b'>' | b'/')
        }) {
            self.position += 1;
        }
        let name_end = self.position;
        if name_end == start {
            return None;
        }
        self.skip_space();
        if self.input.get(self.position) != Some(&b'=') {
            return None;
        }
        self.position += 1;
        self.skip_space();
        let quote = *self.input.get(self.position)?;
        if !matches!(quote, b'\'' | b'"') {
            return None;
        }
        self.position += 1;
        let value_start = self.position;
        let length = self.input[value_start..].iter().position(|b| *b == quote)?;
        let value_end = value_start + length;
        self.position = value_end + 1;
        if self
            .input
            .get(self.position)
            .is_some_and(|b| !b.is_ascii_whitespace())
        {
            return None;
        }
        Some(Attribute {
            name: &self.input[start..name_end],
            value: &self.input[value_start..value_end],
            assignment: &self.input[start..self.position],
            value_start,
            value_end,
        })
    }
}

impl<'a> Iterator for Attributes<'a> {
    type Item = Result<Attribute<'a>, ()>;

    fn next(&mut self) -> Option<Self::Item> {
        self.skip_space();
        if self.position == self.input.len() {
            return None;
        }
        match self.read_attribute() {
            Some(attribute) => Some(Ok(attribute)),
            None => {
                self.position = self.input.len();
                Some(Err(()))
            }
        }
    }
}
