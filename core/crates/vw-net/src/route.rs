//! Read-only route diagnosis and reviewable command text. Nothing here executes
//! PowerShell or changes adapter metrics, privileges, services or firewall rules.
use crate::pairing::{PairingError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
}
impl AddressFamily {
    fn powershell(self) -> &'static str {
        match self {
            Self::Ipv4 => "IPv4",
            Self::Ipv6 => "IPv6",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultRoute {
    pub interface_index: u32,
    pub family: AddressFamily,
    pub route_metric: u32,
    pub interface_metric: u32,
    pub automatic_metric: bool,
}
impl DefaultRoute {
    fn effective(self) -> u64 {
        u64::from(self.route_metric) + u64::from(self.interface_metric)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteRisk {
    Preferred,
    Tied,
    OnlyDefault,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricSuggestion {
    /// Requires explicit user approval and an administrator PowerShell window.
    pub fix_command: String,
    pub revert_command: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteFinding {
    pub family: AddressFamily,
    pub interface_index: u32,
    pub risk: RouteRisk,
    pub suggestion: Option<MetricSuggestion>,
}
pub trait RouteProvider {
    fn default_routes(&self) -> Result<Vec<DefaultRoute>>;
}

/// `tether_interface` comes from the adapter selected for the connection. The
/// function never guesses a tether from a private device name or network address.
pub fn detect_tether_default(
    routes: &[DefaultRoute],
    tether_interface: u32,
) -> Result<Vec<RouteFinding>> {
    if tether_interface == 0
        || routes.len() > 4096
        || routes
            .iter()
            .any(|route| route.interface_index == 0 || route.interface_metric > 9999)
    {
        return Err(PairingError::Invalid("route snapshot"));
    }
    let mut findings = Vec::new();
    for family in [AddressFamily::Ipv4, AddressFamily::Ipv6] {
        let Some(tether) = routes
            .iter()
            .filter(|r| r.family == family && r.interface_index == tether_interface)
            .min_by_key(|r| r.effective())
        else {
            continue;
        };
        let alternative = routes
            .iter()
            .filter(|r| r.family == family && r.interface_index != tether_interface)
            .map(|r| r.effective())
            .min();
        if alternative.is_some_and(|metric| tether.effective() > metric) {
            continue;
        }
        let risk = match alternative {
            None => RouteRisk::OnlyDefault,
            Some(metric) if tether.effective() == metric => RouteRisk::Tied,
            Some(_) => RouteRisk::Preferred,
        };
        let suggestion = alternative.and_then(|metric| {
            let desired_total = metric.checked_add(50)?;
            let desired = desired_total
                .saturating_sub(u64::from(tether.route_metric))
                .max(500);
            if desired > 9999 {
                return None;
            }
            let prefix = format!(
                "Set-NetIPInterface -InterfaceIndex {} -AddressFamily {}",
                tether.interface_index,
                family.powershell()
            );
            let revert_command = if tether.automatic_metric {
                format!("{prefix} -AutomaticMetric Enabled")
            } else {
                format!(
                    "{prefix} -AutomaticMetric Disabled -InterfaceMetric {}",
                    tether.interface_metric
                )
            };
            Some(MetricSuggestion {
                fix_command: format!(
                    "{prefix} -AutomaticMetric Disabled -InterfaceMetric {desired}"
                ),
                revert_command,
            })
        });
        findings.push(RouteFinding {
            family,
            interface_index: tether_interface,
            risk,
            suggestion,
        });
    }
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn route(
        index: u32,
        family: AddressFamily,
        route_metric: u32,
        interface_metric: u32,
        automatic_metric: bool,
    ) -> DefaultRoute {
        DefaultRoute {
            interface_index: index,
            family,
            route_metric,
            interface_metric,
            automatic_metric,
        }
    }
    #[test]
    fn considers_effective_metrics_and_retains_exact_revert() -> Result<()> {
        let routes = [
            route(7, AddressFamily::Ipv4, 0, 5, true),
            route(3, AddressFamily::Ipv4, 100, 20, true),
            route(7, AddressFamily::Ipv6, 0, 35, false),
            route(3, AddressFamily::Ipv6, 5, 30, true),
        ];
        let findings = detect_tether_default(&routes, 7)?;
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].risk, RouteRisk::Preferred);
        assert_eq!(findings[1].risk, RouteRisk::Tied);
        assert_eq!(
            findings[0]
                .suggestion
                .as_ref()
                .map(|s| s.fix_command.as_str()),
            Some(
                "Set-NetIPInterface -InterfaceIndex 7 -AddressFamily IPv4 -AutomaticMetric Disabled -InterfaceMetric 500"
            )
        );
        assert!(
            findings[0]
                .suggestion
                .as_ref()
                .is_some_and(|s| s.revert_command.ends_with("-AutomaticMetric Enabled"))
        );
        assert!(
            findings[1]
                .suggestion
                .as_ref()
                .is_some_and(|s| s.revert_command.ends_with("-InterfaceMetric 35"))
        );
        Ok(())
    }
    #[test]
    fn no_alternative_never_promises_a_metric_fix_and_bounds_are_checked() -> Result<()> {
        let routes = [route(7, AddressFamily::Ipv4, 0, 5, true)];
        let findings = detect_tether_default(&routes, 7)?;
        assert_eq!(findings[0].risk, RouteRisk::OnlyDefault);
        assert!(findings[0].suggestion.is_none());
        assert!(detect_tether_default(&routes, 0).is_err());
        let expensive = [routes[0], route(3, AddressFamily::Ipv4, u32::MAX, 20, true)];
        assert!(
            detect_tether_default(&expensive, 7)?[0]
                .suggestion
                .is_none()
        );
        let better = [routes[0], route(3, AddressFamily::Ipv4, 0, 1, true)];
        assert!(detect_tether_default(&better, 7)?.is_empty());
        Ok(())
    }
}
