use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::contract::{now_utc, CollectionStatus, ComponentOutcome};

const RLIMIT_SOURCE: &str = "proc.rlimit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceLimitKind {
    Nofile,
    Nproc,
    Memlock,
    AddressSpace,
    Core,
    FileSize,
    Stack,
}

impl ResourceLimitKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nofile => "nofile",
            Self::Nproc => "nproc",
            Self::Memlock => "memlock",
            Self::AddressSpace => "address_space",
            Self::Core => "core",
            Self::FileSize => "file_size",
            Self::Stack => "stack",
        }
    }

    pub const fn unit(self) -> &'static str {
        match self {
            Self::Nofile | Self::Nproc => "count",
            Self::Memlock | Self::AddressSpace | Self::Core | Self::FileSize | Self::Stack => {
                "bytes"
            }
        }
    }

    pub const fn resource(self) -> libc::c_int {
        match self {
            Self::Nofile => libc::RLIMIT_NOFILE as libc::c_int,
            Self::Nproc => libc::RLIMIT_NPROC as libc::c_int,
            Self::Memlock => libc::RLIMIT_MEMLOCK as libc::c_int,
            Self::AddressSpace => libc::RLIMIT_AS as libc::c_int,
            Self::Core => libc::RLIMIT_CORE as libc::c_int,
            Self::FileSize => libc::RLIMIT_FSIZE as libc::c_int,
            Self::Stack => libc::RLIMIT_STACK as libc::c_int,
        }
    }
}

const ALL_LIMITS: &[ResourceLimitKind] = &[
    ResourceLimitKind::Nofile,
    ResourceLimitKind::Nproc,
    ResourceLimitKind::Memlock,
    ResourceLimitKind::AddressSpace,
    ResourceLimitKind::Core,
    ResourceLimitKind::FileSize,
    ResourceLimitKind::Stack,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLimitEntry {
    pub resource: ResourceLimitKind,
    pub soft: Option<u64>,
    pub hard: Option<u64>,
    pub soft_unlimited: bool,
    pub hard_unlimited: bool,
    pub unit: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LimitsObservations {
    pub limits: Vec<ResourceLimitEntry>,
}

pub fn collect_limits_isolated() -> (LimitsObservations, Vec<ComponentOutcome>) {
    catch_unwind(AssertUnwindSafe(collect_limits)).unwrap_or_else(|_| {
        (
            LimitsObservations::default(),
            vec![ComponentOutcome::failed(
                "linux.limits",
                CollectionStatus::IoError,
                "collector_panicked",
                None,
            )],
        )
    })
}

fn collect_limits() -> (LimitsObservations, Vec<ComponentOutcome>) {
    let mut observations = LimitsObservations::default();
    let mut outcomes = Vec::new();

    let started_at_utc = now_utc();

    let mut limits = Vec::new();
    let mut had_error = false;

    for &kind in ALL_LIMITS {
        match getrlimit(kind) {
            Ok(entry) => limits.push(entry),
            Err(_) => {
                had_error = true;
            }
        }
    }

    if had_error && limits.is_empty() {
        outcomes.push(
            ComponentOutcome::failed(
                RLIMIT_SOURCE,
                CollectionStatus::IoError,
                "getrlimit_failed",
                None,
            )
            .with_timing(started_at_utc, now_utc()),
        );
        return (observations, outcomes);
    }

    let observed_at_utc = now_utc();
    observations.limits = limits;
    outcomes.push(ComponentOutcome::collected(
        RLIMIT_SOURCE,
        started_at_utc,
        now_utc(),
        observed_at_utc,
    ));

    (observations, outcomes)
}

fn getrlimit(kind: ResourceLimitKind) -> Result<ResourceLimitEntry, ()> {
    let mut rlim: libc::rlimit = unsafe { std::mem::zeroed() };
    // SAFETY: getrlimit writes into rlim, a plain C struct of the expected type.
    if unsafe { libc::getrlimit(kind.resource() as _, &mut rlim) } != 0 {
        return Err(());
    }

    const RLIM_INFINITY_VAL: u64 = libc::RLIM_INFINITY;

    let soft_unlimited = rlim.rlim_cur == RLIM_INFINITY_VAL;
    let hard_unlimited = rlim.rlim_max == RLIM_INFINITY_VAL;

    let soft = if soft_unlimited {
        None
    } else {
        Some(rlim.rlim_cur)
    };
    let hard = if hard_unlimited {
        None
    } else {
        Some(rlim.rlim_max)
    };

    Ok(ResourceLimitEntry {
        resource: kind,
        soft,
        hard,
        soft_unlimited,
        hard_unlimited,
        unit: kind.unit(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlimit_kind_str_values() {
        assert_eq!(ResourceLimitKind::Nofile.as_str(), "nofile");
        assert_eq!(ResourceLimitKind::Nproc.as_str(), "nproc");
        assert_eq!(ResourceLimitKind::Memlock.as_str(), "memlock");
        assert_eq!(ResourceLimitKind::AddressSpace.as_str(), "address_space");
        assert_eq!(ResourceLimitKind::Core.as_str(), "core");
        assert_eq!(ResourceLimitKind::FileSize.as_str(), "file_size");
        assert_eq!(ResourceLimitKind::Stack.as_str(), "stack");
    }

    #[test]
    fn rlimit_kind_units() {
        assert_eq!(ResourceLimitKind::Nofile.unit(), "count");
        assert_eq!(ResourceLimitKind::Nproc.unit(), "count");
        assert_eq!(ResourceLimitKind::Memlock.unit(), "bytes");
        assert_eq!(ResourceLimitKind::AddressSpace.unit(), "bytes");
        assert_eq!(ResourceLimitKind::Core.unit(), "bytes");
        assert_eq!(ResourceLimitKind::FileSize.unit(), "bytes");
        assert_eq!(ResourceLimitKind::Stack.unit(), "bytes");
    }

    #[test]
    fn all_limits_covers_seven_resources() {
        assert_eq!(ALL_LIMITS.len(), 7);
    }
}
