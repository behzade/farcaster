use std::{io::Read as _, time::Duration};

use futures::future::BoxFuture;
use gpui::http_client::{self, AsyncBody, HttpClient, Request, Response, Url, http::HeaderValue};

pub(super) struct ImageHttpClient;

impl HttpClient for ImageHttpClient {
    fn user_agent(&self) -> Option<&HeaderValue> {
        None
    }
    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, http_client::Result<Response<AsyncBody>>> {
        let (send, receive) = async_channel::bounded(1);
        let worker = std::thread::Builder::new()
            .name("image-http".into())
            .spawn(move || {
                let result = fetch(request);
                let _ = send.send_blocking(result);
            });
        Box::pin(async move {
            worker?;
            receive.recv().await?
        })
    }
}

fn fetch(request: Request<AsyncBody>) -> http_client::Result<Response<AsyncBody>> {
    if request.method() != http_client::Method::GET {
        return Err(http_client::anyhow!(
            "Image loader supports GET requests only"
        ));
    }
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .max_redirects(5)
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent.get(request.uri().to_string()).call()?;
    let status = response.status();
    let headers = response.headers().clone();
    let mut bytes = Vec::new();
    const LIMIT: u64 = crate::app::attachment::IMAGE_PREVIEW_BYTES;
    response
        .body_mut()
        .as_reader()
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(http_client::anyhow!("Image preview exceeds 32 MiB"));
    }
    let mut result = Response::builder().status(status);
    *result.headers_mut().expect("valid response builder") = headers;
    Ok(result.body(bytes.into())?)
}

#[cfg(test)]
#[path = "image_http_tests.rs"]
mod tests;
