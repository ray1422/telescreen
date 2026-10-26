//! Layer 1: strict YAML to JSON-compatible data (`docs/config.md`, "Validation Layers").

use saphyr_parser::{Event, Parser, ScalarStyle};
use serde_json::{Map, Number, Value};

use super::ConfigError;

/// Parses YAML text into JSON-compatible data.
///
/// Accepts only the JSON-compatible subset of YAML. Rejects with
/// [`ConfigError::Yaml`]: more than one document, duplicate keys, aliases and
/// anchors, tags, merge keys (`<<`), non-string keys, non-finite numbers, and
/// plain scalars that YAML 1.1 and YAML 1.2 resolve differently (see
/// `docs/config.md`).
pub fn to_json(text: &str) -> Result<Value, ConfigError> {
    let mut parser = Parser::new_from_str(text);
    let mut builder = Builder::default();
    let mut document_seen = false;
    while let Some(event) = parser.next_event() {
        let (event, span) = event.map_err(|error| ConfigError::Yaml(error.to_string()))?;
        let line = span.start.line();
        match event {
            Event::StreamStart | Event::DocumentEnd => {}
            Event::StreamEnd => break,
            Event::DocumentStart(_) => {
                if document_seen {
                    return Err(yaml_error(line, "only one YAML document is supported"));
                }
                document_seen = true;
            }
            event => builder.event(event, line)?,
        }
    }
    // An empty stream, like an empty document, represents null.
    Ok(builder.root.unwrap_or(Value::Null))
}

fn yaml_error(line: usize, message: impl std::fmt::Display) -> ConfigError {
    ConfigError::Yaml(format!("{message} at line {line}"))
}

enum Frame {
    Sequence(Vec<Value>),
    Mapping {
        values: Map<String, Value>,
        key: Option<String>,
    },
}

#[derive(Default)]
struct Builder {
    frames: Vec<Frame>,
    root: Option<Value>,
}

impl Builder {
    fn is_mapping_key(&self) -> bool {
        matches!(self.frames.last(), Some(Frame::Mapping { key: None, .. }))
    }

    fn event(&mut self, event: Event<'_>, line: usize) -> Result<(), ConfigError> {
        match &event {
            Event::Scalar(_, _, anchor, tag)
            | Event::SequenceStart(anchor, tag)
            | Event::MappingStart(anchor, tag) => {
                if *anchor != 0 {
                    return Err(yaml_error(line, "YAML anchors are not supported"));
                }
                if tag.is_some() {
                    return Err(yaml_error(line, "YAML tags are not supported"));
                }
            }
            _ => {}
        }
        match event {
            Event::Scalar(text, style, _, _) => {
                if self.is_mapping_key() && style == ScalarStyle::Plain && text == "<<" {
                    return Err(yaml_error(line, "YAML merge keys are not supported"));
                }
                self.value(scalar(&text, style, line)?, line)
            }
            Event::SequenceStart(..) => self.start(Frame::Sequence(Vec::new()), line),
            Event::MappingStart(..) => self.start(
                Frame::Mapping {
                    values: Map::new(),
                    key: None,
                },
                line,
            ),
            Event::SequenceEnd | Event::MappingEnd => self.end(event, line),
            Event::Alias(_) => Err(yaml_error(line, "YAML aliases are not supported")),
            _ => Err(yaml_error(line, "unexpected YAML event")),
        }
    }

    fn start(&mut self, frame: Frame, line: usize) -> Result<(), ConfigError> {
        if self.is_mapping_key() {
            return Err(yaml_error(line, "mapping keys must be strings"));
        }
        self.frames.push(frame);
        Ok(())
    }

    fn end(&mut self, event: Event<'_>, line: usize) -> Result<(), ConfigError> {
        let value = match (self.frames.pop(), event) {
            (Some(Frame::Sequence(values)), Event::SequenceEnd) => Value::Array(values),
            (Some(Frame::Mapping { values, key: None }), Event::MappingEnd) => {
                Value::Object(values)
            }
            _ => return Err(yaml_error(line, "unexpected YAML collection end")),
        };
        self.value(value, line)
    }

    fn value(&mut self, value: Value, line: usize) -> Result<(), ConfigError> {
        match self.frames.last_mut() {
            Some(Frame::Sequence(values)) => values.push(value),
            Some(Frame::Mapping { values, key }) => {
                if let Some(key) = key.take() {
                    values.insert(key, value);
                } else {
                    let Value::String(text) = value else {
                        return Err(yaml_error(line, "mapping keys must be strings"));
                    };
                    if values.contains_key(&text) {
                        return Err(yaml_error(line, format_args!("duplicate YAML key: {text}")));
                    }
                    *key = Some(text);
                }
            }
            None => {
                if self.root.replace(value).is_some() {
                    return Err(yaml_error(line, "multiple YAML root values"));
                }
            }
        }
        Ok(())
    }
}

