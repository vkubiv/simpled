//! `controller.cloudwatch`: every minute the controller publishes what
//! `/simpled/health` reports as CloudWatch metrics, and keeps the alarms on them
//! (and the SNS topic that emails them) as the env spec describes.
//!
//! The controller only reports what it sees, so the `silent` alarm treats
//! missing data as breaching: a dead server, Docker or controller, or a
//! controller that cannot reach CloudWatch, stops the metrics and fires it.

use super::aws::{self, Aws};
use super::model::Status;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

pub const NAMESPACE: &str = "Simpled";
const MONITORING: (&str, &str) = ("monitoring", "2010-08-01");
const SNS: (&str, &str) = ("sns", "2010-03-31");
/// How often the metrics are published, and the alarms' period.
const EVERY: Duration = Duration::from_secs(60);
/// How often the alarms and the topic are checked against the config.
const RECONCILE_EVERY: u64 = 6 * 3600;
/// After a failed reconcile.
const RECONCILE_RETRY: u64 = 300;
/// The certificate alarm fires below this many days left; renewal starts at 30.
const CERT_DAYS_ALARM: f64 = 14.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudWatchConfig {
    /// The `Env` dimension.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarm_email: Vec<String>,
    /// `/simpled/health` on the controller's host, for the alarms' descriptions.
    pub health_url: String,
}

/// One reading of the environment, as published.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// The worst of everything, deployments without services included.
    pub env: Status,
    pub gateway: Status,
    pub deployments: BTreeMap<String, Status>,
    /// The certificate closest to expiry: 0 for one not issued yet. `None`
    /// without certificates the controller issues.
    pub cert_days_left: Option<f64>,
}

fn level(status: Status) -> u8 {
    match status {
        Status::Healthy => 0,
        Status::Degraded => 1,
        Status::Down => 2,
    }
}

/// A metric's name, value and dimensions.
type Datum<'a> = (&'static str, f64, Vec<(&'static str, &'a str)>);

/// `PutMetricData`'s parameters for one sample.
pub fn metric_params(env: &str, sample: &Sample) -> Vec<(String, String)> {
    let mut metrics: Vec<Datum> = vec![
        ("Health", f64::from(level(sample.env)), vec![("Env", env)]),
        ("GatewayHealth", f64::from(level(sample.gateway)), vec![("Env", env)]),
    ];
    for (deployment, status) in &sample.deployments {
        metrics.push((
            "Health",
            f64::from(level(*status)),
            vec![("Env", env), ("Deployment", deployment.as_str())],
        ));
    }
    if let Some(days) = sample.cert_days_left {
        metrics.push(("CertDaysLeft", days, vec![("Env", env)]));
    }
    let mut params = vec![("Namespace".to_string(), NAMESPACE.to_string())];
    for (i, (name, value, dimensions)) in metrics.iter().enumerate() {
        let m = format!("MetricData.member.{}", i + 1);
        params.push((format!("{}.MetricName", m), name.to_string()));
        params.push((format!("{}.Value", m), format!("{}", value)));
        params.push((format!("{}.Unit", m), "None".to_string()));
        for (j, (k, v)) in dimensions.iter().enumerate() {
            params.push((format!("{}.Dimensions.member.{}.Name", m, j + 1), k.to_string()));
            params.push((format!("{}.Dimensions.member.{}.Value", m, j + 1), v.to_string()));
        }
    }
    params
}

/// One alarm the controller keeps.
#[derive(Debug, Clone, PartialEq)]
pub struct Alarm {
    pub name: String,
    pub description: String,
    pub metric: &'static str,
    pub deployment: Option<String>,
    pub statistic: &'static str,
    pub period: u64,
    /// Datapoints, all of which must breach.
    pub minutes: u64,
    pub comparison: &'static str,
    pub threshold: f64,
    pub missing: &'static str,
}

/// Every alarm of this environment starts with this; the controller deletes
/// only alarms under it. A name holds no `:`, so `simpled:prod:` cannot be
/// the start of another environment's alarms.
pub fn prefix(env: &str) -> String {
    format!("simpled:{}:", env)
}

