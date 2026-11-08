//! Just enough of AWS for the controller to publish metrics and keep alarms:
//! Query API calls (CloudWatch, SNS) signed with Signature Version 4, and
//! credentials from the environment or the instance's role over IMDSv2. The AWS
//! SDK would bring several megabytes and a slower build for four calls.

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// The instance metadata service. `SIMPLED_IMDS_ENDPOINT` overrides it in tests.
const IMDS: &str = "http://169.254.169.254";
/// Credentials are renewed this long before they expire.
const RENEW_BEFORE_SECONDS: u64 = 300;

#[derive(Clone, Debug, PartialEq)]
pub struct Credentials {
    pub access_key: String,
    pub secret_key: String,
    pub token: Option<String>,
    /// Unix seconds; `None` for keys that do not expire.
    pub expires: Option<u64>,
}

pub struct Aws {
    http: reqwest::Client,
    imds: String,
    pub region: String,
    credentials: tokio::sync::Mutex<Option<Credentials>>,
}

/// Why a container may not reach the instance's credentials, for the error.
const HOP_LIMIT_HINT: &str = "In a container, IMDSv2 needs a hop limit of 2: aws ec2 modify-instance-metadata-options \
     --instance-id <id> --http-tokens required --http-put-response-hop-limit 2 --http-endpoint enabled";

impl Aws {
    /// The region is `region`, else `AWS_REGION` / `AWS_DEFAULT_REGION`, else the
    /// instance's own.
    pub async fn new(region: Option<String>) -> Result<Aws> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .context("Cannot set up the HTTP client")?;
        let imds = std::env::var("SIMPLED_IMDS_ENDPOINT").unwrap_or_else(|_| IMDS.to_string());
        let mut aws = Aws {
            http,
            imds,
            region: String::new(),
            credentials: tokio::sync::Mutex::new(None),
        };
        let from_env = || {
            ["AWS_REGION", "AWS_DEFAULT_REGION"]
                .iter()
                .find_map(|v| std::env::var(v).ok().filter(|r| !r.is_empty()))
        };
        aws.region = match region.or_else(from_env) {
            Some(region) => region,
            None => aws
                .imds_get("/latest/meta-data/placement/region")
                .await
                .context("No region is set (controller.cloudwatch.region) and the instance's cannot be read")?,
        };
        Ok(aws)
    }

    async fn imds_token(&self) -> Result<String> {
        let response = self
            .http
            .put(format!("{}/latest/api/token", self.imds))
            .header("X-aws-ec2-metadata-token-ttl-seconds", "21600")
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .with_context(|| format!("Cannot reach the instance metadata service. {}", HOP_LIMIT_HINT))?;
        if !response.status().is_success() {
            bail!(
                "The instance metadata service refused a token: HTTP {}",
                response.status()
            );
        }
        Ok(response.text().await?)
    }

    async fn imds_get(&self, path: &str) -> Result<String> {
        let token = self.imds_token().await?;
        let response = self
            .http
            .get(format!("{}{}", self.imds, path))
            .header("X-aws-ec2-metadata-token", token)
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .with_context(|| format!("Cannot read {} from the instance metadata service", path))?;
        if !response.status().is_success() {
            bail!(
                "The instance metadata service answered {} for {}",
                response.status(),
                path
            );
        }
        Ok(response.text().await?.trim().to_string())
    }

    /// `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` (and `AWS_SESSION_TOKEN`),
    /// else the instance role's, renewed before they expire.
    async fn credentials(&self) -> Result<Credentials> {
        let mut cached = self.credentials.lock().await;
        if let Some(c) = cached.as_ref() {
            if c.expires.is_none_or(|e| e > super::unix_now() + RENEW_BEFORE_SECONDS) {
                return Ok(c.clone());
            }
        }
        let fresh = match (
            std::env::var("AWS_ACCESS_KEY_ID"),
            std::env::var("AWS_SECRET_ACCESS_KEY"),
        ) {
            (Ok(access_key), Ok(secret_key)) if !access_key.is_empty() => Credentials {
                access_key,
                secret_key,
                token: std::env::var("AWS_SESSION_TOKEN").ok().filter(|t| !t.is_empty()),
                expires: None,
            },
            _ => {
                let base = "/latest/meta-data/iam/security-credentials/";
                let role = self.imds_get(base).await?;
                let role = role.lines().next().unwrap_or_default().trim().to_string();
                if role.is_empty() {
                    bail!("The instance has no IAM role: attach one with cloudwatch:PutMetricData (see the reference)");
                }
                let text = self.imds_get(&format!("{}{}", base, role)).await?;
                instance_credentials(&text).with_context(|| format!("Cannot read role {}'s credentials", role))?
            }
        };
        *cached = Some(fresh.clone());
        Ok(fresh)
    }

    /// A Query API action on `service` (`monitoring`, `sns`), returning the
    /// response's XML.
    pub async fn call(
        &self,
        service: &str,
        version: &str,
        action: &str,
        params: &[(String, String)],
    ) -> Result<String> {
        let credentials = self.credentials().await?;
        let host = format!("{}.{}.amazonaws.com", service, self.region);
        let mut all = vec![
            ("Action".to_string(), action.to_string()),
            ("Version".to_string(), version.to_string()),
        ];
        all.extend(params.iter().cloned());
        let body = form(&all);
        let date = amz_date(super::unix_now());
        let request = Signed {
            host: &host,
            region: &self.region,
            service,
            date: &date,
            body: &body,
            credentials: &credentials,
        };
        let mut builder = self
            .http
            .post(format!("https://{}/", host))
            .header("Content-Type", CONTENT_TYPE)
            .header("X-Amz-Date", &date)
            .header("Authorization", request.authorization());
        if let Some(token) = &credentials.token {
            builder = builder.header("X-Amz-Security-Token", token);
        }
        let response = builder
            .body(body)
            .send()
            .await
            .with_context(|| format!("{} {} failed", service, action))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let code = first(&text, "Code").unwrap_or_default();
            let message = first(&text, "Message").unwrap_or_default();
            bail!("{} {}: HTTP {} {}: {}", service, action, status.as_u16(), code, message);
        }
        Ok(text)
    }
}

