use std::collections::HashMap;
use std::path::Path;

use crate::{Error, Result};

pub type Environment = HashMap<String, String>;

pub fn collect(path: Option<&Path>) -> Result<Environment> {
    let mut values = Environment::new();
    if let Some(path) = path.filter(|path| path.is_file()) {
        let iter = dotenvy::from_path_iter(path).map_err(|source| Error::ReadFile {
            path: path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
        })?;
        for item in iter {
            let (key, value) = item.map_err(|source| Error::ReadFile {
                path: path.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
            })?;
            values.insert(key, value);
        }
    }

    values.extend(std::env::vars());
    Ok(values)
}

pub fn interpolate(input: &str, env: &Environment) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'$' {
            let ch = input[index..]
                .chars()
                .next()
                .expect("index is inside string");
            output.push(ch);
            index += ch.len_utf8();
            continue;
        }

        if bytes.get(index + 1) == Some(&b'$') {
            output.push('$');
            index += 2;
            continue;
        }

        if bytes.get(index + 1) == Some(&b'{') {
            let body_start = index + 2;
            let Some(relative_end) = input[body_start..].find('}') else {
                return Err(Error::InvalidConfig(
                    "unterminated environment interpolation".to_owned(),
                ));
            };
            let body_end = body_start + relative_end;
            output.push_str(&expand_expression(&input[body_start..body_end], env)?);
            index = body_end + 1;
            continue;
        }

        let name_start = index + 1;
        let mut name_end = name_start;
        while let Some(byte) = bytes.get(name_end) {
            if byte.is_ascii_alphanumeric() || *byte == b'_' {
                name_end += 1;
            } else {
                break;
            }
        }
        if name_end == name_start {
            output.push('$');
            index += 1;
        } else {
            let name = &input[name_start..name_end];
            output.push_str(env.get(name).map(String::as_str).unwrap_or_default());
            index = name_end;
        }
    }

    Ok(output)
}

fn expand_expression(expression: &str, env: &Environment) -> Result<String> {
    let name_end = expression
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .unwrap_or(expression.len());
    let name = &expression[..name_end];
    if name.is_empty()
        || !name
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
    {
        return Err(Error::InvalidConfig(format!(
            "invalid environment expression: ${{{expression}}}"
        )));
    }

    let suffix = &expression[name_end..];
    let value = env.get(name);
    let is_set = value.is_some();
    let is_nonempty = value.is_some_and(|value| !value.is_empty());

    let (operator, operand) = [":-", "-", ":?", "?", ":+", "+"]
        .into_iter()
        .find_map(|operator| suffix.strip_prefix(operator).map(|rest| (operator, rest)))
        .unwrap_or(("", ""));

    let expanded_operand = || interpolate(operand, env);
    match operator {
        "" if suffix.is_empty() => Ok(value.cloned().unwrap_or_default()),
        ":-" if !is_nonempty => expanded_operand(),
        "-" if !is_set => expanded_operand(),
        ":?" if !is_nonempty => Err(Error::RequiredVariable {
            name: name.to_owned(),
            message: if operand.is_empty() {
                "variable is unset or empty".to_owned()
            } else {
                operand.to_owned()
            },
        }),
        "?" if !is_set => Err(Error::RequiredVariable {
            name: name.to_owned(),
            message: if operand.is_empty() {
                "variable is unset".to_owned()
            } else {
                operand.to_owned()
            },
        }),
        ":+" if is_nonempty => expanded_operand(),
        "+" if is_set => expanded_operand(),
        ":+" | "+" => Ok(String::new()),
        ":-" | "-" | ":?" | "?" => Ok(value.cloned().unwrap_or_default()),
        _ => Err(Error::InvalidConfig(format!(
            "unsupported environment expression: ${{{expression}}}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(values: &[(&str, &str)]) -> Environment {
        values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn expands_compose_parameter_forms() {
        let values = env(&[("SET", "value"), ("EMPTY", "")]);
        assert_eq!(interpolate("$SET", &values).unwrap(), "value");
        assert_eq!(interpolate("${SET}", &values).unwrap(), "value");
        assert_eq!(
            interpolate("${MISSING:-fallback}", &values).unwrap(),
            "fallback"
        );
        assert_eq!(interpolate("${EMPTY-fallback}", &values).unwrap(), "");
        assert_eq!(
            interpolate("${EMPTY:-fallback}", &values).unwrap(),
            "fallback"
        );
        assert_eq!(interpolate("${SET:+yes}", &values).unwrap(), "yes");
        assert_eq!(interpolate("$$SET", &values).unwrap(), "$SET");
    }

    #[test]
    fn reports_required_variables() {
        let error = interpolate("${TOKEN:?configure TOKEN}", &Environment::new()).unwrap_err();
        assert!(error.to_string().contains("configure TOKEN"));
    }
}