pub fn topic_name(env: &str) -> String {
    // SNS topic names allow letters, digits, `-` and `_`.
    format!("simpled-{}-health", env.replace('.', "-"))
}

/// The alarms for the environment's deployments; `certificates` adds the
/// certificate alarm.
pub fn alarms(config: &CloudWatchConfig, deployments: &[String], certificates: bool) -> Vec<Alarm> {
    let env = &config.name;
    let p = prefix(env);
    let url = &config.health_url;
    let alarm =
        |name: String, description: String, metric, deployment, comparison, threshold, minutes, missing| Alarm {
            name,
            description,
            metric,
            deployment,
            statistic: "Maximum",
            period: EVERY.as_secs(),
            minutes,
            comparison,
            threshold,
            missing,
        };
    let mut list = vec![
        // Never breaches on data (Health is at most 2): only when there is none.
        alarm(
            format!("{}silent", p),
            format!(
                "The simpled controller of {} has not reported for 3 minutes: the server, Docker or the \
                 controller is down, or it cannot reach CloudWatch. {}",
                env, url
            ),
            "Health",
            None,
            "GreaterThanThreshold",
            2.0,
            3,
            "breaching",
        ),
        alarm(
            format!("{}unhealthy", p),
            format!(
                "{} has not been healthy for 15 minutes: a stuck rollout, missing replicas, a certificate, or \
                 something down. {}",
                env, url
            ),
            "Health",
            None,
            "GreaterThanOrEqualToThreshold",
            1.0,
            15,
            "notBreaching",
        ),
        alarm(
            format!("{}gateway", p),
            format!(
                "The gateway of {} is down: no task or container is running. {}",
                env, url
            ),
            "GatewayHealth",
            None,
            "GreaterThanOrEqualToThreshold",
            2.0,
            2,
            "notBreaching",
        ),
    ];
    for deployment in deployments {
        list.push(alarm(
            format!("{}deployment:{}", p, deployment),
            format!(
                "Deployment {} of {} is down: a service has no running task. {}/{}",
                deployment, env, url, deployment
            ),
            "Health",
            Some(deployment.clone()),
            "GreaterThanOrEqualToThreshold",
            2.0,
            3,
            "notBreaching",
        ));
    }
    if certificates {
        list.push(Alarm {
            name: format!("{}certificate", p),
            description: format!(
                "A certificate of {} has had under {} days left, or none issued, for an hour: renewal is failing. {}",
                env, CERT_DAYS_ALARM, url
            ),
            metric: "CertDaysLeft",
            deployment: None,
            statistic: "Minimum",
            period: 300,
            minutes: 12,
            comparison: "LessThanThreshold",
            threshold: CERT_DAYS_ALARM,
            missing: "notBreaching",
        });
    }
    list
}

