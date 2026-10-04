use crate::Result;
use std::io::{BufRead, Write};
pub fn line(input: &mut impl BufRead, limit: usize) -> Result<Option<Vec<u8>>> {
    let mut out = Vec::new();
    loop {
        let bytes = input.fill_buf().map_err(|_| "read")?;
        if bytes.is_empty() {
            return if out.is_empty() {
                Ok(None)
            } else {
                Err("truncated")
            };
        }
        let count = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |n| n + 1);
        if out.len().saturating_add(count) > limit + 1 {
            return Err("frame_limit");
        }
        let done = bytes[count - 1] == b'\n';
        out.extend_from_slice(&bytes[..count]);
        input.consume(count);
        if done {
            out.pop();
            if out.is_empty() {
                return Err("empty_frame");
            }
            return Ok(Some(out));
        }
    }
}
pub fn parse(bytes: &[u8]) -> Result<serde_json::Value> {
    if bytes.len() > crate::WIRE_BYTES {
        return Err("frame_limit");
    }
    let (mut depth, mut nodes, mut quoted, mut escape) = (0i32, 0u32, false, false);
    for b in bytes {
        if quoted {
            if escape {
                escape = false;
            } else if *b == b'\\' {
                escape = true;
            } else if *b == b'"' {
                quoted = false;
            }
            continue;
        }
        match b {
            b'"' => {
                quoted = true;
                nodes += 1;
            }
            b'{' | b'[' => {
                depth += 1;
                nodes += 1;
            }
            b'}' | b']' => depth -= 1,
            b',' => nodes += 1,
            _ => {}
        }
        if !(0..=32).contains(&depth) || nodes > 32768 {
            return Err("complexity");
        }
    }
    serde_json::from_slice(bytes).map_err(|_| "json")
}
pub fn write(output: &mut impl Write, value: &serde_json::Value) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| "json")?;
    if bytes.len() > crate::WIRE_BYTES {
        return Err("frame_limit");
    }
    output
        .write_all(&bytes)
        .and_then(|()| output.write_all(b"\n"))
        .and_then(|()| output.flush())
        .map_err(|_| "write")
}
/// The borrowed credential is never cloned into a generic JSON Value. Partial
/// serialization and write failures both drop the dedicated zeroizing buffer.
pub fn write_init(
    output: &mut impl Write,
    token: &str,
    port: u16,
    verifier: &std::path::Path,
) -> Result<()> {
    if token.len() != 64
        || !token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || verifier.to_str().is_none_or(|path| path.len() > 4096)
    {
        return Err("init");
    }
    #[derive(serde::Serialize)]
    struct Init<'a> {
        kind: &'static str,
        token: &'a str,
        port: u16,
        verifier: &'a std::path::Path,
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    serde_json::to_writer(
        &mut *bytes,
        &Init {
            kind: "init",
            token,
            port,
            verifier,
        },
    )
    .map_err(|_| "json")?;
    if bytes.len() > 8192 {
        return Err("frame_limit");
    }
    output
        .write_all(&bytes)
        .and_then(|()| output.write_all(b"\n"))
        .and_then(|()| output.flush())
        .map_err(|_| "write")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limit_applies_before_unterminated_allocation() {
        let mut x = std::io::Cursor::new(vec![b'x'; 100]);
        assert_eq!(line(&mut x, 8), Err("frame_limit"));
    }
    #[test]
    fn exact_limit_and_eof() {
        let mut x = std::io::Cursor::new(b"{}\n");
        assert_eq!(line(&mut x, 2), Ok(Some(b"{}".to_vec())));
        assert_eq!(line(&mut x, 2), Ok(None));
    }
    #[test]
    fn structural_bound_precedes_serde() {
        assert_eq!(
            parse(format!("[{}]", "0,".repeat(32769)).as_bytes()),
            Err("complexity")
        );
        assert!(parse(br#"{"quoted":"[[["}"#).is_ok());
    }
    #[test]
    fn startup_serializes_borrowed_fields_and_propagates_write_refusal()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let token = "a".repeat(64);
        let mut bytes = Vec::new();
        write_init(&mut bytes, &token, 42, std::path::Path::new("verifier"))?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(value["token"], token);
        assert_eq!(value["port"], 42);
        struct Refuse;
        impl Write for Refuse {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("fixture"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(
            write_init(&mut Refuse, &token, 42, std::path::Path::new("verifier")),
            Err("write")
        );
        Ok(())
    }
}
