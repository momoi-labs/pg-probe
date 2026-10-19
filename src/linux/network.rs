use std::io;
use std::path::Path;

use crate::collector::{Collector, Sink};
use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};
use crate::linux::identity::read_bounded;

const NETWORK_SOURCE: &str = "proc.net.dev";

/// Cumulative traffic counters for one network interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkEntry {
    pub interface: String,
    pub rx_bytes: u64,
    pub rx_packets: u64,
    pub rx_errors: u64,
    pub rx_dropped: u64,
    pub tx_bytes: u64,
    pub tx_packets: u64,
    pub tx_errors: u64,
    pub tx_dropped: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkObservations {
    pub interfaces: Vec<NetworkEntry>,
}

/// Interfaces left out: loopback and the virtual links that containers and
/// bridges create. They would drown the host's real traffic.
const SKIPPED_PREFIXES: &[&str] = &[
    "veth", "docker", "br-", "virbr", "cni", "flannel", "cali", "vxlan", "kube-",
];

/// Reads `/proc/net/dev`. Every value is a cumulative counter, so it is read
/// again in every sample.
pub struct NetworkCollector;

impl Collector for NetworkCollector {
    type Output = NetworkObservations;

    fn source(&self) -> &'static str {
        "linux.network"
    }

    fn collect(&self) -> (Self::Output, Vec<ComponentOutcome>) {
        let mut observations = NetworkObservations::default();
        let mut outcomes = Vec::new();
        let started_at_utc = now_utc();

        let content = match read_bounded(Path::new("/proc/net/dev")) {
            Ok(content) => content,
            Err(error) => {
                let status = match error.kind() {
                    io::ErrorKind::NotFound => CollectionStatus::Unsupported,
                    io::ErrorKind::PermissionDenied => CollectionStatus::PermissionDenied,
                    _ => CollectionStatus::IoError,
                };
                outcomes.push(
                    ComponentOutcome::failed(
                        NETWORK_SOURCE,
                        status,
                        "net_dev_read_failed",
                        error.raw_os_error(),
                    )
                    .with_timing(started_at_utc, now_utc()),
                );
                return (observations, outcomes);
            }
        };

        match parse_net_dev(&content) {
            Ok(interfaces) => {
                let observed_at_utc = now_utc();
                observations.interfaces = interfaces;
                outcomes.push(ComponentOutcome::collected(
                    NETWORK_SOURCE,
                    started_at_utc,
                    now_utc(),
                    observed_at_utc,
                ));
            }
            Err(error_code) => outcomes.push(
                ComponentOutcome::failed(
                    NETWORK_SOURCE,
                    CollectionStatus::ParseError,
                    error_code,
                    None,
                )
                .with_timing(started_at_utc, now_utc()),
            ),
        }
        (observations, outcomes)
    }

    fn write<S: Sink>(
        &self,
        sink: &mut S,
        output: &Self::Output,
        outcomes: &[ComponentOutcome],
    ) -> Result<(), S::Error> {
        sink.write_network(output, outcomes)
    }
}

/// An interface name is at most 15 bytes (IFNAMSIZ − 1) of printable ASCII.
fn valid_interface(name: &str) -> bool {
    !name.is_empty() && name.len() <= 15 && name.bytes().all(|byte| byte.is_ascii_graphic())
}

pub fn parse_net_dev(content: &str) -> Result<Vec<NetworkEntry>, &'static str> {
    let mut interfaces = Vec::new();
    // The first two lines are column headers.
    for line in content.lines().skip(2) {
        let Some((name, counters)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if !valid_interface(name) {
            return Err("invalid_interface_name");
        }
        if name == "lo"
            || SKIPPED_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
        {
            continue;
        }
        let fields = counters
            .split_whitespace()
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "invalid_net_dev_counter")?;
        if fields.len() < 16 {
            return Err("invalid_net_dev_format");
        }
        interfaces.push(NetworkEntry {
            interface: name.to_owned(),
            rx_bytes: fields[0],
            rx_packets: fields[1],
            rx_errors: fields[2],
            rx_dropped: fields[3],
            tx_bytes: fields[8],
            tx_packets: fields[9],
            tx_errors: fields[10],
            tx_dropped: fields[11],
        });
    }
    Ok(interfaces)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo:  1000      10    0    0    0     0          0         0     1000      10    0    0    0     0       0          0
  eth0: 52000     400    1    2    0     0          0         0    31000     300    0    3    0     0       0          0
veth1a2b:   10       1    0    0    0     0          0         0       10       1    0    0    0     0       0          0
";

    #[test]
    fn parse_net_dev_reads_counters_and_skips_virtual_links() {
        let interfaces = parse_net_dev(SAMPLE).unwrap();
        assert_eq!(interfaces.len(), 1);
        let eth0 = &interfaces[0];
        assert_eq!(eth0.interface, "eth0");
        assert_eq!(eth0.rx_bytes, 52000);
        assert_eq!(eth0.rx_dropped, 2);
        assert_eq!(eth0.tx_bytes, 31000);
        assert_eq!(eth0.tx_dropped, 3);
    }

    #[test]
    fn parse_net_dev_rejects_a_malformed_counter() {
        let bad = "h1\nh2\n  eth0: 1 2 x 4 5 6 7 8 9 10 11 12 13 14 15 16\n";
        assert_eq!(parse_net_dev(bad), Err("invalid_net_dev_counter"));
    }
}