/// The role credentials document IMDS serves.
fn instance_credentials(text: &str) -> Result<Credentials> {
    let doc: serde_json::Value = serde_json::from_str(text)?;
    let field = |name: &str| {
        doc[name]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("no {} in the credentials", name))
    };
    Ok(Credentials {
        access_key: field("AccessKeyId")?,
        secret_key: field("SecretAccessKey")?,
        token: field("Token").ok(),
        expires: field("Expiration").ok().and_then(|e| parse_utc(&e)),
    })
}

const CONTENT_TYPE: &str = "application/x-www-form-urlencoded; charset=utf-8";

/// One request's SigV4 signature.
struct Signed<'a> {
    host: &'a str,
    region: &'a str,
    service: &'a str,
    /// `YYYYMMDDTHHMMSSZ`.
    date: &'a str,
    body: &'a str,
    credentials: &'a Credentials,
}

impl Signed<'_> {
    fn authorization(&self) -> String {
        let (headers, signed) = match &self.credentials.token {
            Some(token) => (
                format!(
                    "content-type:{}\nhost:{}\nx-amz-date:{}\nx-amz-security-token:{}\n",
                    CONTENT_TYPE, self.host, self.date, token
                ),
                "content-type;host;x-amz-date;x-amz-security-token",
            ),
            None => (
                format!(
                    "content-type:{}\nhost:{}\nx-amz-date:{}\n",
                    CONTENT_TYPE, self.host, self.date
                ),
                "content-type;host;x-amz-date",
            ),
        };
        let canonical = format!(
            "POST\n/\n\n{}\n{}\n{}",
            headers,
            signed,
            hex(&Sha256::digest(self.body))
        );
        let day = &self.date[..8];
        let scope = format!("{}/{}/{}/aws4_request", day, self.region, self.service);
        let to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            self.date,
            scope,
            hex(&Sha256::digest(canonical.as_bytes()))
        );
        let key = signing_key(&self.credentials.secret_key, day, self.region, self.service);
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.credentials.access_key,
            scope,
            signed,
            hex(&hmac(&key, to_sign.as_bytes()))
        )
    }
}

fn signing_key(secret: &str, day: &str, region: &str, service: &str) -> [u8; 32] {
    let k_date = hmac(format!("AWS4{}", secret).as_bytes(), day.as_bytes());
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, service.as_bytes());
    hmac(&k_service, b"aws4_request")
}

