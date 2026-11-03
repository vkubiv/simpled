//! Kubernetes-style `resources` translated for Docker targets: Swarm's
//! `deploy.resources`, the matching `docker service create` flags for jobs, and
//! `docker run` flags. Only what the env spec sets is carried; an unset field
//! leaves the container unconstrained instead of taking the Kubernetes default.

use crate::spec::{ResourceLimits, ResourcesSpec};
use anyhow::{anyhow, Result};
use serde::Serialize;

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct DockerResources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<DockerResourceValues>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reservations: Option<DockerResourceValues>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct DockerResourceValues {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpus: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

impl DockerResources {
    /// `None` when the spec sets no request or limit at all. `owner` starts any
    /// error message, e.g. "Service 'api'".
    pub fn from_spec(resources: &ResourcesSpec, owner: &str) -> Result<Option<Self>> {
        let limits = values(&resources.limits, owner, "limits")?;
        let reservations = values(&resources.requests, owner, "requests")?;

        if let (Some(request), Some(limit)) = (&resources.requests.memory, &resources.limits.memory) {
            if memory_bytes(request)? > memory_bytes(limit)? {
                return Err(anyhow!(
                    "{}: resources.requests.memory '{}' is larger than resources.limits.memory '{}'",
                    owner,
                    request,
                    limit
                ));
            }
        }

        if limits.is_none() && reservations.is_none() {
            return Ok(None);
        }
        Ok(Some(DockerResources { limits, reservations }))
    }

    pub fn limits(&self) -> DockerResourceValues {
        self.limits.clone().unwrap_or_default()
    }

    pub fn reservations(&self) -> DockerResourceValues {
        self.reservations.clone().unwrap_or_default()
    }
}

fn values(limits: &ResourceLimits, owner: &str, field: &str) -> Result<Option<DockerResourceValues>> {
    let invalid = |kind: &str, value: &str, e: anyhow::Error| {
        anyhow!("{}: resources.{}.{} '{}': {}", owner, field, kind, value, e)
    };
    let memory = limits
        .memory
        .as_deref()
        .map(|m| memory_bytes(m).map(docker_memory).map_err(|e| invalid("memory", m, e)))
        .transpose()?;
    let cpus = limits
        .cpu
        .as_deref()
        .map(|c| docker_cpus(c).map_err(|e| invalid("cpu", c, e)))
        .transpose()?;

    if memory.is_none() && cpus.is_none() {
        return Ok(None);
    }
    Ok(Some(DockerResourceValues { cpus, memory }))
}

/// Bytes in a Kubernetes memory quantity: plain bytes, a decimal suffix
/// (`k`, `M`, `G`, `T`, `P`, `E`) or a binary one (`Ki` ... `Ei`). A fraction
/// rounds up to the next byte, as Kubernetes does.
pub fn memory_bytes(quantity: &str) -> Result<u64> {
    const SUFFIXES: [(&str, u128); 12] = [
        ("Ki", 1 << 10),
        ("Mi", 1 << 20),
        ("Gi", 1 << 30),
        ("Ti", 1 << 40),
        ("Pi", 1 << 50),
        ("Ei", 1 << 60),
        ("k", 1_000),
        ("M", 1_000_000),
        ("G", 1_000_000_000),
        ("T", 1_000_000_000_000),
        ("P", 1_000_000_000_000_000),
        ("E", 1_000_000_000_000_000_000),
    ];
    let quantity = quantity.trim();
    let (number, multiplier) = SUFFIXES
        .iter()
        .find_map(|(suffix, m)| quantity.strip_suffix(suffix).map(|n| (n, *m)))
        .unwrap_or((quantity, 1));

    let (whole, fraction) = decimal(number).ok_or_else(|| {
        anyhow!("not a memory quantity; use bytes or a Ki/Mi/Gi (binary) or k/M/G (decimal) suffix, e.g. 512Mi")
    })?;
    let scale = 10u128.pow(fraction.len() as u32);
    let fraction: u128 = if fraction.is_empty() { 0 } else { fraction.parse()? };
    let bytes = whole
        .checked_mul(multiplier)
        .and_then(|w| w.checked_add((fraction * multiplier).div_ceil(scale)))
        .filter(|b| *b <= u64::MAX as u128)
        .ok_or_else(|| anyhow!("memory quantity is too large"))?;
    if bytes == 0 {
        return Err(anyhow!("memory must be larger than zero"));
    }
    Ok(bytes as u64)
}

/// Docker reads `k`/`m`/`g` as binary units, so whole binary multiples keep the
/// suffix and anything else is written in bytes.
fn docker_memory(bytes: u64) -> String {
    for (suffix, unit) in [("g", 1u64 << 30), ("m", 1 << 20), ("k", 1 << 10)] {
        if bytes.is_multiple_of(unit) {
            return format!("{}{}", bytes / unit, suffix);
        }
    }
    bytes.to_string()
}

/// A Kubernetes CPU quantity (`500m`, `0.5`, `2`) as Docker's decimal CPU count.
pub fn docker_cpus(quantity: &str) -> Result<String> {
    let quantity = quantity.trim();
    let invalid = || anyhow!("not a CPU quantity; use cores (0.5, 2) or millicores (500m)");
    let millicores: u128 = match quantity.strip_suffix('m') {
        Some(milli) => {
            let (whole, fraction) = decimal(milli).ok_or_else(invalid)?;
            if !fraction.is_empty() {
                return Err(anyhow!("millicores cannot have a fraction"));
            }
            whole
        }
        None => {
            let (whole, fraction) = decimal(quantity).ok_or_else(invalid)?;
            if fraction.len() > 3 {
                return Err(anyhow!("CPU is limited to millicore precision (three decimals)"));
            }
            let fraction: u128 = format!("{:0<3}", fraction).parse()?;
            whole
                .checked_mul(1000)
                .and_then(|w| w.checked_add(fraction))
                .ok_or_else(invalid)?
        }
    };
    if millicores == 0 {
        return Err(anyhow!("CPU must be larger than zero"));
    }
    let (whole, milli) = (millicores / 1000, millicores % 1000);
    if milli == 0 {
        return Ok(whole.to_string());
    }
    let fraction = format!("{:03}", milli);
    Ok(format!("{}.{}", whole, fraction.trim_end_matches('0')))
}

/// Splits `12.5` into `(12, "5")`. Digits only, at most one dot, digits on at
/// least one side of it.
fn decimal(number: &str) -> Option<(u128, &str)> {
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty()) || !digits(whole) || !digits(fraction) {
        return None;
    }
    let whole = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    Some((whole, fraction))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(memory: Option<&str>, cpu: Option<&str>) -> ResourceLimits {
        ResourceLimits {
            memory: memory.map(String::from),
            cpu: cpu.map(String::from),
        }
    }

    fn spec(requests: ResourceLimits, limits: ResourceLimits) -> ResourcesSpec {
        ResourcesSpec {
            replicas: 1,
            requests,
            limits,
        }
    }

    #[test]
    fn memory_converts_to_the_exact_byte_count() {
        let convert = |q: &str| docker_memory(memory_bytes(q).unwrap());
        assert_eq!(convert("512Mi"), "512m");
        assert_eq!(convert("1Gi"), "1g");
        assert_eq!(convert("1024Mi"), "1g");
        assert_eq!(convert("64Ki"), "64k");
        assert_eq!(convert("1.5Gi"), "1536m");
        assert_eq!(convert("512M"), "500000k");
        assert_eq!(convert("1G"), "1000000000");
        assert_eq!(convert("100k"), "100000");
        assert_eq!(convert("1048576"), "1m");
        assert_eq!(convert("1000"), "1000");
    }

    #[test]
    fn unparseable_memory_is_rejected() {
        for q in ["", "Mi", "1GB", "1gi", "512 MB", "-1Gi", "1.2.3Gi", "0", "0Mi", "abc"] {
            assert!(memory_bytes(q).is_err(), "{:?} should be rejected", q);
        }
    }

    #[test]
    fn cpu_becomes_a_decimal_core_count() {
        assert_eq!(docker_cpus("500m").unwrap(), "0.5");
        assert_eq!(docker_cpus("250m").unwrap(), "0.25");
        assert_eq!(docker_cpus("1m").unwrap(), "0.001");
        assert_eq!(docker_cpus("1500m").unwrap(), "1.5");
        assert_eq!(docker_cpus("2000m").unwrap(), "2");
        assert_eq!(docker_cpus("2").unwrap(), "2");
        assert_eq!(docker_cpus("0.5").unwrap(), "0.5");
        assert_eq!(docker_cpus(".25").unwrap(), "0.25");
        assert_eq!(docker_cpus("1.50").unwrap(), "1.5");
    }

    #[test]
    fn unparseable_cpu_is_rejected() {
        for q in ["", "m", "half", "0.5m", "1.0001", "0", "0m", "-1", "2 cores", "1e3"] {
            assert!(docker_cpus(q).is_err(), "{:?} should be rejected", q);
        }
    }

    #[test]
    fn nothing_set_means_no_resources() {
        let res = spec(ResourceLimits::default(), ResourceLimits::default());
        assert_eq!(DockerResources::from_spec(&res, "Service 'api'").unwrap(), None);
    }

    #[test]
    fn requests_become_reservations_and_only_set_fields_are_carried() {
        let res = spec(limits(Some("256Mi"), None), limits(Some("1Gi"), Some("500m")));
        let docker = DockerResources::from_spec(&res, "Service 'api'").unwrap().unwrap();
        assert_eq!(
            docker.limits,
            Some(DockerResourceValues {
                cpus: Some("0.5".into()),
                memory: Some("1g".into()),
            })
        );
        assert_eq!(
            docker.reservations,
            Some(DockerResourceValues {
                cpus: None,
                memory: Some("256m".into()),
            })
        );
    }

    #[test]
    fn errors_name_the_service_and_field() {
        let res = spec(ResourceLimits::default(), limits(Some("1GB"), None));
        let err = DockerResources::from_spec(&res, "Service 'web'")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Service 'web': resources.limits.memory '1GB'"), "{}", err);
    }

    #[test]
    fn a_reservation_above_the_limit_is_rejected() {
        let res = spec(limits(Some("2Gi"), None), limits(Some("1Gi"), None));
        let err = DockerResources::from_spec(&res, "Service 'web'")
            .unwrap_err()
            .to_string();
        assert!(err.contains("larger than resources.limits.memory"), "{}", err);
    }
}
