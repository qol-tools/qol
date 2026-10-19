use super::{failure, shared, Shared, Watch};

pub struct ObservedBody {
    response: reqwest::Response,
    watch: Watch,
}

pub fn observe_body(response: reqwest::Response) -> ObservedBody {
    ObservedBody::on(shared(), response)
}

impl ObservedBody {
    pub(super) fn on(shared: &'static Shared, response: reqwest::Response) -> Self {
        Self {
            response,
            watch: Watch::begin(shared),
        }
    }

    pub fn content_length(&self) -> Option<u64> {
        self.response.content_length()
    }

    pub async fn chunk(&mut self) -> reqwest::Result<Option<impl AsRef<[u8]>>> {
        let chunk = self.response.chunk().await;
        match &chunk {
            Ok(Some(_)) => self.watch.progress(),
            Ok(None) => self.watch.end(None),
            Err(error) => self.watch.end(failure(error)),
        }
        chunk
    }

    pub async fn bytes(mut self) -> reqwest::Result<Vec<u8>> {
        let mut body = Vec::new();
        while let Some(chunk) = self.chunk().await? {
            body.extend_from_slice(chunk.as_ref());
        }
        Ok(body)
    }
}