/// `PutMetricAlarm`'s parameters, with `topic` as its alarm and OK action.
pub fn alarm_params(env: &str, alarm: &Alarm, topic: Option<&str>) -> Vec<(String, String)> {
    let mut params: Vec<(String, String)> = [
        ("AlarmName", alarm.name.clone()),
        ("AlarmDescription", alarm.description.clone()),
        ("Namespace", NAMESPACE.to_string()),
        ("MetricName", alarm.metric.to_string()),
        ("Statistic", alarm.statistic.to_string()),
        ("Period", alarm.period.to_string()),
        ("EvaluationPeriods", alarm.minutes.to_string()),
        ("DatapointsToAlarm", alarm.minutes.to_string()),
        ("Threshold", alarm.threshold.to_string()),
        ("ComparisonOperator", alarm.comparison.to_string()),
        ("TreatMissingData", alarm.missing.to_string()),
        ("Dimensions.member.1.Name", "Env".to_string()),
        ("Dimensions.member.1.Value", env.to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    if let Some(deployment) = &alarm.deployment {
        params.push(("Dimensions.member.2.Name".to_string(), "Deployment".to_string()));
        params.push(("Dimensions.member.2.Value".to_string(), deployment.clone()));
    }
    if let Some(topic) = topic {
        params.push(("AlarmActions.member.1".to_string(), topic.to_string()));
        params.push(("OKActions.member.1".to_string(), topic.to_string()));
    }
    params
}

/// What reconciling changed, for the log.
#[derive(Debug, Default, PartialEq)]
pub struct Reconciled {
    pub alarms: usize,
    pub deleted: Vec<String>,
    pub subscribed: Vec<String>,
    pub unsubscribed: Vec<String>,
}

/// Brings the topic, its email subscriptions and the alarms in line with the
/// config. Every call is idempotent; only alarms under `prefix` are deleted,
/// and only email subscriptions of the controller's own topic are removed.
async fn reconcile(aws: &Aws, config: &CloudWatchConfig, wanted: &[Alarm]) -> Result<Reconciled> {
    let mut done = Reconciled::default();
    let topic = if config.alarm_email.is_empty() {
        None
    } else {
        let xml = aws
            .call(
                SNS.0,
                SNS.1,
                "CreateTopic",
                &[("Name".to_string(), topic_name(&config.name))],
            )
            .await?;
        let arn = aws::first(&xml, "TopicArn").context("CreateTopic returned no TopicArn")?;
        // Endpoint to subscription ARN; `PendingConfirmation` until the email is confirmed.
        let mut subscribed: BTreeMap<String, String> = BTreeMap::new();
        let mut next: Option<String> = None;
        loop {
            let mut params = vec![("TopicArn".to_string(), arn.clone())];
            if let Some(token) = &next {
                params.push(("NextToken".to_string(), token.clone()));
            }
            let xml = aws.call(SNS.0, SNS.1, "ListSubscriptionsByTopic", &params).await?;
            for member in aws::members(&xml) {
                if aws::first(member, "Protocol").as_deref() == Some("email") {
                    if let (Some(endpoint), Some(sub)) =
                        (aws::first(member, "Endpoint"), aws::first(member, "SubscriptionArn"))
                    {
                        subscribed.insert(endpoint.to_lowercase(), sub);
                    }
                }
            }
            next = aws::first(&xml, "NextToken").filter(|t| !t.is_empty());
            if next.is_none() {
                break;
            }
        }
        let wanted_emails: BTreeSet<String> = config.alarm_email.iter().map(|e| e.to_lowercase()).collect();
        for email in &config.alarm_email {
            if !subscribed.contains_key(&email.to_lowercase()) {
                let params = [
                    ("TopicArn".to_string(), arn.clone()),
                    ("Protocol".to_string(), "email".to_string()),
                    ("Endpoint".to_string(), email.clone()),
                ];
                aws.call(SNS.0, SNS.1, "Subscribe", &params).await?;
                done.subscribed.push(email.clone());
            }
        }
        for (endpoint, sub) in &subscribed {
            // A pending subscription has no ARN to remove; it lapses in 3 days.
            if !wanted_emails.contains(endpoint) && sub.starts_with("arn:") {
                aws.call(
                    SNS.0,
                    SNS.1,
                    "Unsubscribe",
                    &[("SubscriptionArn".to_string(), sub.clone())],
                )
                .await?;
                done.unsubscribed.push(endpoint.clone());
            }
        }
        Some(arn)
    };

    for alarm in wanted {
        aws.call(
            MONITORING.0,
            MONITORING.1,
            "PutMetricAlarm",
            &alarm_params(&config.name, alarm, topic.as_deref()),
        )
        .await?;
    }
    done.alarms = wanted.len();

    let prefix = prefix(&config.name);
    let mut existing = Vec::new();
    let mut next: Option<String> = None;
    loop {
        let mut params = vec![
            ("AlarmNamePrefix".to_string(), prefix.clone()),
            ("AlarmTypes.member.1".to_string(), "MetricAlarm".to_string()),
        ];
        if let Some(token) = &next {
            params.push(("NextToken".to_string(), token.clone()));
        }
        let xml = aws.call(MONITORING.0, MONITORING.1, "DescribeAlarms", &params).await?;
        existing.extend(aws::all(&xml, "AlarmName"));
        next = aws::first(&xml, "NextToken").filter(|t| !t.is_empty());
        if next.is_none() {
            break;
        }
    }
    let stale = stale_alarms(&prefix, &existing, wanted);
    // DeleteAlarms takes up to 100 names.
    for chunk in stale.chunks(100) {
        let params: Vec<(String, String)> = chunk
            .iter()
            .enumerate()
            .map(|(i, name)| (format!("AlarmNames.member.{}", i + 1), name.clone()))
            .collect();
        aws.call(MONITORING.0, MONITORING.1, "DeleteAlarms", &params).await?;
    }
    done.deleted = stale;
    Ok(done)
}

/// The alarms under `prefix` that are no longer wanted: a deployment that left
/// the env spec, or the certificate alarm once there are no certificates.
pub fn stale_alarms(prefix: &str, existing: &[String], wanted: &[Alarm]) -> Vec<String> {
    let wanted: BTreeSet<&str> = wanted.iter().map(|a| a.name.as_str()).collect();
    existing
        .iter()
        .filter(|name| name.starts_with(prefix) && !wanted.contains(name.as_str()))
        .cloned()
        .collect()
}

/// The controller's CloudWatch task: never returns.
pub async fn run(config: CloudWatchConfig, app: Arc<super::http::App>) {
    let deployments = app.deployments.clone();
    let certificates = app.certs.is_some();
    let wanted = alarms(&config, &deployments, certificates);
    let aws = loop {
        match Aws::new(config.region.clone()).await {
            Ok(aws) => break aws,
            Err(e) => {
                eprintln!("cloudwatch: {:#}; retrying in a minute", e);
                tokio::time::sleep(EVERY).await;
            }
        }
    };
    eprintln!(
        "cloudwatch: publishing {}'s health to namespace {} in {}",
        config.name, NAMESPACE, aws.region
    );
    let mut reconcile_at = 0;
    // Logged when it changes, not every minute.
    let mut last_error: Option<String> = None;
    let mut interval = tokio::time::interval(EVERY);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let now = super::unix_now();
        if now >= reconcile_at {
            match reconcile(&aws, &config, &wanted).await {
                Ok(done) => {
                    eprintln!(
                        "cloudwatch: {} alarm(s) up to date{}{}{}{}",
                        done.alarms,
                        if config.alarm_email.is_empty() {
                            ", without email (no alarm_email)".to_string()
                        } else {
                            format!(", emailing topic {}", topic_name(&config.name))
                        },
                        list("; deleted", &done.deleted),
                        list("; subscribed (each must confirm the email AWS sends)", &done.subscribed),
                        list("; unsubscribed", &done.unsubscribed),
                    );
                    reconcile_at = now + RECONCILE_EVERY;
                }
                Err(e) => {
                    eprintln!("cloudwatch: cannot update the alarms: {:#}", e);
                    reconcile_at = now + RECONCILE_RETRY;
                }
            }
        }
        let result = match super::http::sample(&app).await {
            // Nothing is published when Docker cannot be read: the silent alarm says so.
            Err(e) => Err(e.context("cannot read the environment, nothing published")),
            Ok(sample) => aws
                .call(
                    MONITORING.0,
                    MONITORING.1,
                    "PutMetricData",
                    &metric_params(&config.name, &sample),
                )
                .await
                .map(|_| ()),
        };
        match (result, &last_error) {
            (Ok(()), Some(_)) => {
                eprintln!("cloudwatch: publishing again");
                last_error = None;
            }
            (Ok(()), None) => {}
            (Err(e), _) => {
                let message = format!("{:#}", e);
                if last_error.as_ref() != Some(&message) {
                    eprintln!("cloudwatch: {}", message);
                    last_error = Some(message);
                }
            }
        }
    }
}

fn list(label: &str, items: &[String]) -> String {
    if items.is_empty() {
        String::new()
    } else {
        format!("{} {}", label, items.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(emails: &[&str]) -> CloudWatchConfig {
        CloudWatchConfig {
            name: "prod".to_string(),
            region: None,
            alarm_email: emails.iter().map(|e| e.to_string()).collect(),
            health_url: "https://ops.example.com/simpled/health".to_string(),
        }
    }

    fn param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
        params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn a_sample_is_one_metric_per_part_with_the_env_dimension() {
        let sample = Sample {
            env: Status::Degraded,
            gateway: Status::Healthy,
            deployments: [
                ("api".to_string(), Status::Degraded),
                ("blog".to_string(), Status::Down),
            ]
            .into(),
            cert_days_left: Some(42.5),
        };
        let p = metric_params("prod", &sample);
        assert_eq!(param(&p, "Namespace"), Some("Simpled"));
        assert_eq!(param(&p, "MetricData.member.1.MetricName"), Some("Health"));
        assert_eq!(param(&p, "MetricData.member.1.Value"), Some("1"));
        assert_eq!(param(&p, "MetricData.member.1.Dimensions.member.1.Value"), Some("prod"));
        assert_eq!(param(&p, "MetricData.member.2.MetricName"), Some("GatewayHealth"));
        assert_eq!(
            param(&p, "MetricData.member.4.Dimensions.member.2.Name"),
            Some("Deployment")
        );
        assert_eq!(param(&p, "MetricData.member.4.Dimensions.member.2.Value"), Some("blog"));
        assert_eq!(param(&p, "MetricData.member.4.Value"), Some("2"));
        assert_eq!(param(&p, "MetricData.member.5.MetricName"), Some("CertDaysLeft"));
        assert_eq!(param(&p, "MetricData.member.5.Value"), Some("42.5"));
        assert_eq!(param(&p, "MetricData.member.6.MetricName"), None);
    }

    #[test]
    fn the_alarms_cover_silence_health_the_gateway_each_deployment_and_certificates() {
        let names = |certs| {
            alarms(&config(&[]), &["api".to_string(), "blog".to_string()], certs)
                .into_iter()
                .map(|a| a.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(true),
            [
                "simpled:prod:silent",
                "simpled:prod:unhealthy",
                "simpled:prod:gateway",
                "simpled:prod:deployment:api",
                "simpled:prod:deployment:blog",
                "simpled:prod:certificate",
            ]
        );
        assert!(!names(false).contains(&"simpled:prod:certificate".to_string()));
    }

    #[test]
    fn only_silence_breaches_on_missing_data() {
        let all = alarms(&config(&[]), &["api".to_string()], true);
        let silent = &all[0];
        assert_eq!(
            (silent.missing, silent.comparison, silent.threshold),
            ("breaching", "GreaterThanThreshold", 2.0)
        );
        assert!(all[1..].iter().all(|a| a.missing == "notBreaching"));
    }

    #[test]
    fn alarm_params_carry_the_dimensions_and_the_topic_both_ways() {
        let all = alarms(&config(&["ops@example.com"]), &["api".to_string()], false);
        let deployment = all.iter().find(|a| a.deployment.is_some()).unwrap();
        let p = alarm_params("prod", deployment, Some("arn:aws:sns:us-east-2:1:simpled-prod-health"));
        assert_eq!(param(&p, "Dimensions.member.2.Value"), Some("api"));
        assert_eq!(param(&p, "EvaluationPeriods"), Some("3"));
        assert_eq!(param(&p, "DatapointsToAlarm"), Some("3"));
        assert_eq!(param(&p, "AlarmActions.member.1"), param(&p, "OKActions.member.1"));
        assert!(param(&p, "AlarmDescription").unwrap().ends_with("/simpled/health/api"));
        let without = alarm_params("prod", deployment, None);
        assert_eq!(param(&without, "AlarmActions.member.1"), None);
    }

    #[test]
    fn only_this_environments_unwanted_alarms_are_stale() {
        let wanted = alarms(&config(&[]), &["api".to_string()], false);
        let existing: Vec<String> = [
            "simpled:prod:silent",
            "simpled:prod:deployment:old",
            "simpled:prod:certificate",
            "simpled:prod-eu:silent",
            "someone-elses-alarm",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            stale_alarms(&prefix("prod"), &existing, &wanted),
            ["simpled:prod:deployment:old", "simpled:prod:certificate"]
        );
    }

    #[test]
    fn the_topic_name_is_one_sns_accepts() {
        assert_eq!(topic_name("hobby.prod"), "simpled-hobby-prod-health");
    }
}
