use fabricator_vm::LineNumber;

/// Find the line breaks in a string and record them, allowing querying the line number for any
/// given byte offset.
///
/// Any of  "\n", "\r", "\n\r", or "\r\n" in the source string is counted as a single newline.
#[derive(Clone)]
pub struct LineNumbers {
    // Stores the byte offset immediately after the first character of each break.
    line_breaks: Vec<usize>,
}

impl LineNumbers {
    pub fn new(src: &str) -> Self {
        fn is_newline(c: u8) -> bool {
            c == b'\n' || c == b'\r'
        }

        let mut line_breaks = Vec::new();
        let mut bytes = src.bytes().peekable();
        while let Some(c) = bytes.next() {
            // Either newline character is counted as a newline, if it is followed by the *other*
            // newline character, this is counted as part of the same newline.
            if is_newline(c) {
                line_breaks.push(src.len() - bytes.len());
                bytes.next_if(|&n| is_newline(n) && n != c);
            }
        }

        Self { line_breaks }
    }

    /// Returns the line number for this byte offset.
    ///
    /// If a byte offset is given that is in the *middle* of a newline character pair, then this
    /// will count as the line following the newline character pair.
    pub fn line(&self, byte_offset: usize) -> LineNumber {
        LineNumber(match self.line_breaks.binary_search(&byte_offset) {
            Ok(i) => i + 1,
            Err(i) => i,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_numbers() {
        let src = "\r1\n2\r\n3\n\r4";
        assert_eq!(src.len(), 10);

        let line_numbers = LineNumbers::new(src);
        assert_eq!(line_numbers.line(0).0, 0);
        assert_eq!(line_numbers.line(1).0, 1);
        assert_eq!(line_numbers.line(2).0, 1);
        assert_eq!(line_numbers.line(3).0, 2);
        assert_eq!(line_numbers.line(4).0, 2);
        assert_eq!(line_numbers.line(5).0, 3);
        assert_eq!(line_numbers.line(6).0, 3);
        assert_eq!(line_numbers.line(7).0, 3);
        assert_eq!(line_numbers.line(8).0, 4);
        assert_eq!(line_numbers.line(9).0, 4);
        assert_eq!(line_numbers.line(10).0, 4);
    }
}
