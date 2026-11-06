//! What the private API reports beyond counts: a service's tasks and logs, the
//! gateway, and the swarm's nodes. Docker's records are turned into JSON here,
//! as pure functions, so the shapes are tested without Docker.

use bollard::models::{ContainerSummary, Node, Task};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

/// How many tasks of a service are reported, newest first: the running ones
/// and the recent failures, which say why a service is not converging.
pub const MAX_TASKS: usize = 20;

fn text<T: serde::Serialize>(value: &Option<T>) -> Option<String> {
    value
        .as_ref()
        .and_then(|v| serde_json::to_value(v).ok())
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.is_empty())
}

/// One Swarm task. `nodes` maps node IDs to hostnames.
pub fn task_json(task: &Task, nodes: &HashMap<String, String>) -> Value {
    let status = task.status.as_ref();
    let node = task
        .node_id
        .as_ref()
        .map(|id| nodes.get(id).cloned().unwrap_or_else(|| id.clone()));
    json!({
        "id": task.id.as_deref().map(|id| &id[..id.len().min(12)]),
        "slot": task.slot,
        "node": node,
        "state": status.and_then(|s| text(&s.state)),
        "desired_state": text(&task.desired_state),
        "since": status.and_then(|s| s.timestamp.clone()),
        "exit_code": status.and_then(|s| s.container_status.as_ref()).and_then(|c| c.exit_code),
        "error": status.and_then(|s| s.err.clone()).filter(|e| !e.is_empty()),
        "message": status.and_then(|s| s.message.clone()),
    })
}

/// A service's tasks, newest first, at most `MAX_TASKS`.
pub fn tasks_json(tasks: &[Task], nodes: &HashMap<String, String>) -> Vec<Value> {
    let mut sorted: Vec<&Task> = tasks.iter().collect();
    // Swarm's timestamps are RFC 3339 in UTC with a fixed width per controller, so
    // they order as text.
    sorted.sort_by(|a, b| {
        let at = |t: &Task| t.status.as_ref().and_then(|s| s.timestamp.clone()).unwrap_or_default();
        at(b).cmp(&at(a))
    });
    sorted
        .into_iter()
        .take(MAX_TASKS)
        .map(|t| task_json(t, nodes))
        .collect()
}

/// One standalone container, in the shape of a task where it has one.
pub fn container_json(container: &ContainerSummary) -> Value {
    json!({
        "id": container.id.as_deref().map(|id| &id[..id.len().min(12)]),
        "name": container.names.as_ref().and_then(|n| n.first()).map(|n| n.trim_start_matches('/').to_string()),
        "state": text(&container.state),
        "status": container.status,
        "health": container.health.as_ref().and_then(|h| text(&h.status)),
        "image": container.image,
    })
}

pub fn node_json(node: &Node) -> Value {
    let spec = node.spec.as_ref();
    json!({
        "id": node.id.as_deref().map(|id| &id[..id.len().min(12)]),
        "hostname": node.description.as_ref().and_then(|d| d.hostname.clone()),
        "role": spec.and_then(|s| text(&s.role)),
        "availability": spec.and_then(|s| text(&s.availability)),
        "state": node.status.as_ref().and_then(|s| text(&s.state)),
        "address": node.status.as_ref().and_then(|s| s.addr.clone()),
        "leader": node.manager_status.as_ref().and_then(|m| m.leader).unwrap_or(false),
        "labels": spec.and_then(|s| s.labels.clone()).unwrap_or_default(),
    })
}

pub fn hostnames(nodes: &[Node]) -> HashMap<String, String> {
    nodes
        .iter()
        .filter_map(|n| Some((n.id.clone()?, n.description.as_ref()?.hostname.clone()?)))
        .collect()
}

/// The query of the logs route.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LogQuery {
    /// Lines from the end; `None` for all of them.
    pub tail: Option<u32>,
    /// Unix seconds; 0 for no limit.
    pub since: i64,
    pub follow: bool,
    pub timestamps: bool,
}

/// Lines a request may ask for, and the default.
pub const MAX_TAIL: u32 = 10_000;
pub const DEFAULT_TAIL: u32 = 200;

/// `tail=200|all`, `since=10m|2h|30s|1d|<unix seconds>`, `follow=1`,
/// `timestamps=1`. `now` is Unix seconds.
pub fn log_query(params: &HashMap<String, String>, now: i64) -> Result<LogQuery, String> {
    let flag = |name: &str| {
        params
            .get(name)
            .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes"))
    };
    let tail = match params.get("tail").map(String::as_str) {
        None => Some(DEFAULT_TAIL),
        Some("all") => None,
        Some(n) => Some(
            n.parse::<u32>()
                .map_err(|_| format!("tail '{}' is not a number of lines or 'all'", n))?
                .min(MAX_TAIL),
        ),
    };
    let since = match params.get("since") {
        None => 0,
        Some(v) => match parse_duration(v) {
            Some(d) => now.saturating_sub(i64::try_from(d.as_secs()).unwrap_or(i64::MAX)),
            None => v
                .parse::<i64>()
                .map_err(|_| format!("since '{}' is neither a duration like 10m nor Unix seconds", v))?,
        },
    };
    Ok(LogQuery {
        tail,
        since,
        follow: flag("follow"),
        timestamps: flag("timestamps"),
    })
}

