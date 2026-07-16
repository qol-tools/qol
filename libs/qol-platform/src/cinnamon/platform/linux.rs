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
        crate::cinnamon::eval_result(success, result)
    }
}