/// HMAC-SHA256 (RFC 2104).
fn hmac(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|b| b ^ 0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(block.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// `application/x-www-form-urlencoded`, every byte but the unreserved ones
/// percent-encoded, as SigV4 expects.
fn form(params: &[(String, String)]) -> String {
    let encode = |s: &str| {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                _ => format!("%{:02X}", b),
            })
            .collect::<String>()
    };
    params
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// (year, month, day) to days since 1970-01-01.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let month = i64::from(month);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// `YYYYMMDDTHHMMSSZ`.
fn amz_date(unix: u64) -> String {
    let secs = unix as i64;
    let (year, month, day) = civil(secs.div_euclid(86400));
    let rest = secs.rem_euclid(86400);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        year,
        month,
        day,
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// `2026-11-08T14:00:00Z` to Unix seconds.
fn parse_utc(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| text.get(r)?.parse::<i64>().ok();
    let days = days_from_civil(n(0..4)?, n(5..7)? as u32, n(8..10)? as u32);
    u64::try_from(days * 86400 + n(11..13)? * 3600 + n(14..16)? * 60 + n(17..19)?).ok()
}

/// The text of every `<tag>...</tag>` in `xml`, unescaped. Enough for the flat
/// fields of a Query API response.
pub fn all(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        rest = &rest[start + open.len()..];
        let Some(end) = rest.find(&close) else { break };
        found.push(unescape(&rest[..end]));
        rest = &rest[end + close.len()..];
    }
    found
}

pub fn first(xml: &str, tag: &str) -> Option<String> {
    all(xml, tag).into_iter().next()
}

/// The `<member>` elements of a flat list, each as XML.
pub fn members(xml: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<member>") {
        rest = &rest[start + "<member>".len()..];
        let Some(end) = rest.find("</member>") else { break };
        found.push(&rest[..end]);
        rest = &rest[end + "</member>".len()..];
    }
    found
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test case 2.
        assert_eq!(
            hex(&hmac(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 6: a key longer than the block.
        assert_eq!(
            hex(&hmac(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn the_signing_key_matches_the_aws_example() {
        // "Examples of how to derive a signing key for Signature Version 4".
        let key = signing_key(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20120215",
            "us-east-1",
            "iam",
        );
        assert_eq!(
            hex(&key),
            "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d"
        );
    }

    #[test]
    fn dates_round_trip_in_both_formats() {
        assert_eq!(amz_date(0), "19700101T000000Z");
        assert_eq!(amz_date(1_787_400_000), "20260822T120000Z");
        assert_eq!(parse_utc("2026-08-22T12:00:00Z"), Some(1_787_400_000));
        assert_eq!(amz_date(951_782_400), "20000229T000000Z");
        assert_eq!(parse_utc("2000-02-29T00:00:00Z"), Some(951_782_400));
        assert_eq!(parse_utc("not a date"), None);
    }

    #[test]
    fn form_encodes_everything_but_unreserved_bytes() {
        let body = form(&[
            ("A.b_c~d-1".to_string(), "x y/z:é".to_string()),
            ("Email".to_string(), "ops+alerts@example.com".to_string()),
        ]);
        assert_eq!(body, "A.b_c~d-1=x%20y%2Fz%3A%C3%A9&Email=ops%2Balerts%40example.com");
    }

    #[test]
    fn the_authorization_header_names_the_scope_and_the_token() {
        let credentials = Credentials {
            access_key: "AKIDEXAMPLE".to_string(),
            secret_key: "secret".to_string(),
            token: Some("session".to_string()),
            expires: None,
        };
        let header = Signed {
            host: "monitoring.us-east-2.amazonaws.com",
            region: "us-east-2",
            service: "monitoring",
            date: "20261108T120000Z",
            body: "Action=PutMetricData",
            credentials: &credentials,
        }
        .authorization();
        assert!(header.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20261108/us-east-2/monitoring/aws4_request, \
             SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature="
        ));
        assert_eq!(header.rsplit('=').next().unwrap().len(), 64);
    }

    #[test]
    fn instance_credentials_expire_at_their_expiration() {
        let c = instance_credentials(
            r#"{"Code":"Success","AccessKeyId":"ASIA1","SecretAccessKey":"s","Token":"t","Expiration":"2026-08-22T12:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(c.access_key, "ASIA1");
        assert_eq!(c.token.as_deref(), Some("t"));
        assert_eq!(c.expires, Some(1_787_400_000));
    }

    #[test]
    fn xml_fields_and_members_are_read_and_unescaped() {
        let xml =
            "<R><member><Endpoint>a@x.com</Endpoint><SubscriptionArn>PendingConfirmation</SubscriptionArn></member>\
                   <member><Endpoint>b&amp;c@x.com</Endpoint></member><NextToken>n1</NextToken></R>";
        let m = members(xml);
        assert_eq!(m.len(), 2);
        assert_eq!(first(m[0], "SubscriptionArn").as_deref(), Some("PendingConfirmation"));
        assert_eq!(first(m[1], "Endpoint").as_deref(), Some("b&c@x.com"));
        assert_eq!(first(xml, "NextToken").as_deref(), Some("n1"));
        assert_eq!(all(xml, "Endpoint").len(), 2);
    }
}

/// Against real AWS, read-only: `SIMPLED_AWS_LIVE=1 cargo test aws_live -- --ignored`
/// with credentials and `AWS_REGION` in the environment.
#[cfg(test)]
mod live {
    #[test]
    #[ignore]
    fn aws_live_signed_read_only_calls_are_accepted() {
        if std::env::var("SIMPLED_AWS_LIVE").is_err() {
            return;
        }
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let aws = super::Aws::new(None).await.unwrap();
            let me = aws.call("sts", "2011-06-15", "GetCallerIdentity", &[]).await.unwrap();
            println!("sts: account {:?}", super::first(&me, "Account"));
            let params = [("AlarmNamePrefix".to_string(), "simpled:".to_string())];
            let alarms = aws
                .call("monitoring", "2010-08-01", "DescribeAlarms", &params)
                .await
                .unwrap();
            println!(
                "monitoring: {} simpled alarm(s)",
                super::all(&alarms, "AlarmName").len()
            );
            let topics = aws.call("sns", "2010-03-31", "ListTopics", &[]).await.unwrap();
            println!("sns: {} topic(s)", super::all(&topics, "TopicArn").len());
        });
    }
}
