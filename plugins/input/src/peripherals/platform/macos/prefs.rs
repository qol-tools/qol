use std::ptr::NonNull;

use anyhow::{bail, Result};
use objc2_core_foundation::{CFBoolean, CFNumber, CFRetained, CFString, CFType};
use serde::{Deserialize, Serialize};

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFPreferencesAnyApplication: &'static CFString;
    static kCFPreferencesCurrentUser: &'static CFString;
    static kCFPreferencesAnyHost: &'static CFString;
    static kCFPreferencesCurrentHost: &'static CFString;
    fn CFPreferencesCopyValue(
        key: &CFString,
        application: &CFString,
        user: &CFString,
        host: &CFString,
    ) -> Option<NonNull<CFType>>;
    fn CFPreferencesSetValue(
        key: &CFString,
        value: Option<&CFType>,
        application: &CFString,
        user: &CFString,
        host: &CFString,
    );
    fn CFPreferencesSynchronize(application: &CFString, user: &CFString, host: &CFString) -> u8;
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
}

impl Value {
    pub(super) fn number(self) -> f64 {
        match self {
            Self::Bool(flag) => f64::from(u8::from(flag)),
            Self::Int(number) => number as f64,
            Self::Float(number) => number,
        }
    }

    pub(super) fn flag(self) -> bool {
        self.number() != 0.0
    }

    fn with_cf<T>(self, run: impl FnOnce(&CFType) -> T) -> T {
        match self {
            Self::Bool(flag) => run(CFBoolean::new(flag)),
            Self::Int(number) => run(&CFNumber::new_i64(number)),
            Self::Float(number) => run(&CFNumber::new_f64(number)),
        }
    }

    fn from_cf(value: &CFType) -> Option<Self> {
        if let Some(flag) = value.downcast_ref::<CFBoolean>() {
            return Some(Self::Bool(flag.as_bool()));
        }
        let number = value.downcast_ref::<CFNumber>()?;
        if number.is_float_type() {
            number.as_f64().map(Self::Float)
        } else {
            number.as_i64().map(Self::Int)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Domain {
    Global,
    GlobalThisMac,
    App(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pref {
    domain: Domain,
    pub(super) key: &'static str,
    pub(super) default: Value,
}

impl Pref {
    pub(super) const fn global(key: &'static str, default: Value) -> Self {
        Self {
            domain: Domain::Global,
            key,
            default,
        }
    }

    pub(super) const fn global_this_mac(key: &'static str, default: Value) -> Self {
        Self {
            domain: Domain::GlobalThisMac,
            key,
            default,
        }
    }

    pub(super) const fn app(app: &'static str, key: &'static str, default: Value) -> Self {
        Self {
            domain: Domain::App(app),
            key,
            default,
        }
    }

    pub(super) fn read(&self) -> Result<Value> {
        Ok(self.raw()?.unwrap_or(self.default))
    }

    pub(super) fn raw(&self) -> Result<Option<Value>> {
        let key = CFString::from_str(self.key);
        self.with_domain(|application, user, host| {
            let Some(value) = (unsafe { CFPreferencesCopyValue(&key, application, user, host) })
            else {
                return Ok(None);
            };
            let value = unsafe { CFRetained::from_raw(value) };
            match Value::from_cf(&value) {
                Some(value) => Ok(Some(value)),
                None => bail!("the macOS setting {} is not a number", self.key),
            }
        })
    }

    pub(super) fn write(&self, value: Option<Value>) -> Result<()> {
        let key = CFString::from_str(self.key);
        self.with_domain(|application, user, host| {
            let set = |value: Option<&CFType>| unsafe {
                CFPreferencesSetValue(&key, value, application, user, host)
            };
            match value {
                Some(value) => value.with_cf(|value| set(Some(value))),
                None => set(None),
            }
            if unsafe { CFPreferencesSynchronize(application, user, host) } == 0 {
                bail!("macOS did not save the setting {}", self.key);
            }
            Ok(())
        })
    }

    fn with_domain<T>(
        &self,
        run: impl FnOnce(&CFString, &CFString, &CFString) -> Result<T>,
    ) -> Result<T> {
        let user = unsafe { kCFPreferencesCurrentUser };
        match self.domain {
            Domain::Global => run(unsafe { kCFPreferencesAnyApplication }, user, unsafe {
                kCFPreferencesAnyHost
            }),
            Domain::GlobalThisMac => run(unsafe { kCFPreferencesAnyApplication }, user, unsafe {
                kCFPreferencesCurrentHost
            }),
            Domain::App(app) => run(&CFString::from_str(app), user, unsafe {
                kCFPreferencesAnyHost
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_survives_the_trip_through_core_foundation() {
        for value in [
            Value::Bool(true),
            Value::Bool(false),
            Value::Int(30),
            Value::Float(0.6875),
        ] {
            assert_eq!(value.with_cf(Value::from_cf), Some(value));
        }
        let text = CFString::from_str("fast");
        assert_eq!(Value::from_cf(&text), None);
    }

    #[test]
    fn a_value_reads_as_a_number_and_as_a_switch() {
        assert_eq!(Value::Bool(true).number(), 1.0);
        assert_eq!(Value::Int(5).number(), 5.0);
        assert!(Value::Int(1).flag());
        assert!(!Value::Float(0.0).flag());
    }
}