/// `30s`, `10m`, `2h`, `1d`. `None` for anything else, or a duration too long
/// to count in seconds.
pub fn parse_duration(value: &str) -> Option<Duration> {
    let unit = value.chars().last()?;
    let number = &value[..value.len() - unit.len_utf8()];
    let n: u64 = number.parse().ok()?;
    let per_unit = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86400,
        _ => return None,
    };
    Some(Duration::from_secs(n.checked_mul(per_unit)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{ContainerStatus, NodeDescription, NodeSpec, NodeSpecRoleEnum, TaskState, TaskStatus};

    fn task(id: &str, at: &str, state: TaskState, err: Option<&str>, exit: Option<i64>) -> Task {
        Task {
            id: Some(id.to_string()),
            node_id: Some("n1".to_string()),
            slot: Some(1),
            desired_state: Some(TaskState::RUNNING),
            status: Some(TaskStatus {
                timestamp: Some(at.to_string()),
                state: Some(state),
                err: err.map(str::to_string),
                message: Some("started".to_string()),
                container_status: Some(ContainerStatus {
                    exit_code: exit,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn tasks_are_newest_first_with_node_names_and_errors() {
        let nodes = HashMap::from([("n1".to_string(), "app-1".to_string())]);
        let tasks = [
            task(
                "aaaaaaaaaaaaaaaa",
                "2026-11-05T10:00:00.000000000Z",
                TaskState::FAILED,
                Some("exit 1"),
                Some(1),
            ),
            task(
                "bbbbbbbbbbbbbbbb",
                "2026-11-05T10:05:00.000000000Z",
                TaskState::RUNNING,
                None,
                Some(0),
            ),
        ];
        let out = tasks_json(&tasks, &nodes);
        assert_eq!(out[0]["id"], "bbbbbbbbbbbb");
        assert_eq!(out[0]["state"], "running");
        assert_eq!(out[0]["error"], Value::Null);
        assert_eq!(out[1]["node"], "app-1");
        assert_eq!(out[1]["state"], "failed");
        assert_eq!(out[1]["error"], "exit 1");
        assert_eq!(out[1]["exit_code"], 1);
    }

    #[test]
    fn a_node_reports_its_role_and_hostname() {
        let node = Node {
            id: Some("n1".to_string()),
            description: Some(NodeDescription {
                hostname: Some("app-1".to_string()),
                ..Default::default()
            }),
            spec: Some(NodeSpec {
                role: Some(NodeSpecRoleEnum::MANAGER),
                ..Default::default()
            }),
            ..Default::default()
        };
        let out = node_json(&node);
        assert_eq!(
            (out["hostname"].as_str(), out["role"].as_str()),
            (Some("app-1"), Some("manager"))
        );
        assert_eq!(hostnames(&[node]).get("n1").map(String::as_str), Some("app-1"));
    }

    #[test]
    fn log_queries_take_durations_and_limits() {
        let q = |pairs: &[(&str, &str)]| {
            log_query(
                &pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                1_000_000,
            )
        };
        assert_eq!(
            q(&[]).unwrap(),
            LogQuery {
                tail: Some(200),
                since: 0,
                follow: false,
                timestamps: false
            }
        );
        let full = q(&[
            ("tail", "all"),
            ("since", "10m"),
            ("follow", "1"),
            ("timestamps", "true"),
        ])
        .unwrap();
        assert_eq!(full.tail, None);
        assert_eq!(full.since, 1_000_000 - 600);
        assert!(full.follow && full.timestamps);
        assert_eq!(q(&[("tail", "999999")]).unwrap().tail, Some(MAX_TAIL));
        assert_eq!(q(&[("since", "1790000000")]).unwrap().since, 1_790_000_000);
        assert!(q(&[("tail", "lots")]).unwrap_err().contains("tail 'lots'"));
        assert!(q(&[("since", "yesterday")]).is_err());
        // A multi-byte last character, and a duration too long, are refused.
        assert!(q(&[("since", "5\u{f6}")]).is_err());
        assert!(q(&[("since", "999999999999999999d")]).is_err());
        assert_eq!(parse_duration("10\u{f6}"), None);
        assert_eq!(parse_duration(""), None);
    }
}
