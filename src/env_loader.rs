use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use anyhow::{Context, Result, anyhow};
use crate::spec::EnvVariable;

#[derive(Debug, PartialEq, Eq)]
pub struct EnvDescriptor {
    pub name: String,
    pub default: Option<String>,
}


pub fn parse_env_string(input: &str) -> Result<EnvDescriptor> {
    let input = input.trim();
    if input.is_empty() {
        return Err(anyhow!("Empty string"));
    }

    if let Some((name, value)) = input.split_once('=') {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(anyhow!("Empty name"));
        }
        let value = value.trim();

        if (value.starts_with('"') && value.ends_with('"')) || 
           (value.starts_with('\'') && value.ends_with('\'')) {
            if value.len() >= 2 {
                let inner = &value[1..value.len() - 1];
                return Ok(EnvDescriptor {
                    name,
                    default: Some(inner.to_string()),
                });
            } else {
                return Err(anyhow!("Invalid quoted value"));
            }
        }

        // Check for unquoted '='
        if value.contains('=') {
             return Err(anyhow!("Unquoted '=' found in value. Use quotes."));
        }

        Ok(EnvDescriptor {
            name,
            default: Some(value.to_string()),
        })
    } else {
        Ok(EnvDescriptor {
            name: input.to_string(),
            default: None,
        })
    }
}

pub fn parse_env_variable(input: &str) -> Result<EnvVariable> {
    let desc = parse_env_string(input)?;
    Ok(EnvVariable {
        name: desc.name,
        value: desc.default.ok_or_else(|| anyhow!("No value for env variable: {}", input))?,
    })
}

pub fn load_env_file<P: AsRef<Path>>(path: P) -> Result<Vec<EnvVariable>> {
    let path_ref = path.as_ref();
    let file = File::open(path_ref).context(format!("Failed to open .env file: {:?}", path_ref))?;
    let reader = BufReader::new(file);
    let mut env_vars = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line?;
        let trimmed = line.trim();
        
        // Skip comments and empty lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let parsed = parse_env_variable(trimmed).with_context(|| format!("Failed to parse env variable on line {}: {}", line_num + 1, trimmed))?;
        env_vars.push(parsed);
    }

    Ok(env_vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_env_string() {
        // FOO=bar -> EnvVariable { name: "FOO", default: Some("bar") }
        let res = parse_env_string("FOO=bar").unwrap();
        assert_eq!(res, EnvDescriptor { name: "FOO".to_string(), default: Some("bar".to_string()) });

        // FOO="bar" -> EnvVariable { name: "FOO", default: Some("bar") }
        let res = parse_env_string("FOO=\"bar\"").unwrap();
        assert_eq!(res, EnvDescriptor { name: "FOO".to_string(), default: Some("bar".to_string()) });

        // FOO='bar' -> EnvVariable { name: "FOO", default: Some("bar") }
        let res = parse_env_string("FOO='bar'").unwrap();
        assert_eq!(res, EnvDescriptor { name: "FOO".to_string(), default: Some("bar".to_string()) });

        // FOO=bar=bar -> Error
        let res = parse_env_string("FOO=bar=bar");
        assert!(res.is_err());

        // FOO="bar=bar" -> EnvVariable { name: "FOO", default: Some("bar=bar") }
        let res = parse_env_string("FOO=\"bar=bar\"").unwrap();
        assert_eq!(res, EnvDescriptor { name: "FOO".to_string(), default: Some("bar=bar".to_string()) });

        // FOO -> EnvVariable { name: "FOO", default: None }
        let res = parse_env_string("FOO").unwrap();
        assert_eq!(res, EnvDescriptor { name: "FOO".to_string(), default: None });

        // Test whitespace trimming
        let res = parse_env_string("  FOO  =  bar  ").unwrap();
        assert_eq!(res, EnvDescriptor { name: "FOO".to_string(), default: Some("bar".to_string()) });
        
        // Empty
        assert!(parse_env_string("").is_err());
        assert!(parse_env_string("=").is_err()); // empty name
    }

    #[test]
    fn test_load_env_file() -> Result<()> {
        let mut file = NamedTempFile::new()?;
        writeln!(file, "FOO=bar")?;
        writeln!(file, "# Comment")?;
        writeln!(file, "")?;
        writeln!(file, "BAZ=\"qux\"")?;

        let vars = load_env_file(file.path())?;
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0], EnvVariable { name: "FOO".to_string(), value: "bar".to_string() });
        assert_eq!(vars[1], EnvVariable { name: "BAZ".to_string(), value: "qux".to_string() });

        Ok(())
    }

    #[test]
    fn test_load_env_file_error() -> Result<()> {
        let mut file = NamedTempFile::new()?;
        writeln!(file, "FOO")?; // No value, should error

        let res = load_env_file(file.path());
        assert!(res.is_err());
        Ok(())
    }
}
