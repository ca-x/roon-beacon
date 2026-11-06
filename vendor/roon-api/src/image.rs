use crate::error::ApiError;

/// Options for image retrieval.
#[derive(Debug, Clone, Default)]
pub struct ImageOptions {
    /// Scale mode: "fit", "fill", or "stretch".
    pub scale: Option<String>,
    /// Desired width in pixels.
    pub width: Option<u32>,
    /// Desired height in pixels.
    pub height: Option<u32>,
    /// Output format: "image/jpeg" or "image/png".
    pub format: Option<String>,
}

/// Image service for retrieving album art and other images from Roon Core.
#[derive(Debug, Clone)]
pub struct ImageService {
    base_url: String,
}

impl ImageService {
    pub(crate) fn new(host: &str, http_port: u16) -> Self {
        Self {
            base_url: format!("http://{}:{}", host, http_port),
        }
    }

    /// Get an image by its key. Returns the raw image bytes.
    pub async fn get_image(
        &self,
        image_key: &str,
        opts: &ImageOptions,
    ) -> Result<Vec<u8>, ApiError> {
        let mut url = format!("{}/api/image/{}", self.base_url, image_key);

        let mut params = Vec::new();
        if let Some(ref scale) = opts.scale {
            params.push(format!("scale={}", scale));
        }
        if let Some(width) = opts.width {
            params.push(format!("width={}", width));
        }
        if let Some(height) = opts.height {
            params.push(format!("height={}", height));
        }
        if let Some(ref format) = opts.format {
            params.push(format!("format={}", format));
        }
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }

        let response = reqwest::get(&url)
            .await
            .map_err(|e| ApiError::Io(format!("image fetch failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(ApiError::Io(format!(
                "image fetch returned status {}",
                response.status()
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| ApiError::Io(format!("image read failed: {}", e)))?;

        Ok(bytes.to_vec())
    }

    /// Build an image URL without fetching (for embedding in UIs).
    pub fn image_url(&self, image_key: &str, opts: &ImageOptions) -> String {
        let mut url = format!("{}/api/image/{}", self.base_url, image_key);
        let mut params = Vec::new();
        if let Some(ref scale) = opts.scale {
            params.push(format!("scale={}", scale));
        }
        if let Some(width) = opts.width {
            params.push(format!("width={}", width));
        }
        if let Some(height) = opts.height {
            params.push(format!("height={}", height));
        }
        if let Some(ref format) = opts.format {
            params.push(format!("format={}", format));
        }
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }
        url
    }
}
