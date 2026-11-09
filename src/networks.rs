//! Docker networks on Swarm and standalone Docker.
//!
//! Every deployment has a network of its own, the only one its services carry
//! their bare names on. A network declared in the env spec's `networks:` is
//! shared by the deployments that join it, and there a service answers only to
//! `<deployment>_<service>`, except that on Swarm `docker stack deploy` adds the
//! bare name on every network a service joins.
//!
//! Docker's DNS looks a name up network by network, in network-name order, and
//! answers with the first network that has it. An own network's name sorts before
//! every declared one (`-` before `_`), so a bare name always resolves inside the
//! asking deployment first.

/// The network every service was on before simpled 2.4. The gateway stays on it
/// so that deployments not yet prepared by 2.4 keep being routed.
pub const LEGACY: &str = "common_network";

pub fn own(deployment: &str) -> String {
    format!("simpled-{}", deployment)
}

pub fn declared(name: &str) -> String {
    format!("simpled_{}", name)
}

/// What Docker accepts as a network name, kept to the characters a deployment
/// name may have: letters, digits, `_`, `.` and `-`, starting with a letter or digit.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_own_network_sorts_before_every_declared_one() {
        for (deployment, network) in [("zz", "aa"), ("prod_app", "prod"), ("a", "0")] {
            assert!(own(deployment) < declared(network), "{deployment} / {network}");
        }
    }

    #[test]
    fn names_are_checked() {
        assert!(is_valid_name("app1-db"));
        assert!(is_valid_name("db.v2_x"));
        assert!(!is_valid_name("-db"));
        assert!(!is_valid_name("db net"));
        assert!(!is_valid_name(""));
    }
}
