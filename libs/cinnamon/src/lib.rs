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
