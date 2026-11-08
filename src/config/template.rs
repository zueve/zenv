//! Substitute known variables in parsed TOML strings, preserving escaped braces.

use std::collections::BTreeMap;

use ::toml::Value;

pub(super) fn interpolate_value(value: &mut Value, parameters: &BTreeMap<&str, String>) {
    match value {
        Value::String(text) => *text = interpolate(text, parameters),
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| interpolate_value(value, parameters)),
        Value::Table(values) => values
            .iter_mut()
            .for_each(|(_, value)| interpolate_value(value, parameters)),
        _ => {}
    }
}

fn interpolate(source: &str, parameters: &BTreeMap<&str, String>) -> String {
    let mut result = String::new();
    let mut rest = source;
    while let Some(index) = rest.find(['{', '}']) {
        result.push_str(&rest[..index]);
        rest = &rest[index..];
        if rest.starts_with("{{") || rest.starts_with("}}") {
            result.push(rest.as_bytes()[0] as char);
            rest = &rest[2..];
        } else if rest.starts_with('{') {
            if let Some(end) = rest.find('}') {
                let token = &rest[1..end];
                result.push_str(parameters.get(token).map_or(&rest[..=end], String::as_str));
                rest = &rest[end + 1..];
            } else {
                result.push_str(rest);
                return result;
            }
        } else {
            result.push('}');
            rest = &rest[1..];
        }
    }
    result.push_str(rest);
    result
}
