//! Strict content admission for the small PC vector-preserving subset. This
//! reads already-decoded private worker bytes and never retains source strings.
use crate::pc_redaction::{PcRedactionError, PcRedactionResult};

pub(crate) const MAX_CONTENT: usize = 16 * 1024 * 1024;
const MAX_TOKENS: usize = 1_000_000;
const MAX_STACK: usize = 64;

/// The allowlist deliberately omits clipping, marked content, external graphics
/// states, color spaces/patterns, XObjects and shadings. Text may paint normally;
/// object admission later refuses any text that would remain outside raster.
pub(crate) fn admit_source_content(bytes: &[u8]) -> PcRedactionResult<()> {
    if bytes.len() > MAX_CONTENT {
        return Err(PcRedactionError::Limit);
    }
    let mut lexer = Lexer {
        bytes,
        offset: 0,
        tokens: 0,
    };
    let mut operands = Vec::new();
    operands
        .try_reserve_exact(MAX_STACK)
        .map_err(|_| PcRedactionError::Limit)?;
    let mut saves = 0usize;
    let mut text_open = false;
    while let Some(token) = lexer.next()? {
        match token {
            Token::Operand(value) => {
                if operands.len() == MAX_STACK {
                    return Err(PcRedactionError::Limit);
                }
                operands.push(value);
            }
            Token::Operator(operator) => {
                let numeric = |count| {
                    operands.len() == count
                        && operands
                            .iter()
                            .all(|value| matches!(value, Operand::Number(_)))
                };
                let admitted = match operator {
                    b"q" if !text_open => {
                        saves = saves.checked_add(1).ok_or(PcRedactionError::Limit)?;
                        saves <= MAX_STACK && operands.is_empty()
                    }
                    b"Q" if !text_open => {
                        saves = saves
                            .checked_sub(1)
                            .ok_or(PcRedactionError::UnsupportedContent)?;
                        operands.is_empty()
                    }
                    b"cm" | b"c" if !text_open => numeric(6),
                    b"m" | b"l" if !text_open => numeric(2),
                    b"re" | b"v" | b"y" if !text_open => numeric(4),
                    b"w" | b"J" | b"j" | b"M" if !text_open => numeric(1),
                    b"d" if !text_open => matches!(
                        operands.as_slice(),
                        [Operand::Array { numeric: true }, Operand::Number(_)]
                    ),
                    b"rg" | b"RG" if !text_open => quantized_color(&operands, 3),
                    b"g" | b"G" if !text_open => quantized_color(&operands, 1),
                    b"h" | b"f" | b"F" | b"f*" | b"S" | b"s" | b"B" | b"B*" | b"b" | b"b*"
                    | b"n"
                        if !text_open =>
                    {
                        operands.is_empty()
                    }
                    b"BT" if !text_open => {
                        text_open = true;
                        operands.is_empty()
                    }
                    b"ET" if text_open => {
                        text_open = false;
                        operands.is_empty()
                    }
                    b"Tc" | b"Tw" | b"Tz" | b"TL" | b"Ts" if text_open => numeric(1),
                    b"Td" | b"TD" if text_open => numeric(2),
                    b"Tf" if text_open => {
                        matches!(operands.as_slice(), [Operand::Name, Operand::Number(size)] if *size > 0.0)
                    }
                    b"Tm" if text_open => numeric(6),
                    b"T*" if text_open => operands.is_empty(),
                    b"Tj" | b"'" if text_open => matches!(operands.as_slice(), [Operand::String]),
                    b"TJ" if text_open => matches!(operands.as_slice(), [Operand::Array { .. }]),
                    b"\"" if text_open => matches!(
                        operands.as_slice(),
                        [Operand::Number(_), Operand::Number(_), Operand::String]
                    ),
                    b"Tr" if text_open => {
                        matches!(operands.as_slice(), [Operand::Number(value)] if *value == 0.0)
                    }
                    _ => false,
                };
                if !admitted {
                    return Err(PcRedactionError::UnsupportedContent);
                }
                operands.clear();
            }
        }
    }
    if !operands.is_empty() || saves != 0 || text_open {
        return Err(PcRedactionError::UnsupportedContent);
    }
    Ok(())
}

fn quantized_color(operands: &[Operand], count: usize) -> bool {
    operands.len() == count
        && operands.iter().all(|number| match number {
            Operand::Number(value) => {
                (0.0..=1.0).contains(value)
                    && (value * 255.0 - (value * 255.0).round()).abs() <= 0.000_01
            }
            _ => false,
        })
}

