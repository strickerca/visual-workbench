pub fn string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            value if value < ' ' => output.push_str(&format!("\\u{:04x}", value as u32)),
            value => output.push(value),
        }
    }
    output.push('"');
    output
}

pub fn object(fields: &[(&str, String)]) -> String {
    format!(
        "{{{}}}",
        fields
            .iter()
            .map(|(key, value)| format!("{}:{value}", string(key)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

pub fn array(values: &[String]) -> String {
    format!("[{}]", values.join(","))
}

#[cfg(test)]
mod tests {
    #[test]
    fn escaped_controls_and_unicode_are_valid_json_strings() {
        assert_eq!(
            super::string("\"\\\n\r\t\u{0000}é"),
            "\"\\\"\\\\\\n\\r\\t\\u0000é\""
        );
    }
}