fn scalar(text: &str, style: ScalarStyle, line: usize) -> Result<Value, ConfigError> {
    if style != ScalarStyle::Plain {
        return Ok(Value::String(text.to_owned()));
    }
    if ambiguous(text) {
        return Err(yaml_error(
            line,
            format_args!("ambiguous plain scalar (or non-finite number), quote it: {text}"),
        ));
    }
    match text {
        "" | "~" | "null" | "Null" | "NULL" => return Ok(Value::Null),
        "true" | "True" | "TRUE" => return Ok(Value::Bool(true)),
        "false" | "False" | "FALSE" => return Ok(Value::Bool(false)),
        _ => {}
    }
    if let Some(hex) = text
        .strip_prefix("0x")
        .filter(|hex| !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        let number = match u64::from_str_radix(hex, 16) {
            Ok(number) => Number::from(number),
            Err(_) => finite_number(
                hex.bytes().fold(0.0, |value, byte| {
                    let digit = match byte {
                        b'0'..=b'9' => byte - b'0',
                        b'a'..=b'f' => byte - b'a' + 10,
                        _ => byte - b'A' + 10,
                    };
                    value * 16.0 + f64::from(digit)
                }),
                line,
            )?,
        };
        return Ok(Value::Number(number));
    }
    if digits(unsigned(text)) {
        if let Ok(number) = text.parse::<i64>() {
            return Ok(Value::Number(number.into()));
        }
        if let Ok(number) = text.parse::<u64>() {
            return Ok(Value::Number(number.into()));
        }
        return float_value(text, line);
    }
    if decimal(unsigned(text)) {
        return float_value(text, line);
    }
    Ok(Value::String(text.to_owned()))
}

fn float_value(text: &str, line: usize) -> Result<Value, ConfigError> {
    let number = text
        .parse::<f64>()
        .map_err(|error| yaml_error(line, format_args!("invalid number {text}: {error}")))?;
    Ok(Value::Number(finite_number(number, line)?))
}

fn finite_number(number: f64, line: usize) -> Result<Number, ConfigError> {
    Number::from_f64(number).ok_or_else(|| yaml_error(line, "non-finite number"))
}

fn unsigned(text: &str) -> &str {
    text.strip_prefix(['-', '+']).unwrap_or(text)
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

// The exponent forms are rejected as ambiguous before core scalar resolution.
fn decimal(text: &str) -> bool {
    match text.split_once('.') {
        Some((whole, fraction)) => {
            (whole.is_empty() || digits(whole))
                && (fraction.is_empty() || digits(fraction))
                && !(whole.is_empty() && fraction.is_empty())
        }
        None => digits(text),
    }
}

fn ambiguous(text: &str) -> bool {
    if matches!(
        text,
        "y" | "Y"
            | "n"
            | "N"
            | "yes"
            | "Yes"
            | "YES"
            | "no"
            | "No"
            | "NO"
            | "on"
            | "On"
            | "ON"
            | "off"
            | "Off"
            | "OFF"
            | "="
    ) || timestamp_prefix(text)
    {
        return true;
    }
    let number = unsigned(text);
    matches!(number, ".inf" | ".Inf" | ".INF")
        || matches!(text, ".nan" | ".NaN" | ".NAN")
        || (number.starts_with('0') && number.len() > 1 && digits(number))
        || prefixed_digits(number, "0b", b"01_")
        || prefixed_digits(number, "0o", b"01234567_")
        || underscored_number(number)
        || sexagesimal(number)
        || exponent(number)
}

fn prefixed_digits(text: &str, prefix: &str, allowed: &[u8]) -> bool {
    text.strip_prefix(prefix)
        .is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|b| allowed.contains(&b)))
}

fn underscored_number(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_digit())
        && text.contains('_')
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'_'))
}

fn timestamp_prefix(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 8 || !bytes[..4].iter().all(u8::is_ascii_digit) || bytes[4] != b'-' {
        return false;
    }
    let mut rest = &bytes[5..];
    let month_len = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if !(1..=2).contains(&month_len) || rest.get(month_len) != Some(&b'-') {
        return false;
    }
    rest = &rest[month_len + 1..];
    rest.first().is_some_and(u8::is_ascii_digit)
}

fn sexagesimal(text: &str) -> bool {
    let (whole, fraction) = text
        .split_once('.')
        .map_or((text, None), |(whole, fraction)| (whole, Some(fraction)));
    let mut groups = whole.split(':');
    let Some(first) = groups.next() else {
        return false;
    };
    first.starts_with(|c: char| c.is_ascii_digit())
        && digit_underscores(first)
        && whole.contains(':')
        && groups.all(|group| !group.is_empty() && digit_underscores(group))
        && fraction.is_none_or(digit_underscores)
}

fn digit_underscores(text: &str) -> bool {
    text.bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'_')
}

fn exponent(text: &str) -> bool {
    let Some((mantissa, power)) = text.split_once(['e', 'E']) else {
        return false;
    };
    // Underscores in the mantissa follow the ambiguous forms in the reference.
    let valid_mantissa = if let Some(fraction) = mantissa.strip_prefix('.') {
        fraction.starts_with(|c: char| c.is_ascii_digit()) && digit_underscores(fraction)
    } else {
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        whole.starts_with(|c: char| c.is_ascii_digit())
            && digit_underscores(whole)
            && digit_underscores(fraction)
    };
    valid_mantissa && digits(unsigned(power))
}
