mod platform;

pub struct Session {
    inner: platform::Session,
}

impl Session {
    pub fn connect() -> Result<Self, String> {
        platform::Session::connect().map(|inner| Self { inner })
    }

    pub fn eval(&self, script: &str) -> Result<String, String> {
        self.inner.eval(script)
    }
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn eval_result(success: bool, result: String) -> Result<String, String> {
    if success && !result.contains("ERROR:") {
        return Ok(result);
    }
    Err(format!("Cinnamon Eval failed: {result}"))
}

#[cfg(test)]
mod tests {
    use super::eval_result;

    #[test]
    fn accepts_successful_eval() {
        assert_eq!(eval_result(true, "moved".into()).unwrap(), "moved");
    }

    #[test]
    fn rejects_dbus_failure_and_script_error() {
        assert!(eval_result(false, "boom".into()).is_err());
        assert!(eval_result(true, "ERROR: No focused window".into()).is_err());
    }
}