enum Operand {
    Number(f64),
    Name,
    String,
    Array { numeric: bool },
}
enum Token<'a> {
    Operand(Operand),
    Operator(&'a [u8]),
}
struct Lexer<'a> {
    bytes: &'a [u8],
    offset: usize,
    tokens: usize,
}
impl<'a> Lexer<'a> {
    fn next(&mut self) -> PcRedactionResult<Option<Token<'a>>> {
        self.space();
        if self.offset == self.bytes.len() {
            return Ok(None);
        }
        self.tokens += 1;
        if self.tokens > MAX_TOKENS {
            return Err(PcRedactionError::Limit);
        }
        let first = self.bytes[self.offset];
        match first {
            b'(' => {
                self.string()?;
                Ok(Some(Token::Operand(Operand::String)))
            }
            b'<' => {
                self.hex()?;
                Ok(Some(Token::Operand(Operand::String)))
            }
            b'[' => {
                let numeric = self.array()?;
                Ok(Some(Token::Operand(Operand::Array { numeric })))
            }
            b'/' => {
                self.offset += 1;
                let start = self.offset;
                while self.offset < self.bytes.len() && !delimiter(self.bytes[self.offset]) {
                    self.offset += 1;
                }
                if self.offset == start || self.offset - start > 127 {
                    return Err(PcRedactionError::UnsupportedContent);
                }
                Ok(Some(Token::Operand(Operand::Name)))
            }
            b']' | b')' | b'>' | b'{' | b'}' => Err(PcRedactionError::UnsupportedContent),
            _ => {
                let start = self.offset;
                while self.offset < self.bytes.len() && !delimiter(self.bytes[self.offset]) {
                    self.offset += 1;
                }
                let word = &self.bytes[start..self.offset];
                if word.is_empty() || word.len() > 64 {
                    return Err(PcRedactionError::Limit);
                }
                if matches!(first, b'+' | b'-' | b'.' | b'0'..=b'9') {
                    // PDF numbers have decimal syntax, not exponents/NaN/Inf.
                    let digits = word
                        .strip_prefix(b"+")
                        .or_else(|| word.strip_prefix(b"-"))
                        .unwrap_or(word);
                    if !digits.iter().all(|c| c.is_ascii_digit() || *c == b'.')
                        || digits.iter().filter(|c| **c == b'.').count() > 1
                        || !digits.iter().any(u8::is_ascii_digit)
                    {
                        return Err(PcRedactionError::UnsupportedContent);
                    }
                    let value = std::str::from_utf8(word)
                        .ok()
                        .and_then(|word| word.parse::<f64>().ok())
                        .filter(|v| v.is_finite() && v.abs() <= 10_000_000.0)
                        .ok_or(PcRedactionError::UnsupportedContent)?;
                    Ok(Some(Token::Operand(Operand::Number(value))))
                } else {
                    Ok(Some(Token::Operator(word)))
                }
            }
        }
    }
    fn space(&mut self) {
        while self.offset < self.bytes.len() {
            if white(self.bytes[self.offset]) {
                self.offset += 1;
            } else if self.bytes[self.offset] == b'%' {
                while self.offset < self.bytes.len() && !b"\r\n".contains(&self.bytes[self.offset])
                {
                    self.offset += 1;
                }
            } else {
                break;
            }
        }
    }
    fn string(&mut self) -> PcRedactionResult<()> {
        self.offset += 1;
        let mut depth = 1usize;
        while self.offset < self.bytes.len() {
            let value = self.bytes[self.offset];
            self.offset += 1;
            match value {
                b'\\' => {
                    if self.offset == self.bytes.len() {
                        break;
                    }
                    self.offset += 1;
                }
                b'(' => {
                    depth += 1;
                    if depth > MAX_STACK {
                        return Err(PcRedactionError::Limit);
                    }
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        Err(PcRedactionError::UnsupportedContent)
    }
    fn hex(&mut self) -> PcRedactionResult<()> {
        self.offset += 1;
        while self.offset < self.bytes.len() {
            let value = self.bytes[self.offset];
            self.offset += 1;
            if value == b'>' {
                return Ok(());
            }
            if !value.is_ascii_hexdigit() && !white(value) {
                return Err(PcRedactionError::UnsupportedContent);
            }
        }
        Err(PcRedactionError::UnsupportedContent)
    }
    fn array(&mut self) -> PcRedactionResult<bool> {
        self.offset += 1;
        let mut numeric = true;
        loop {
            self.space();
            let Some(&next) = self.bytes.get(self.offset) else {
                return Err(PcRedactionError::UnsupportedContent);
            };
            if next == b']' {
                self.offset += 1;
                return Ok(numeric);
            }
            // TJ admits only flat strings/numbers; dash arrays additionally
            // require numeric elements. Nested arrays and names are refused.
            if next == b'[' {
                return Err(PcRedactionError::UnsupportedContent);
            }
            match self.next()? {
                Some(Token::Operand(Operand::Number(_))) => {}
                Some(Token::Operand(Operand::String)) => numeric = false,
                _ => return Err(PcRedactionError::UnsupportedContent),
            }
        }
    }
}
fn white(value: u8) -> bool {
    b"\0\t\n\x0c\r ".contains(&value)
}
fn delimiter(value: u8) -> bool {
    white(value) || b"()<>[]{}/%".contains(&value)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    #[test]
    fn plain_polygon_and_covered_text_syntax_are_admitted() {
        assert!(
            admit_source_content(
                b"q 1 0 0 rg 2 2 m 8 2 l 8 8 l h f Q BT /F1 8 Tf 2 20 Td (secret) Tj ET"
            )
            .is_ok()
        );
    }
    #[test]
    fn clip_blend_xobject_and_marked_content_are_refused() {
        for bytes in [
            b"0 0 8 8 re W n".as_slice(),
            b"/G1 gs",
            b"/X1 Do",
            b"/Span BMC",
            b"/DeviceRGB cs",
        ] {
            assert!(admit_source_content(bytes).is_err());
        }
    }
    #[test]
    fn invisible_or_clipping_text_modes_are_refused() {
        for mode in 1..8 {
            assert!(admit_source_content(format!("BT {mode} Tr ET").as_bytes()).is_err());
        }
        assert!(admit_source_content(b"BT 0 Tr ET").is_ok());
    }
    #[test]
    fn operators_inside_strings_do_not_confuse_admission() {
        assert!(
            admit_source_content(
                b"BT /F1 8 Tf (gs W /X1 Do \\(nested\\)) Tj [(Do) -20 <5757>] TJ ET"
            )
            .is_ok()
        );
    }
    #[test]
    fn fractional_non_eight_bit_colors_are_visibly_refused() {
        assert!(admit_source_content(b"0.12345 0 1 rg").is_err());
        assert!(admit_source_content(b"0.5019607843137255 g").is_ok());
    }
    #[test]
    fn malformed_structures_and_unknown_numbers_refuse() {
        for bytes in [
            b"Q".as_slice(),
            b"q",
            b"BT",
            b"0 1 cm",
            b"BT (secret Tj ET",
            b"BT <nothex> Tj ET",
            b"1e2 1 m",
            b"BT [Do] TJ ET",
            b"--1 2 m",
            b". 2 m",
        ] {
            assert!(admit_source_content(bytes).is_err());
        }
    }
    #[test]
    fn text_operands_are_typed_and_text_operators_require_text_state() {
        for bytes in [
            b"1 2 Td".as_slice(),
            b"0 Tc",
            b"BT (wrong) 8 Tf ET",
            b"BT /F1 (wrong) Tf ET",
            b"BT /F1 -1 Tf ET",
            b"BT 1 Tj ET",
            b"BT [(x) /F1] TJ ET",
            b"BT [[(x)]] TJ ET",
            b"BT (x) (y) (z) \" ET",
            b"[(x)] 0 d",
        ] {
            assert!(admit_source_content(bytes).is_err());
        }
        assert!(admit_source_content(b"[3 2] 0 d BT /F1 8 Tf 0 Tc 0 Tw 100 Tz 9 TL 0 Ts 1 2 Td (x) ' 1 2 (y) \" [(z) -10] TJ ET").is_ok());
    }
    #[test]
    fn path_paint_and_state_stack_cannot_cross_text_boundaries() {
        for bytes in [
            b"BT 1 2 m ET".as_slice(),
            b"BT 1 0 0 rg ET",
            b"BT q ET Q",
            b"BT BT ET ET",
            b"q BT ET",
        ] {
            assert!(admit_source_content(bytes).is_err());
        }
    }
}
