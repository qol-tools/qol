use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

type HttpResult<T> = Result<T, Box<Response>>;

pub(in super::super) async fn serve_icon(Path(bundle_id): Path<String>) -> impl IntoResponse {
    serve_icon_inner(bundle_id)
        .await
        .unwrap_or_else(|response| *response)
}

async fn serve_icon_inner(bundle_id: String) -> HttpResult<Response> {
    let (mut rgba, width, height) = load_icon_rgba(bundle_id).await?;
    swap_bgra_channels(&mut rgba);
    let png = encode_rgba_to_png(&rgba, width, height)?;
    Ok(icon_png_response(png))
}

async fn load_icon_rgba(bundle_id: String) -> HttpResult<(Vec<u8>, u32, u32)> {
    let icon = tokio::task::spawn_blocking(move || {
        qol_app_icon::icon_for_bundle_id(&bundle_id, 32)
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(|| Box::new(icon_not_found_response()))?;
    Ok((icon.data, icon.width as u32, icon.height as u32))
}

fn swap_bgra_channels(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

fn encode_rgba_to_png(rgba: &[u8], width: u32, height: u32) -> HttpResult<Vec<u8>> {
    let mut png_buf = Vec::new();
    let mut writer = png_writer(&mut png_buf, width, height)?;
    writer
        .write_image_data(rgba)
        .map_err(|_| Box::new(png_encode_failed_response()))?;
    drop(writer);
    Ok(png_buf)
}

fn png_writer(
    buffer: &mut Vec<u8>,
    width: u32,
    height: u32,
) -> HttpResult<png::Writer<&mut Vec<u8>>> {
    let mut encoder = png::Encoder::new(buffer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .map_err(|_| Box::new(png_encode_failed_response()))
}

fn icon_png_response(data: Vec<u8>) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, "image/png")], data).into_response()
}

fn icon_not_found_response() -> Response {
    (StatusCode::NOT_FOUND, "Icon not found").into_response()
}

fn png_encode_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "PNG encode failed").into_response()
}
