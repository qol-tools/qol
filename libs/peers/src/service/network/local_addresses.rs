use std::collections::BTreeSet;
use std::net::IpAddr;

use crate::admin::{EnrollmentFailure, Error};

pub fn current() -> Result<Vec<IpAddr>, Error> {
    select_with(|| {
        if_addrs::get_if_addrs().map(|interfaces| {
            interfaces
                .into_iter()
                .map(|interface| interface.ip())
                .collect()
        })
    })
}

pub fn select_with(
    read: impl FnOnce() -> std::io::Result<Vec<IpAddr>>,
) -> Result<Vec<IpAddr>, Error> {
    let addresses: Vec<_> = read()
        .map_err(|_| unavailable())?
        .into_iter()
        .filter(|address| super::enrollment::validate_address(*address, false).is_ok())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(8)
        .collect();
    if addresses.is_empty() {
        return Err(unavailable());
    }
    Ok(addresses)
}

fn unavailable() -> Error {
    Error::Enrollment {
        error: EnrollmentFailure::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::select_with;
    use std::net::IpAddr;

    #[test]
    fn filters_sorts_deduplicates_and_bounds_current_ipv4() {
        let mut input: Vec<IpAddr> = [
            "::1",
            "127.0.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
        ]
        .into_iter()
        .map(|value| value.parse().unwrap())
        .collect();
        input.extend((1..=12).rev().map(|last| IpAddr::from([192, 168, 1, last])));
        input.push(IpAddr::from([192, 168, 1, 1]));
        let expected: Vec<_> = (1..=8)
            .map(|last| IpAddr::from([192, 168, 1, last]))
            .collect();
        assert_eq!(select_with(|| Ok(input)).unwrap(), expected);
    }

    #[test]
    fn no_usable_address_or_interface_failure_refuses() {
        assert!(select_with(|| Ok(vec![IpAddr::from([127, 0, 0, 1])])).is_err());
        assert!(select_with(|| Err(std::io::Error::other("fixture"))).is_err());
    }
}
