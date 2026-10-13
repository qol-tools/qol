use std::collections::HashSet;
use std::sync::Mutex;

use crate::monitor::backends::i2c_ddc::I2cError;

#[derive(Default)]
pub(crate) struct DdcWrites {
    dropped: Mutex<HashSet<String>>,
}

impl DdcWrites {
    pub(crate) fn dropped(&self, connector: &str) -> bool {
        self.dropped.lock().unwrap().contains(connector)
    }

    pub(crate) fn refuse_dropped(&self, connector: &str) -> Result<(), I2cError> {
        if !self.dropped(connector) {
            return Ok(());
        }
        Err(I2cError::UnsupportedTransport {
            detail: format!(
                "DDC/CI writes were dropped on {connector}; set the display policy to gamma to \
                 keep brightness control"
            ),
        })
    }

    pub(crate) fn write_verified<E: From<I2cError>>(
        &self,
        connector: &str,
        value: u8,
        max: u32,
        attempts: usize,
        mut write_then_read: impl FnMut(usize, u32) -> Result<u32, E>,
    ) -> Result<(), E> {
        if max == 0 {
            return Err(I2cError::Protocol {
                detail: format!("{connector} reports a maximum brightness of 0"),
            }
            .into());
        }
        let target = raw_from_percent(value, max);
        for attempt in 0..attempts {
            if scale(write_then_read(attempt, target)?, max) == scale(target, max) {
                return Ok(());
            }
        }
        self.dropped.lock().unwrap().insert(connector.to_string());
        Ok(())
    }
}

pub(crate) fn percent_from_raw(current: u32, max: u32) -> Result<u8, I2cError> {
    if max == 0 {
        return Err(I2cError::Protocol {
            detail: "the display reports a maximum brightness of 0".into(),
        });
    }
    Ok(scale(current, max))
}

fn raw_from_percent(value: u8, max: u32) -> u32 {
    (u64::from(value.min(100)) * u64::from(max) / 100) as u32
}

fn scale(current: u32, max: u32) -> u8 {
    (u64::from(current) * 100 / u64::from(max)).min(100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_from_raw_scales_and_clamps() {
        let cases = [
            (0, 100, Some(0)),
            (50, 100, Some(50)),
            (100, 100, Some(100)),
            (127, 255, Some(49)),
            (255, 255, Some(100)),
            (250, 1000, Some(25)),
            (999, 1000, Some(99)),
            (120, 100, Some(100)),
            (1000, 500, Some(100)),
            (u32::MAX, u32::MAX, Some(100)),
            (10, 0, None),
        ];
        for (current, max, expected) in cases {
            assert_eq!(
                percent_from_raw(current, max).ok(),
                expected,
                "current {current} max {max}"
            );
        }
    }

    #[test]
    fn raw_from_percent_scales_into_the_monitor_range() {
        let cases = [
            (0, 100, 0),
            (50, 100, 50),
            (100, 100, 100),
            (50, 255, 127),
            (100, 255, 255),
            (35, 1000, 350),
            (150, 100, 100),
            (70, u32::MAX, 3_006_477_106),
        ];
        for (value, max, expected) in cases {
            assert_eq!(
                raw_from_percent(value, max),
                expected,
                "value {value} max {max}"
            );
        }
    }

    #[test]
    fn a_write_verifies_by_percent_and_marks_a_connector_that_drops_it() {
        struct Case {
            name: &'static str,
            value: u8,
            max: u32,
            stored: fn(u32) -> u32,
            writes: Vec<u32>,
            dropped: bool,
        }
        let cases = [
            Case {
                name: "the monitor keeps the value",
                value: 40,
                max: 100,
                stored: |target| target,
                writes: vec![40],
                dropped: false,
            },
            Case {
                name: "a coarse range keeps the floor of the value",
                value: 50,
                max: 255,
                stored: |target| target,
                writes: vec![127],
                dropped: false,
            },
            Case {
                name: "the monitor rounds within the same percent",
                value: 50,
                max: 1000,
                stored: |target| target + 4,
                writes: vec![500],
                dropped: false,
            },
            Case {
                name: "the monitor ignores the write",
                value: 50,
                max: 1000,
                stored: |_| 200,
                writes: vec![500, 500],
                dropped: true,
            },
            Case {
                name: "the monitor lands on another percent",
                value: 40,
                max: 100,
                stored: |target| target + 5,
                writes: vec![40, 40],
                dropped: true,
            },
        ];
        for case in cases {
            let writes_tracker = DdcWrites::default();
            let mut writes = Vec::new();
            writes_tracker
                .write_verified::<I2cError>("DP-1", case.value, case.max, 2, |attempt, target| {
                    assert_eq!(attempt, writes.len(), "{}", case.name);
                    writes.push(target);
                    Ok((case.stored)(target))
                })
                .unwrap();
            assert_eq!(writes, case.writes, "{}", case.name);
            assert_eq!(
                writes_tracker.dropped("DP-1"),
                case.dropped,
                "{}",
                case.name
            );
            assert!(!writes_tracker.dropped("DP-2"), "{}", case.name);
            let refusal = writes_tracker.refuse_dropped("DP-1");
            assert_eq!(refusal.is_err(), case.dropped, "{}", case.name);
            if let Err(error) = refusal {
                assert!(
                    error.to_string().contains("writes were dropped on DP-1"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn a_zero_maximum_refuses_before_any_write() {
        let mut writes = 0;
        let error = DdcWrites::default()
            .write_verified::<I2cError>("DP-1", 50, 0, 2, |_, target| {
                writes += 1;
                Ok(target)
            })
            .unwrap_err();
        assert!(matches!(error, I2cError::Protocol { .. }), "{error}");
        assert_eq!(writes, 0);
    }
}
