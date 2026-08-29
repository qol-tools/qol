pub(crate) struct Session {
    connection: zbus::blocking::Connection,
}

impl Session {
    pub(crate) fn connect() -> Result<Self, String> {
        zbus::blocking::Connection::session()
            .map(|connection| Self { connection })
            .map_err(|error| format!("Failed to connect to session D-Bus: {error}"))
    }

    pub(crate) fn eval(&self, script: &str) -> Result<String, String> {
        let reply = self
            .connection
            .call_method(
                Some("org.Cinnamon"),
                "/org/Cinnamon",
                Some("org.Cinnamon"),
                "Eval",
                &(script,),
            )
            .map_err(|error| format!("Cinnamon Eval call failed: {error}"))?;
        let (success, result): (bool, String) = reply
            .body()
            .deserialize()
            .map_err(|error| format!("Cinnamon Eval response was invalid: {error}"))?;
        eval_result(success, result)
    }
}

fn eval_result(success: bool, result: String) -> Result<String, String> {
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
