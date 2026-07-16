pub(crate) struct Session;

impl Session {
    pub(crate) fn connect() -> Result<Self, String> {
        Err("Cinnamon Eval is only available on Linux".into())
    }

    pub(crate) fn eval(&self, _script: &str) -> Result<String, String> {
        Err("Cinnamon Eval is only available on Linux".into())
    }
}
