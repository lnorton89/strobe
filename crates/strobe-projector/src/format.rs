// ============================================================================
// Module:       strobe_projector::format
// Description:  PJLink's terse status answers in words: the input, the lamp
//               hours, and the error-status digits.
//
// Dependencies: none
// ============================================================================

//! Status answers, readable.
//!
//! Each function returns its input unchanged when it does not have the
//! expected shape, so an unusual unit still shows something.

/// `INPT`'s first digit.
const INPUT_TYPES: [(char, &str); 5] = [
    ('1', "RGB"),
    ('2', "VIDEO"),
    ('3', "DIGITAL"),
    ('4', "STORAGE"),
    ('5', "NETWORK"),
];

/// `ERST`'s six digits, in order.
const ERST_FIELDS: [&str; 6] = [
    "fan",
    "lamp/light",
    "temperature",
    "cover",
    "filter",
    "other",
];

/// `31` as `DIGITAL 1 (31)`.
pub fn input(value: &str) -> String {
    let mut chars = value.chars();
    if let (Some(kind), Some(number), None) = (chars.next(), chars.next(), chars.next())
        && let Some((_, name)) = INPUT_TYPES.iter().find(|(code, _)| *code == kind)
    {
        return format!("{name} {number} ({value})");
    }
    value.to_string()
}

/// `1234 1` as `lamp1: 1234 h (on)`, one entry per lamp.
pub fn lamp(value: &str) -> String {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let lamps: Vec<String> = parts
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
        .map(|(index, [hours, flag])| {
            let lit = *flag == "1";
            format!(
                "lamp{}: {hours} h ({})",
                index + 1,
                if lit { "on" } else { "off" }
            )
        })
        .collect();
    if lamps.is_empty() {
        value.to_string()
    } else {
        lamps.join(", ")
    }
}

/// `000010` as `filter=WARNING`, or `no errors`.
pub fn erst(value: &str) -> String {
    if value.chars().count() != ERST_FIELDS.len() {
        return value.to_string();
    }
    let bad: Vec<String> = ERST_FIELDS
        .iter()
        .zip(value.chars())
        .filter(|(_, level)| *level != '0')
        .map(|(field, level)| {
            let level = match level {
                '1' => "WARNING".to_string(),
                '2' => "ERROR".to_string(),
                other => other.to_string(),
            };
            format!("{field}={level}")
        })
        .collect();
    if bad.is_empty() {
        "no errors".to_string()
    } else {
        bad.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_read_as_words() {
        assert_eq!(input("31"), "DIGITAL 1 (31)", "input");
        assert_eq!(input("9Z"), "9Z", "unknown input kept");
        assert_eq!(lamp("1234 1"), "lamp1: 1234 h (on)", "one lamp");
        assert_eq!(
            lamp("10 0 20 1"),
            "lamp1: 10 h (off), lamp2: 20 h (on)",
            "two lamps"
        );
        assert_eq!(lamp(""), "", "no lamps");
        assert_eq!(erst("000000"), "no errors", "clean");
        assert_eq!(erst("000010"), "filter=WARNING", "filter");
        assert_eq!(erst("200001"), "fan=ERROR, other=WARNING", "two");
        assert_eq!(erst("00"), "00", "short kept");
    }
}
